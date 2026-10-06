//! Exclusive allocation of objects to table rows, with per-row count and
//! summed-area requirements.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, Deviation, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::{Evidence, Finding, Object, ObjectId, PropertyValue, Scope};

use crate::counts::{Population, real, relation_text, tally};
use crate::pairs::severity;
use crate::plan_area::{Sum, footprint, shown};
use crate::selection::select_objects;
use crate::support::table::{self, Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, invalid, resolve, sources,
    traversal_parameters,
};

/// The key columns, each testing the property the same-named parameter
/// names. A capability declares the ones it takes; an undeclared key stays
/// empty.
pub(crate) const KEYS: [&str; KEY_COUNT] = ["key_1", "key_2", "key_3", "key_4"];

/// How many keys a row can test.
pub(crate) const KEY_COUNT: usize = 4;

/// A row's key cells: each compiled pattern with its text as written.
pub(crate) type KeyCells<'a> = [Option<(TextPattern, &'a str)>; KEY_COUNT];

/// Key properties, one per key column.
pub(crate) type KeyProperties<'a> = [Option<PropertyRef<'a>>; KEY_COUNT];

/// The key properties the `key_1` to `key_4` parameters name.
pub(crate) fn key_properties<'a>(
    parameters: &Parameters<'a>,
) -> Result<KeyProperties<'a>, Unavailable> {
    Ok([
        parameters.property(KEYS[0])?,
        parameters.property(KEYS[1])?,
        parameters.property(KEYS[2])?,
        parameters.property(KEYS[3])?,
    ])
}

/// The key cells of row `number`; a cell whose property is not declared is
/// an invalid declaration.
pub(crate) fn key_cells<'a>(
    row: table::Row<'a>,
    number: usize,
    properties: &KeyProperties<'_>,
    case_sensitive: bool,
) -> Result<KeyCells<'a>, Unavailable> {
    let mut keys = [None, None, None, None];
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
    properties: &KeyProperties<'_>,
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
    properties: &KeyProperties<'_>,
    used: [bool; KEY_COUNT],
) -> ([Option<Key>; KEY_COUNT], Vec<Evidence>) {
    let mut evidence = Vec::new();
    let mut keys = [None, None, None, None];
    for ((slot, property), used) in keys.iter_mut().zip(properties).zip(used) {
        let Some(property) = property.filter(|_| used) else {
            continue;
        };
        *slot = Some(read_key(context, object, property, &mut evidence));
    }
    (keys, evidence)
}

/// One key value of `object`, citing what was read.
pub(crate) fn read_key(
    context: &RuleContext<'_>,
    object: &Object,
    property: PropertyRef<'_>,
    evidence: &mut Vec<Evidence>,
) -> Key {
    match resolve(context, object, property) {
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
    }
}

/// Whether a pattern cell matches one key value: an absent value matches
/// no pattern, an unknown one decides nothing.
pub(crate) fn test_key(pattern: &TextPattern, key: Option<&Key>) -> RowTest {
    match key {
        Some(Key::Text(text)) => pattern.test(text),
        Some(Key::Absent) => RowTest::NoMatch,
        Some(Key::Unknown(..)) | None => RowTest::Undecided,
    }
}

/// Whether a row's key cells match an object's key values: an absent value
/// matches no pattern, an unknown one decides nothing.
pub(crate) fn test_keys(cells: &KeyCells<'_>, keys: &[Option<Key>; KEY_COUNT]) -> RowTest {
    cells
        .iter()
        .zip(keys)
        .fold(RowTest::Match(0), |test, (pattern, key)| {
            let Some((pattern, _)) = pattern else {
                return test;
            };
            test.and(test_key(pattern, key.as_ref()))
        })
}

/// Why an object's keys leave a row undecided: its first unknown key.
pub(crate) fn unknown_key(keys: &[Option<Key>; KEY_COUNT]) -> Unavailable {
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
    properties: &KeyProperties<'_>,
    keys: &[Option<Key>; KEY_COUNT],
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
    TableColumn::optional("key_4", ColumnKind::TextPattern),
    TableColumn::optional("anchor", ColumnKind::TextPattern),
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::optional("count", ColumnKind::Integer),
    TableColumn::optional("area", ColumnKind::Number),
    TableColumn::optional("area_tolerance", ColumnKind::Number),
    TableColumn::optional("area_tolerance_ratio", ColumnKind::Number),
];

