//! Exclusive allocation of objects to table rows, with per-row count and
//! summed-area requirements.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue, Scope};

use crate::counts::{Population, relation_text, tally};
use crate::pairs::severity;
use crate::plan_area::{Sum, footprint, shown};
use crate::selection::select_objects;
use crate::support::table::{self, Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, invalid, resolve, sources,
    traversal_parameters,
};

/// The key columns, each testing the property the same-named parameter names.
pub(crate) const KEYS: [&str; 3] = ["key_1", "key_2", "key_3"];

/// A row's key cells: each compiled pattern with its text as written.
pub(crate) type KeyCells<'a> = [Option<(TextPattern, &'a str)>; 3];

/// The key properties the `key_1` to `key_3` parameters name.
pub(crate) fn key_properties<'a>(
    parameters: &Parameters<'a>,
) -> Result<[Option<PropertyRef<'a>>; 3], Unavailable> {
    Ok([
        parameters.property(KEYS[0])?,
        parameters.property(KEYS[1])?,
        parameters.property(KEYS[2])?,
    ])
}

/// The key cells of row `number`; a cell whose property is not declared is
/// an invalid declaration.
pub(crate) fn key_cells<'a>(
    row: table::Row<'a>,
    number: usize,
    properties: &[Option<PropertyRef<'_>>; 3],
    case_sensitive: bool,
) -> Result<KeyCells<'a>, Unavailable> {
    let mut keys = [None, None, None];
    for (slot, (column, property)) in keys.iter_mut().zip(KEYS.iter().zip(properties)) {
        let Some(text) = row.text(column)? else {
            continue;
        };
        if property.is_none() {
            return Err(invalid(format!(
                "row {number} fills `{column}`, but no `{column}` property is declared"
            )));
        }
        let pattern = row
            .pattern(column, case_sensitive)?
            .expect("a filled cell compiles");
        *slot = Some((pattern, text));
    }
    Ok(keys)
}