/// Assigns each selected object to exactly one row of a table, then checks
/// every row's assigned objects: how many, and their summed area.
///
/// Rows are keyed by text patterns: the parameters `key_1` to `key_4` name
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
/// square metres, or ± `area_tolerance_ratio` of `area`. Areas are measured
/// footprints, or an area quantity stated by `area_property`. With
/// `area_mode` `each`, the area is no sum: an object fits a row only when
/// its own area lies within the row's, a match condition beside the keys,
/// and one whose area straddles it is undecided.
///
/// With `anchor_key`, a row filling the `anchor` cell (a pattern over the
/// anchor's `anchor_key` value, such as a storey name) applies only in the
/// anchors it matches, and its match adds to the row's specificity, so one
/// rule states different rows per storey. An anchor whose key cannot be
/// read is not evaluated.
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

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("rows", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::optional("mode", ParameterType::String),
            ParameterDescriptor::optional("key_1", ParameterType::PropertyReference),
            ParameterDescriptor::optional("key_2", ParameterType::PropertyReference),
            ParameterDescriptor::optional("key_3", ParameterType::PropertyReference),
            ParameterDescriptor::optional("key_4", ParameterType::PropertyReference),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("area_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("area_mode", ParameterType::String),
            ParameterDescriptor::optional("anchor_selector", ParameterType::Selector),
            ParameterDescriptor::optional("anchor_key", ParameterType::PropertyReference),
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
            first: None,
            matched: selected.iter().map(|object| object.id.clone()).collect(),
            undecided: evaluation
                .not_evaluated_outcomes()
                .iter()
                .filter_map(|outcome| outcome.object_id().cloned())
                .collect(),
        };

        let mut allocator = Allocator {
            context,
            rule,
            declaration: &declaration,
            assignments: BTreeMap::new(),
            areas: BTreeMap::new(),
            reported: BTreeSet::new(),
        };
        let every: Applicable = vec![Some(0); declaration.rows.len()];
        // Without anchor rows each object is assigned once, whatever groups
        // it is counted in, and even when no anchor reaches it.
        if !declaration.anchored() {
            for object in &selected {
                allocator.assignment(&every, object, &mut evaluation);
            }
        }

        let groups = match declaration.groups(context, &members, &mut evaluation) {
            Ok(groups) => groups,
            Err((reason, message)) => {
                evaluation.push_not_evaluated(reason, format!("table-allocation: {message}"));
                return evaluation;
            }
        };
        for group in groups {
            let applicable = if declaration.anchored() {
                match declaration.applicable(context, &group) {
                    Ok(applicable) => applicable,
                    Err((reason, message)) => {
                        Report {
                            rule,
                            scope: &group.scope,
                            evaluation: &mut evaluation,
                        }
                        .not_evaluated_because(reason, &message);
                        continue;
                    }
                }
            } else {
                every.clone()
            };
            for member in &group.members {
                if let Some(object) = context.project.object(member) {
                    allocator.assignment(&applicable, object, &mut evaluation);
                }
            }
            let assigned = allocator
                .assignments
                .get(&applicable)
                .cloned()
                .unwrap_or_default();
            declaration.judge(
                context,
                rule,
                &group,
                &applicable,
                &assigned,
                &mut evaluation,
            );
        }
        evaluation
    }
}

/// Per row, whether it applies in a group and the specificity its anchor
/// match adds; `None` where it does not apply.
type Applicable = Vec<Option<u32>>;

/// Where each object belongs among the rows that apply, with the evidence.
type Assignments = BTreeMap<ObjectId, (Assignment, Vec<Evidence>)>;

/// Assigns objects to rows, once per set of applicable rows, and reports an
/// extra or undecided object once however many groups hold it.
struct Allocator<'r, 'a> {
    context: &'r RuleContext<'r>,
    rule: &'r CompiledRule,
    declaration: &'r Declaration<'a>,
    assignments: BTreeMap<Applicable, Assignments>,
    /// Each object's own area, for `area_mode` `each`.
    areas: BTreeMap<ObjectId, Result<Sum, Unavailable>>,
    reported: BTreeSet<ObjectId>,
}

impl Allocator<'_, '_> {
    fn assignment(
        &mut self,
        applicable: &Applicable,
        object: &Object,
        evaluation: &mut CapabilityEvaluation,
    ) {
        if self
            .assignments
            .get(applicable)
            .is_some_and(|assigned| assigned.contains_key(&object.id))
        {
            return;
        }
        let (assignment, evidence) =
            self.declaration
                .assign(self.context, object, applicable, &mut self.areas);
        if self.reported.insert(object.id.clone()) {
            match &assignment {
                Assignment::Extra(keys) => evaluation.push_finding(
                    Finding::new(
                        self.rule.id.clone(),
                        Scope::Object(object.id.clone()),
                        severity(self.rule),
                        format!("no row matches ({keys})"),
                    )
                    .with_evidence(evidence.clone()),
                ),
                Assignment::Open(_, reason, message) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    reason.clone(),
                    message.clone(),
                ),
                Assignment::Row(_) => {
                    self.reported.remove(&object.id);
                }
            }
        }
        self.assignments
            .entry(applicable.clone())
            .or_default()
            .insert(object.id.clone(), (assignment, evidence));
    }
}

/// One row as declared, with its keys compiled.
struct Row<'a> {
    /// One-based, as a reviewer counts.
    number: usize,
    label: Option<&'a str>,
    keys: KeyCells<'a>,
    /// The `anchor` cell, over the anchor's `anchor_key`.
    anchor: Option<(TextPattern, &'a str)>,
    count: Option<i64>,
    area: Option<Area>,
}

impl Row<'_> {
    fn name(&self, properties: &KeyProperties<'_>) -> String {
        let name = row_name(self.number, self.label, &self.keys, properties);
        match &self.anchor {
            Some((_, pattern)) if self.label.is_none() => {
                format!("{name} in anchors like `{pattern}`")
            }
            _ => name,
        }
    }
}

/// A row's required area, square metres.
#[derive(Clone, Copy)]
struct Area {
    target: f64,
    tolerance: f64,
    /// The tolerance as a share of the target, when it was written so.
    ratio: Option<f64>,
}

impl Area {
    fn bounds(self) -> (f64, f64) {
        (self.target - self.tolerance, self.target + self.tolerance)
    }

    fn required(self) -> String {
        match self.ratio {
            Some(ratio) => format!("{} m² ± {} %", self.target, ratio * 100.0),
            None if self.tolerance > 0.0 => format!("{} ± {} m²", self.target, self.tolerance),
            None => format!("{} m²", self.target),
        }
    }
}

/// An object's key value.
#[derive(Clone)]
pub(crate) enum Key {
    Text(String),
    /// Exactly absent, or null: no pattern matches it.
    Absent,
    Unknown(NotEvaluatedReason, String),
}