/// How findings name a row: by its label, or by its number and key cells.
pub(crate) fn row_name(
    number: usize,
    label: Option<&str>,
    cells: &KeyCells<'_>,
    properties: &[Option<PropertyRef<'_>>; 3],
) -> String {
    if let Some(label) = label {
        return format!("row {number} `{label}`");
    }
    let keys: Vec<String> = cells
        .iter()
        .zip(properties)
        .filter_map(|(key, property)| {
            let ((_, pattern), property) = (key.as_ref()?, property.as_ref()?);
            Some(format!("{property} like `{pattern}`"))
        })
        .collect();
    if keys.is_empty() {
        format!("row {number} (any object)")
    } else {
        format!("row {number} ({})", keys.join(", "))
    }
}

/// An object's key values, read only for the keys in `used`, with the
/// evidence for them.
pub(crate) fn read_keys(
    context: &RuleContext<'_>,
    object: &Object,
    properties: &[Option<PropertyRef<'_>>; 3],
    used: [bool; 3],
) -> ([Option<Key>; 3], Vec<Evidence>) {
    let mut evidence = Vec::new();
    let mut keys = [None, None, None];
    for ((slot, property), used) in keys.iter_mut().zip(properties).zip(used) {
        let Some(property) = property.filter(|_| used) else {
            continue;
        };
        *slot = Some(match resolve(context, object, property) {
            Ok(resolved) => {
                evidence.extend(resolved.evidence());
                match resolved.value() {
                    None | Some(PropertyValue::Null) => Key::Absent,
                    Some(PropertyValue::String(text)) => Key::Text(text.clone()),
                    Some(other) => Key::Unknown(
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{property} is {}, not text", display(Some(other))),
                    ),
                }
            }
            Err((reason, message)) => Key::Unknown(reason, message),
        });
    }
    (keys, evidence)
}

/// Whether a row's key cells match an object's key values: an absent value
/// matches no pattern, an unknown one decides nothing.
pub(crate) fn test_keys(cells: &KeyCells<'_>, keys: &[Option<Key>; 3]) -> RowTest {
    cells
        .iter()
        .zip(keys)
        .fold(RowTest::Match(0), |test, (pattern, key)| {
            let Some((pattern, _)) = pattern else {
                return test;
            };
            test.and(match key {
                Some(Key::Text(text)) => pattern.test(text),
                Some(Key::Absent) => RowTest::NoMatch,
                Some(Key::Unknown(..)) | None => RowTest::Undecided,
            })
        })
}

/// Why an object's keys leave a row undecided: its first unknown key.
pub(crate) fn unknown_key(keys: &[Option<Key>; 3]) -> Unavailable {
    keys.iter()
        .flatten()
        .find_map(|key| match key {
            Key::Unknown(reason, message) => Some((reason.clone(), message.clone())),
            _ => None,
        })
        .unwrap_or((
            NotEvaluatedReason::IncompleteEvidence,
            "a key cannot be read".into(),
        ))
}

/// Key values as a reviewer reads them.
pub(crate) fn describe_keys(
    properties: &[Option<PropertyRef<'_>>; 3],
    keys: &[Option<Key>; 3],
) -> String {
    let shown: Vec<String> = properties
        .iter()
        .zip(keys)
        .filter_map(|(property, key)| {
            let value = match key.as_ref()? {
                Key::Text(text) => format!("`{text}`"),
                Key::Absent => "absent".into(),
                Key::Unknown(..) => "unknown".into(),
            };
            Some(format!("{} is {value}", (*property)?))
        })
        .collect();
    if shown.is_empty() {
        "the table has no row".into()
    } else {
        shown.join(", ")
    }
}

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("key_1", ColumnKind::TextPattern),
    TableColumn::optional("key_2", ColumnKind::TextPattern),
    TableColumn::optional("key_3", ColumnKind::TextPattern),
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::optional("count", ColumnKind::Integer),
    TableColumn::optional("area", ColumnKind::Number),
    TableColumn::optional("area_tolerance", ColumnKind::Number),
];

/// Assigns each selected object to exactly one row of a table, then checks
/// every row's assigned objects: how many, and their summed area.
///
/// Rows are keyed by text patterns: the parameters `key_1` to `key_3` name
/// the properties (a space type, a name, a number), and the row's cells of
/// the same names hold whole-value wildcard patterns over them. A row
/// matches an object when every key cell it fills matches; a row with no key
/// cell matches everything. `mode` `first` (the default) assigns the first
/// matching row in declared order, `most_specific` the matching row with the
/// most literal pattern characters, so `Office 1*` wins over `Office*`.
///
/// Rows are then judged per group: per anchor that `anchor_selector` picks,
/// counting the objects it reaches through the traversal parameters (or
/// every object of its source without one), such as per storey; otherwise
/// per source, including a source that holds no objects, or across the whole
/// project with `across_sources`. In each
/// group, a row's `count` must equal the number of objects assigned to it,
/// and their summed plan area must lie within `area` ± `area_tolerance`
/// square metres. Areas are measured footprints, or an area quantity stated
/// by `area_property`.
///
/// An object no row matches is an extra and a finding of its own; a row that
/// matched nothing in a group is a finding unless its `count` is zero. An
/// object whose key cannot be read, or whose most specific rows tie, is not
/// evaluated, and so is every row it might belong to unless an excess over
/// the row's count or area already stands. Measured areas are intervals: one
/// straddling a bound is not evaluated.
pub struct TableAllocation;

impl RuleCapability for TableAllocation {
    fn id(&self) -> &'static str {
        "axioval:capability.table-allocation"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("rows", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::optional("mode", ParameterType::String),
            ParameterDescriptor::optional("key_1", ParameterType::PropertyReference),
            ParameterDescriptor::optional("key_2", ParameterType::PropertyReference),
            ParameterDescriptor::optional("key_3", ParameterType::PropertyReference),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("area_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("anchor_selector", ParameterType::Selector),
            ParameterDescriptor::optional("across_sources", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declaration = match Declaration::parse(rule) {
            Ok(declaration) => declaration,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("table-allocation: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let members = Population {
            matched: selected.iter().map(|object| object.id.clone()).collect(),
            undecided: evaluation
                .not_evaluated_outcomes()
                .iter()
                .filter_map(|outcome| outcome.object_id().cloned())
                .collect(),
        };

        // Each object is assigned once, whatever groups it is counted in.
        let mut assignments = BTreeMap::new();
        for object in &selected {
            let (assignment, evidence) = declaration.assign(context, object);
            match &assignment {
                Assignment::Extra(keys) => evaluation.push_finding(
                    Finding::new(
                        rule.id.clone(),
                        Scope::Object(object.id.clone()),
                        severity(rule),
                        format!("no row matches ({keys})"),
                    )
                    .with_evidence(evidence.clone()),
                ),
                Assignment::Open(_, reason, message) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    reason.clone(),
                    message.clone(),
                ),
                Assignment::Row(_) => {}
            }
            assignments.insert(object.id.clone(), (assignment, evidence));
        }

        let groups = match declaration.groups(context, &members, &mut evaluation) {
            Ok(groups) => groups,
            Err((reason, message)) => {
                evaluation.push_not_evaluated(reason, format!("table-allocation: {message}"));
                return evaluation;
            }
        };
        for group in groups {
            declaration.judge(context, rule, &group, &assignments, &mut evaluation);
        }
        evaluation
    }
}

/// One row as declared, with its keys compiled.
struct Row<'a> {
    /// One-based, as a reviewer counts.
    number: usize,
    label: Option<&'a str>,
    keys: KeyCells<'a>,
    count: Option<i64>,
    /// Required area and tolerance, square metres.
    area: Option<(f64, f64)>,
}

impl Row<'_> {
    fn name(&self, properties: &[Option<PropertyRef<'_>>; 3]) -> String {
        row_name(self.number, self.label, &self.keys, properties)
    }
}

/// An object's key value.
pub(crate) enum Key {
    Text(String),
    /// Exactly absent, or null: no pattern matches it.
    Absent,
    Unknown(NotEvaluatedReason, String),
}

/// Where an object belongs.
enum Assignment {
    Row(usize),
    /// No row matches; the key values as a reviewer reads them.
    Extra(String),
    /// The rows it might belong to (zero-based), and why it is undecided.
    Open(Vec<usize>, NotEvaluatedReason, String),
}

/// The objects judged together, and the scope their findings go against.
struct Group {
    scope: Scope,
    members: Vec<ObjectId>,
    /// Members whose selection is undecided: they may belong to any row.
    undecided: usize,
    evidence: Vec<Evidence>,
}

struct Declaration<'a> {
    rows: Vec<Row<'a>>,
    selection: RowSelection,
    properties: [Option<PropertyRef<'a>>; 3],
    area_property: Option<PropertyRef<'a>>,
    anchors: Option<&'a axioval_ir::contract::Selector>,
    traversal: Option<Traversal<'a>>,
    across_sources: bool,
}

impl<'a> Declaration<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let selection = match parameters.string("mode")? {
            None | Some("first") => RowSelection::First,
            Some("most_specific") => RowSelection::MostSpecific,
            Some(other) => {
                return Err(invalid(format!(
                    "mode `{other}` is unsupported; use `first` or `most_specific`"
                )));
            }
        };
        let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
        let properties = key_properties(&parameters)?;
        let anchors = parameters.selector("anchor_selector")?;
        let traversal = parameters.traversal()?;
        let across_sources = parameters.boolean("across_sources")?.unwrap_or(false);
        if anchors.is_none() && traversal.is_some() {
            return Err(invalid(
                "a relationship reaches members only from `anchor_selector`",
            ));
        }
        if anchors.is_some() && across_sources {
            return Err(invalid(
                "`across_sources` groups without anchors; declare one or the other",
            ));
        }
        let mut rows = Vec::new();
        for (index, row) in parameters
            .table("rows")?
            .ok_or_else(|| invalid("parameter `rows` is required"))?
            .into_iter()
            .enumerate()
        {
            let number = index + 1;
            let keys = key_cells(row, number, &properties, case_sensitive)?;
            let count = row.integer("count")?;
            if count.is_some_and(|count| count < 0) {
                return Err(invalid(format!("row {number} has a negative count")));
            }
            let area = match (row.number("area")?, row.number("area_tolerance")?) {
                (None, None) => None,
                (None, Some(_)) => {
                    return Err(invalid(format!(
                        "row {number} has an area tolerance but no area"
                    )));
                }
                (Some(area), tolerance) => {
                    let tolerance = tolerance.unwrap_or(0.0);
                    if area < 0.0 || tolerance < 0.0 {
                        return Err(invalid(format!(
                            "row {number} has a negative area or tolerance"
                        )));
                    }
                    Some((area, tolerance))
                }
            };
            rows.push(Row {
                number,
                label: row.text("label")?,
                keys,
                count,
                area,
            });
        }
        Ok(Self {
            rows,
            selection,
            properties,
            area_property: parameters.property("area_property")?,
            anchors,
            traversal,
            across_sources,
        })
    }

    /// The row `object` belongs to, with the evidence for its key values.
    fn assign(&self, context: &RuleContext<'_>, object: &Object) -> (Assignment, Vec<Evidence>) {
        // Only keys some row tests are read.
        let used = [0, 1, 2].map(|index| self.rows.iter().any(|row| row.keys[index].is_some()));
        let (keys, evidence) = read_keys(context, object, &self.properties, used);
        let test = |row: &Row<'_>| test_keys(&row.keys, &keys);
        let assignment = match match_rows(&self.rows, self.selection, test) {
            Matched::Rows(rows) => match rows.first() {
                Some((index, _)) => Assignment::Row(*index),
                None => Assignment::Extra(describe_keys(&self.properties, &keys)),
            },
            Matched::Ambiguous(tied) => {
                let names: Vec<String> = tied
                    .iter()
                    .map(|index| self.rows[*index].name(&self.properties))
                    .collect();
                Assignment::Open(
                    tied,
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("{} match equally specifically", names.join(" and ")),
                )
            }
            Matched::Undecided => {
                let candidates = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| test(row) != RowTest::NoMatch)
                    .map(|(index, _)| index)
                    .collect();
                let (reason, message) = unknown_key(&keys);
                Assignment::Open(
                    candidates,
                    reason,
                    format!("the row cannot be decided: {message}"),
                )
            }
        };
        (assignment, evidence)
    }

    /// The groups rows are judged in. Members no anchor reaches are not
    /// evaluated here.
    fn groups(
        &self,
        context: &RuleContext<'_>,
        members: &Population,
        evaluation: &mut CapabilityEvaluation,
    ) -> Result<Vec<Group>, Unavailable> {
        let Some(anchors) = self.anchors else {
            let mut groups: BTreeMap<Scope, Group> = BTreeMap::new();
            // Every source is a group even when it holds no objects, so a
            // row an empty source cannot meet is reported, not skipped.
            if !self.across_sources {
                for source in sources(context) {
                    let scope = Scope::Source(source);
                    groups.insert(
                        scope.clone(),
                        Group {
                            scope,
                            members: Vec::new(),
                            undecided: 0,
                            evidence: Vec::new(),
                        },
                    );
                }
            }
            for object in context.project.objects() {
                let scope = if self.across_sources {
                    Scope::Project
                } else {
                    Scope::Source(object.id.source.clone())
                };
                let group = groups.entry(scope.clone()).or_insert_with(|| Group {
                    scope,
                    members: Vec::new(),
                    undecided: 0,
                    evidence: Vec::new(),
                });
                if members.matched.contains(&object.id) {
                    group.members.push(object.id.clone());
                } else if members.undecided.contains(&object.id) {
                    group.undecided += 1;
                }
            }
            if groups.is_empty() {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    "the project has no source to allocate in".into(),
                ));
            }
            return Ok(groups.into_values().collect());
        };
        let (selected, outcomes) = select_objects(context, anchors);
        for outcome in outcomes.not_evaluated_outcomes() {
            if let Some(object) = outcome.object_id() {
                evaluation.push_object_not_evaluated(
                    object.clone(),
                    outcome.reason().clone(),
                    format!("whether it is an anchor: {}", outcome.message()),
                );
            }
        }
        let mut reached = BTreeSet::new();
        let mut groups = Vec::new();
        for anchor in selected {
            match tally(context, self.traversal.as_ref(), anchor, members) {
                Ok(found) => {
                    reached.extend(found.decided.iter().cloned());
                    groups.push(Group {
                        scope: Scope::Object(anchor.id.clone()),
                        members: found.decided,
                        undecided: found.undecided,
                        evidence: found.evidence,
                    });
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                }
            }
        }
        let via = relation_text(self.traversal.as_ref());
        for member in &members.matched {
            if !reached.contains(member) {
                evaluation.push_object_not_evaluated(
                    member.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("no anchor reaches it {via}"),
                );
            }
        }
        Ok(groups)
    }

    /// Judges every row within one group.
    fn judge(
        &self,
        context: &RuleContext<'_>,
        rule: &CompiledRule,
        group: &Group,
        assignments: &BTreeMap<ObjectId, (Assignment, Vec<Evidence>)>,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let place = match &group.scope {
            Scope::Source(source) => format!(" in source `{source}`"),
            Scope::Project => " in the project".to_owned(),
            Scope::Object(_) => String::new(),
        };
        for (index, row) in self.rows.iter().enumerate() {
            let mut assigned = Vec::new();
            let mut evidence = group.evidence.clone();
            let mut open = group.undecided;
            for member in &group.members {
                match assignments.get(member) {
                    Some((Assignment::Row(row), cited)) if *row == index => {
                        assigned.push(member.clone());
                        evidence.extend(cited.iter().cloned());
                    }
                    Some((Assignment::Open(rows, ..), _)) if rows.contains(&index) => open += 1,
                    _ => {}
                }
            }
            let name = row.name(&self.properties);
            let mut report = Report {
                rule,
                scope: &group.scope,
                evaluation: &mut *evaluation,
            };
            if assigned.is_empty() && open == 0 && row.count != Some(0) {
                let required = row
                    .count
                    .map(|count| format!("; required exactly {count}"))
                    .unwrap_or_default();
                report.finding(
                    format!("{name} matched no object{place}{required}"),
                    evidence,
                    assigned,
                );
                continue;
            }
            if let Some(count) = row.count {
                let found = i64::try_from(assigned.len()).unwrap_or(i64::MAX);
                let most = found.saturating_add(i64::try_from(open).unwrap_or(i64::MAX));
                if found > count || most < count {
                    report.finding(
                        format!("{name} has {found} object(s){place}; required exactly {count}"),
                        evidence.clone(),
                        assigned.clone(),
                    );
                } else if open > 0 {
                    report.not_evaluated(&format!(
                        "{name} has {found} object(s){place} and {open} more that may belong to it; required exactly {count}"
                    ));
                }
            }
            if let Some((area, tolerance)) = row.area {
                self.judge_area(
                    context,
                    &mut report,
                    &name,
                    &place,
                    (area, tolerance),
                    assigned,
                    open,
                    evidence,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn judge_area(
        &self,
        context: &RuleContext<'_>,
        report: &mut Report<'_, '_>,
        name: &str,
        place: &str,
        (area, tolerance): (f64, f64),
        assigned: Vec<ObjectId>,
        open: usize,
        mut evidence: Vec<Evidence>,
    ) {
        let measured = (|| {
            let mut sum = Sum::default();
            for member in &assigned {
                match self.area_property {
                    Some(property) => {
                        let stated =
                            Sum::areas(context, Some(property), std::slice::from_ref(member))?;
                        sum.lower += stated.lower;
                        sum.upper += stated.upper;
                        sum.evidence.extend(stated.evidence);
                    }
                    None => sum.add(&footprint(context, member)?),
                }
            }
            Ok::<_, Unavailable>(sum)
        })();
        let sum = match measured {
            Ok(sum) => sum,
            Err((reason, message)) => {
                report.not_evaluated_because(reason, &format!("{name}{place}: {message}"));
                return;
            }
        };
        evidence.extend(sum.evidence);
        let required = if tolerance > 0.0 {
            format!("{area} ± {tolerance} m²")
        } else {
            format!("{area} m²")
        };
        let (low, high) = (area - tolerance, area + tolerance);
        let summed = shown(sum.lower, sum.upper);
        // Objects that may still belong to the row can only add area.
        if sum.lower > high || (open == 0 && sum.upper < low) {
            report.finding(
                format!("{name} sums {summed} m²{place}; required {required}"),
                evidence,
                assigned,
            );
        } else if open > 0 {
            report.not_evaluated(&format!(
                "{name} sums {summed} m²{place} and {open} more object(s) may belong to it; required {required}"
            ));
        } else if sum.lower < low || sum.upper > high {
            report.not_evaluated(&format!(
                "{name} sums {summed} m²{place}, which straddles the required {required}"
            ));
        }
    }
}

/// Where a group's outcomes go.
struct Report<'r, 'e> {
    rule: &'r CompiledRule,
    scope: &'r Scope,
    evaluation: &'e mut CapabilityEvaluation,
}

impl Report<'_, '_> {
    fn finding(&mut self, message: String, evidence: Vec<Evidence>, related: Vec<ObjectId>) {
        self.evaluation.push_finding(
            Finding::new(
                self.rule.id.clone(),
                self.scope.clone(),
                severity(self.rule),
                message,
            )
            .with_evidence(evidence)
            .with_related(related),
        );
    }

    fn not_evaluated(&mut self, message: &str) {
        self.not_evaluated_because(NotEvaluatedReason::IncompleteEvidence, message);
    }

    fn not_evaluated_because(&mut self, reason: NotEvaluatedReason, message: &str) {
        let message = format!("table-allocation: {message}");
        match self.scope {
            Scope::Object(object) => {
                self.evaluation
                    .push_object_not_evaluated(object.clone(), reason, message);
            }
            Scope::Source(source) => {
                self.evaluation
                    .push_source_not_evaluated(source.clone(), reason, message);
            }
            Scope::Project => self.evaluation.push_not_evaluated(reason, message),
        }
    }
}