/// Where an object belongs.
#[derive(Clone)]
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
    properties: KeyProperties<'a>,
    area_property: Option<PropertyRef<'a>>,
    /// Whether a row's area is each object's own (a match condition)
    /// rather than the sum of its objects.
    each: bool,
    anchors: Option<&'a axioval_ir::contract::Selector>,
    anchor_key: Option<PropertyRef<'a>>,
    traversal: Option<Traversal>,
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
        let anchor_key = parameters.property("anchor_key")?;
        let each = match parameters.string("area_mode")? {
            None | Some("sum") => false,
            Some("each") => true,
            Some(other) => {
                return Err(invalid(format!(
                    "area_mode `{other}` is unsupported; use `sum` or `each`"
                )));
            }
        };
        if anchor_key.is_some() && anchors.is_none() {
            return Err(invalid("`anchor_key` needs `anchor_selector`"));
        }
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
            let area = parse_area(row, number)?;
            let anchor = match row.text("anchor")? {
                None => None,
                Some(_) if anchor_key.is_none() => {
                    return Err(invalid(format!(
                        "row {number} fills `anchor`, but no `anchor_key` property is declared"
                    )));
                }
                Some(text) => Some((
                    row.pattern("anchor", case_sensitive)?
                        .expect("a filled cell compiles"),
                    text,
                )),
            };
            rows.push(Row {
                number,
                label: row.text("label")?,
                keys,
                anchor,
                count,
                area,
            });
        }
        if anchor_key.is_some() && rows.iter().all(|row| row.anchor.is_none()) {
            return Err(invalid(
                "`anchor_key` is declared, but no row fills `anchor`",
            ));
        }
        Ok(Self {
            rows,
            selection,
            properties,
            area_property: parameters.property("area_property")?,
            each,
            anchors,
            anchor_key,
            traversal,
            across_sources,
        })
    }

    /// Whether some row applies only in the anchors its `anchor` matches.
    fn anchored(&self) -> bool {
        self.anchor_key.is_some()
    }

    /// Which rows apply in `group`, from its anchor's `anchor_key`.
    fn applicable(
        &self,
        context: &RuleContext<'_>,
        group: &Group,
    ) -> Result<Applicable, Unavailable> {
        let (Some(property), Scope::Object(anchor)) = (self.anchor_key, &group.scope) else {
            return Ok(vec![Some(0); self.rows.len()]);
        };
        let object = context.project.object(anchor).ok_or((
            NotEvaluatedReason::InvalidEvidence,
            "the anchor is not in the project".to_owned(),
        ))?;
        let key = read_key(context, object, property, &mut Vec::new());
        self.rows
            .iter()
            .map(|row| {
                let Some((pattern, _)) = &row.anchor else {
                    return Ok(Some(0));
                };
                match test_key(pattern, Some(&key)) {
                    RowTest::Match(specificity) => Ok(Some(specificity)),
                    RowTest::NoMatch => Ok(None),
                    RowTest::Undecided => {
                        let (reason, message) = unknown_key(&[Some(key.clone()), None, None, None]);
                        Err((
                            reason,
                            format!("which rows apply to the anchor is undecided: {message}"),
                        ))
                    }
                }
            })
            .collect()
    }

    /// One object's own area, measured or stated.
    fn area_of(&self, context: &RuleContext<'_>, member: &ObjectId) -> Result<Sum, Unavailable> {
        if let Some(property) = self.area_property {
            return Sum::areas(context, Some(property), std::slice::from_ref(member));
        }
        let mut sum = Sum::default();
        sum.add(&footprint(context, member)?);
        Ok(sum)
    }

    /// The row `object` belongs to among the `applicable` ones, with the
    /// evidence for its key values and area.
    fn assign(
        &self,
        context: &RuleContext<'_>,
        object: &Object,
        applicable: &Applicable,
        areas: &mut BTreeMap<ObjectId, Result<Sum, Unavailable>>,
    ) -> (Assignment, Vec<Evidence>) {
        // Only keys some row tests are read.
        let used = [0, 1, 2, 3].map(|index| self.rows.iter().any(|row| row.keys[index].is_some()));
        let (keys, mut evidence) = read_keys(context, object, &self.properties, used);
        let mut area_open = None;
        let mut test = |index: usize, row: &Row<'_>| {
            let Some(anchor) = applicable[index] else {
                return RowTest::NoMatch;
            };
            let keyed = RowTest::Match(anchor).and(test_keys(&row.keys, &keys));
            let Some(area) = row.area.filter(|_| self.each && keyed != RowTest::NoMatch) else {
                return keyed;
            };
            let own = areas
                .entry(object.id.clone())
                .or_insert_with(|| self.area_of(context, &object.id));
            let fits = match own {
                Ok(own) => {
                    let (low, high) = area.bounds();
                    if own.lower >= low && own.upper <= high {
                        RowTest::Match(0)
                    } else if own.upper < low || own.lower > high {
                        RowTest::NoMatch
                    } else {
                        area_open.get_or_insert((
                            NotEvaluatedReason::IncompleteEvidence,
                            format!(
                                "its area {} m² straddles {}",
                                shown(own.lower, own.upper),
                                area.required()
                            ),
                        ));
                        RowTest::Undecided
                    }
                }
                Err(why) => {
                    area_open.get_or_insert(why.clone());
                    RowTest::Undecided
                }
            };
            keyed.and(fits)
        };
        let tests: Vec<RowTest> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| test(index, row))
            .collect();
        if let Some(Ok(own)) = areas.get(&object.id) {
            evidence.extend(own.evidence.iter().cloned());
        }
        let indexed: Vec<usize> = (0..self.rows.len()).collect();
        let assignment = match match_rows(&indexed, self.selection, |index| tests[*index]) {
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
                let candidates = tests
                    .iter()
                    .enumerate()
                    .filter(|(_, test)| **test != RowTest::NoMatch)
                    .map(|(index, _)| index)
                    .collect();
                let (reason, message) = if keys
                    .iter()
                    .flatten()
                    .any(|key| matches!(key, Key::Unknown(..)))
                {
                    unknown_key(&keys)
                } else {
                    area_open.unwrap_or_else(|| unknown_key(&keys))
                };
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

    /// Judges every applicable row within one group.
    fn judge(
        &self,
        context: &RuleContext<'_>,
        rule: &CompiledRule,
        group: &Group,
        applicable: &Applicable,
        assignments: &Assignments,
        evaluation: &mut CapabilityEvaluation,
    ) {
        let place = match &group.scope {
            Scope::Source(source) => format!(" in source `{source}`"),
            Scope::Project => " in the project".to_owned(),
            Scope::Object(_) => String::new(),
        };
        for (index, row) in self.rows.iter().enumerate() {
            if applicable[index].is_none() {
                continue;
            }
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
                report.graded(
                    format!("{name} matched no object{place}{required}"),
                    evidence,
                    assigned,
                    row.count
                        .map(|count| Deviation::below(real(count), 0.0, 0.0)),
                );
                continue;
            }
            if let Some(count) = row.count {
                let found = i64::try_from(assigned.len()).unwrap_or(i64::MAX);
                let most = found.saturating_add(i64::try_from(open).unwrap_or(i64::MAX));
                if found > count || most < count {
                    let (least, greatest) = (real(found), real(most));
                    report.graded(
                        format!("{name} has {found} object(s){place}; required exactly {count}"),
                        evidence.clone(),
                        assigned.clone(),
                        Some(if found > count {
                            Deviation::above(real(count), least, greatest)
                        } else {
                            Deviation::below(real(count), least, greatest)
                        }),
                    );
                } else if open > 0 {
                    report.not_evaluated(&format!(
                        "{name} has {found} object(s){place} and {open} more that may belong to it; required exactly {count}"
                    ));
                }
            }
            if let Some(area) = row.area.filter(|_| !self.each) {
                self.judge_area(
                    context,
                    &mut report,
                    &name,
                    &place,
                    area,
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
        area: Area,
        assigned: Vec<ObjectId>,
        open: usize,
        mut evidence: Vec<Evidence>,
    ) {
        let measured = (|| {
            let mut sum = Sum::default();
            for member in &assigned {
                let own = self.area_of(context, member)?;
                sum.lower += own.lower;
                sum.upper += own.upper;
                sum.evidence.extend(own.evidence);
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
        let required = area.required();
        let (low, high) = area.bounds();
        let summed = shown(sum.lower, sum.upper);
        // Objects that may still belong to the row can only add area.
        if sum.lower > high || (open == 0 && sum.upper < low) {
            let deviation = if sum.lower > high {
                Deviation::above(high, sum.lower, sum.upper)
            } else {
                Deviation::below(low, sum.lower, sum.upper)
            };
            report.graded(
                format!("{name} sums {summed} m²{place}; required {required}"),
                evidence,
                assigned,
                Some(deviation),
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

/// A row's `area` with its absolute or relative tolerance.
fn parse_area(row: table::Row<'_>, number: usize) -> Result<Option<Area>, Unavailable> {
    let tolerance = row.number("area_tolerance")?;
    let ratio = row.number("area_tolerance_ratio")?;
    let Some(target) = row.number("area")? else {
        if tolerance.is_some() || ratio.is_some() {
            return Err(invalid(format!(
                "row {number} has an area tolerance but no area"
            )));
        }
        return Ok(None);
    };
    if tolerance.is_some() && ratio.is_some() {
        return Err(invalid(format!(
            "row {number} states `area_tolerance` and `area_tolerance_ratio`; state one"
        )));
    }
    if target < 0.0 || tolerance.is_some_and(|t| t < 0.0) || ratio.is_some_and(|r| r < 0.0) {
        return Err(invalid(format!(
            "row {number} has a negative area or tolerance"
        )));
    }
    Ok(Some(Area {
        target,
        tolerance: tolerance.unwrap_or_else(|| ratio.map_or(0.0, |ratio| target * ratio)),
        ratio,
    }))
}

/// Where a group's outcomes go.
struct Report<'r, 'e> {
    rule: &'r CompiledRule,
    scope: &'r Scope,
    evaluation: &'e mut CapabilityEvaluation,
}

impl Report<'_, '_> {
    /// A finding of a count or area missing its bound by `deviation`.
    fn graded(
        &mut self,
        message: String,
        evidence: Vec<Evidence>,
        related: Vec<ObjectId>,
        deviation: Option<Deviation>,
    ) {
        self.evaluation.push_finding_deviating(
            Finding::new(
                self.rule.id.clone(),
                self.scope.clone(),
                severity(self.rule),
                message,
            )
            .with_evidence(evidence)
            .with_related(related),
            deviation,
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
