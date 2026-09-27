//! Tables of property requirements: which properties an object must, may or
//! must not carry, and the values they may hold.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::plan_area::{Verdict, footprint, judge, shown};
use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::table::{Matched, Row, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    MAX_DECIMALS, Parameters, PropertyRef, Unavailable, category_prefix, display, exact_f64,
    finding, invalid, resolve, round_decimal, si_quantity, undefined, value_key,
};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("applies_to", ColumnKind::Selector),
    TableColumn::optional("property_set", ColumnKind::TextPattern),
    TableColumn::optional("property", ColumnKind::TextPattern),
    TableColumn::optional("requirement", ColumnKind::String),
    TableColumn::optional("state", ColumnKind::String),
    TableColumn::optional("presence", ColumnKind::String),
    TableColumn::optional("value_like", ColumnKind::TextPattern),
    TableColumn::optional("one_of", ColumnKind::String),
    TableColumn::optional("one_of_like", ColumnKind::String),
    TableColumn::optional("contains", ColumnKind::String),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
    TableColumn::optional("unit", ColumnKind::String),
    TableColumn::optional("per", ColumnKind::String),
    TableColumn::optional("decimals", ColumnKind::Integer),
];

/// Checks each selected object against every row of a `requirements` table
/// that applies to it.
///
/// A row names a property by `property_set` and `property` and states what
/// must hold of it in one of two forms. A `requirement` row marks the
/// property `required`, `optional` or `forbidden`. A `state` row is a
/// filtered-template row: its statement, a `presence` (`defined`,
/// `undefined`, `empty`, `not-empty`) or value conditions, must hold
/// (`include`) or must not hold (`exclude`), and an `ignore` row is skipped.
///
/// Value conditions are `value_like` (a whole-value wildcard pattern),
/// `one_of` (values separated by `|`, a backslash escaping the next
/// character), `one_of_like` (wildcard patterns separated by `|`),
/// `contains` (a substring of a text value, or an element of a list value)
/// and a numeric range `minimum`/`maximum` in `unit`, optionally `per` the
/// object's `measured-area` (plan footprint), `stated-area`
/// (`area_property`) or `stated-volume` (`volume_property`), and optionally
/// rounded to `decimals` in the row's unit before it is bounded.
/// `applies_to` restricts a row to the objects a selector picks, such as one
/// exact class or a class with its subtypes; a blank cell applies the row to
/// every selected object.
///
/// Each row yields at most one finding per object, whose message begins with
/// its result: `missing property`, `missing value`, `forbidden property
/// present`, `forbidden value` or `wrong value`. With `category_property`
/// each finding starts with the object's category in brackets; with
/// `group_by_value` the findings of one row, category and result that found
/// the same value are one finding relating all their objects.
///
/// Property sets and properties are resolved by exact name only: the
/// property service cannot list an object's sets or properties, so a row
/// whose set or property name is a wildcard pattern, or a row naming a set
/// without a property (set presence), is reported not evaluated for the
/// whole rule while the other rows are checked.
pub struct PropertyRequirements;

impl RuleCapability for PropertyRequirements {
    fn id(&self) -> &'static str {
        "axioval:capability.property-requirements"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("requirements", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
            ParameterDescriptor::optional("area_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("volume_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("group_by_value", ParameterType::Boolean),
            ParameterDescriptor::optional("category_property", ParameterType::PropertyReference),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match parse(&Parameters(rule)) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("property-requirements: {message}"),
                );
            }
        };
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        for (index, requirement) in declared.rows.iter().enumerate() {
            if let (Target::Unsupported(why), false) = (&requirement.target, requirement.ignored())
            {
                evaluation.push_not_evaluated(
                    NotEvaluatedReason::MissingService,
                    format!("property-requirements row {index}: {why}"),
                );
            }
        }
        let mut groups = declared.group_by_value.then(BTreeMap::new);
        for subject in subjects {
            check(
                context,
                rule,
                &declared,
                subject,
                groups.as_mut(),
                &mut evaluation,
            );
        }
        for ((index, category, result, _), group) in groups.into_iter().flatten() {
            let count = group.objects.len();
            let objects = if count == 1 { "object" } else { "objects" };
            let first = group.objects[0].clone();
            evaluation.push_finding(finding(
                rule,
                &first,
                format!(
                    "{category}{result}: {} is {} on {count} {objects}{} (requirement row {index})",
                    group.property, group.shown, group.detail
                ),
                group.evidence,
                group.objects,
            ));
        }
        evaluation
    }
}

/// Whether a row's statement must hold, must not hold, or is skipped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Include,
    Exclude,
    Ignore,
}

/// A presence a `state` row states.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    /// Present, with any value, null included.
    Defined,
    /// Exactly absent.
    Undefined,
    /// Present but null, blank or an empty list.
    Empty,
    /// Present with a value.
    NotEmpty,
}

/// What a row states about its property.
#[derive(Clone, Copy)]
enum Statement {
    /// The property's presence.
    Presence(Presence),
    /// A value meeting the row's conditions; `optional` lets a missing or
    /// empty value pass a row that must hold.
    Value { optional: bool },
}

/// The property a row names.
enum Target {
    /// One exactly named property, optionally in one exactly named set.
    Exact { set: Option<String>, name: String },
    /// A row the property contract cannot answer, and why.
    Unsupported(String),
}

/// What a row's numeric bounds are divided by.
#[derive(Clone, Copy)]
enum Per {
    MeasuredArea,
    StatedArea,
    StatedVolume,
}

impl Per {
    fn describe(self) -> &'static str {
        match self {
            Self::MeasuredArea => "per m² of measured plan area",
            Self::StatedArea => "per m² of stated area",
            Self::StatedVolume => "per m³ of stated volume",
        }
    }
}

/// A numeric range: bounds as written in `unit`, or unit-free.
struct Range {
    minimum: Option<f64>,
    maximum: Option<f64>,
    /// The size of one `unit` in canonical SI units; 1 for a number.
    scale: f64,
    /// The declared unit as written and its dimension; `None` for a number.
    unit: Option<(String, QuantityDimension)>,
    /// The declared bounds as written, for messages.
    written: String,
    per: Option<Per>,
    /// Decimals in `unit` the value is rounded to before it is bounded.
    decimals: Option<u32>,
}

/// The conditions a row puts on a value; all must hold.
#[derive(Default)]
struct Condition {
    like: Option<(String, TextPattern)>,
    one_of: Vec<String>,
    one_of_like: Vec<(String, TextPattern)>,
    contains: Option<String>,
    range: Option<Range>,
    case_sensitive: bool,
}

impl Condition {
    fn is_empty(&self) -> bool {
        self.contains.is_none() && !self.per_element()
    }

    /// Whether a condition judges a list value element by element.
    fn per_element(&self) -> bool {
        self.like.is_some()
            || !self.one_of.is_empty()
            || !self.one_of_like.is_empty()
            || self.range.is_some()
    }

    fn describe(&self) -> String {
        let listed = |values: &mut dyn Iterator<Item = &String>| {
            values
                .map(|value| format!("`{value}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut parts = Vec::new();
        if let Some((pattern, _)) = &self.like {
            parts.push(format!("like `{pattern}`"));
        }
        if !self.one_of.is_empty() {
            parts.push(format!("one of {}", listed(&mut self.one_of.iter())));
        }
        if !self.one_of_like.is_empty() {
            parts.push(format!(
                "like one of {}",
                listed(&mut self.one_of_like.iter().map(|(pattern, _)| pattern))
            ));
        }
        if let Some(text) = &self.contains {
            parts.push(format!("containing `{text}`"));
        }
        if let Some(range) = &self.range {
            let mut text = range.written.clone();
            if let Some(per) = range.per {
                text.push(' ');
                text.push_str(per.describe());
            }
            if let Some(decimals) = range.decimals {
                text = format!("{text} (rounded to {decimals} decimal(s))");
            }
            parts.push(text);
        }
        parts.join(" and ")
    }

    fn fold(&self, text: &str) -> String {
        if self.case_sensitive {
            text.to_owned()
        } else {
            text.to_lowercase()
        }
    }
}

/// One row of the requirements table.
struct RequirementRow {
    applies_to: Option<Selector>,
    target: Target,
    state: State,
    statement: Statement,
    condition: Condition,
}

impl RequirementRow {
    fn ignored(&self) -> bool {
        self.state == State::Ignore
    }
}

struct Declared<'a> {
    rows: Vec<RequirementRow>,
    area_property: Option<PropertyRef<'a>>,
    volume_property: Option<PropertyRef<'a>>,
    group_by_value: bool,
    category: Option<PropertyRef<'a>>,
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Declared<'a>, Unavailable> {
    let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
    let area_property = parameters.property("area_property")?;
    let volume_property = parameters.property("volume_property")?;
    let table = parameters
        .table("requirements")?
        .ok_or_else(|| invalid("parameter `requirements` is required"))?;
    let mut rows = Vec::with_capacity(table.len());
    for (index, row) in table.into_iter().enumerate() {
        let parsed = parse_row(row, case_sensitive, area_property, volume_property)
            .map_err(|(reason, message)| (reason, format!("row {index}: {message}")))?;
        rows.push(parsed);
    }
    Ok(Declared {
        rows,
        area_property,
        volume_property,
        group_by_value: parameters.boolean("group_by_value")?.unwrap_or(false),
        category: parameters.property("category_property")?,
    })
}

fn parse_row(
    row: Row<'_>,
    case_sensitive: bool,
    area_property: Option<PropertyRef<'_>>,
    volume_property: Option<PropertyRef<'_>>,
) -> Result<RequirementRow, Unavailable> {
    let condition = parse_condition(row, case_sensitive, area_property, volume_property)?;
    let (state, statement) = parse_statement(row, &condition)?;
    let set = row.text("property_set")?;
    let name = row.text("property")?;
    let target = match (set, name) {
        (None, None) => return Err(invalid("names neither a property set nor a property")),
        (Some(set), None) => {
            if !condition.is_empty() {
                return Err(invalid(
                    "a value condition needs a property, not only a property set",
                ));
            }
            Target::Unsupported(format!(
                "property set `{set}` without a property asks for the set's presence, but the \
                 property service resolves one named property and cannot list an object's \
                 property sets"
            ))
        }
        (set, Some(name)) => match (set.map(literal).transpose()?, literal(name)?) {
            (Some(None), _) | (_, None) => Target::Unsupported(format!(
                "`{}` is a name pattern, but the property service resolves one exactly named \
                 property and cannot list an object's property sets or properties",
                set.map_or_else(|| name.to_owned(), |set| format!("{set}.{name}"))
            )),
            (set, Some(name)) => Target::Exact {
                set: set.flatten(),
                name,
            },
        },
    };
    Ok(RequirementRow {
        applies_to: row.selector("applies_to")?.cloned(),
        target,
        state,
        statement,
        condition,
    })
}

/// A row's state and statement, from its `requirement` or its `state` and
/// `presence`.
fn parse_statement(row: Row<'_>, condition: &Condition) -> Result<(State, Statement), Unavailable> {
    let state = match row.text("state")? {
        None => None,
        Some("include") => Some(State::Include),
        Some("exclude") => Some(State::Exclude),
        Some("ignore") => Some(State::Ignore),
        Some(other) => {
            return Err(invalid(format!(
                "state `{other}` is not `include`, `exclude` or `ignore`"
            )));
        }
    };
    let presence = match row.text("presence")? {
        None => None,
        Some("defined") => Some(Presence::Defined),
        Some("undefined") => Some(Presence::Undefined),
        Some("empty") => Some(Presence::Empty),
        Some("not-empty") => Some(Presence::NotEmpty),
        Some(other) => {
            return Err(invalid(format!(
                "presence `{other}` is not `defined`, `undefined`, `empty` or `not-empty`"
            )));
        }
    };
    Ok(match (row.text("requirement")?, state, presence) {
        (Some(_), _, Some(_)) => {
            return Err(invalid(
                "`presence` belongs to a `state` row; a `requirement` states presence itself",
            ));
        }
        (Some(_), Some(State::Exclude), _) => {
            return Err(invalid(
                "a `requirement` row cannot be excluded; use `forbidden` or a `state` row",
            ));
        }
        (Some(requirement), state, None) => {
            let empty = condition.is_empty();
            let statement = match requirement {
                "required" if empty => (State::Include, Statement::Presence(Presence::NotEmpty)),
                "required" => (State::Include, Statement::Value { optional: false }),
                "optional" => (State::Include, Statement::Value { optional: true }),
                "forbidden" if empty => (State::Exclude, Statement::Presence(Presence::Defined)),
                "forbidden" => (State::Exclude, Statement::Value { optional: false }),
                other => {
                    return Err(invalid(format!(
                        "requirement `{other}` is not `required`, `optional` or `forbidden`"
                    )));
                }
            };
            match state {
                Some(State::Ignore) => (State::Ignore, statement.1),
                _ => statement,
            }
        }
        (None, None, _) => return Err(invalid("declare `requirement` or `state`")),
        (None, Some(state), Some(presence)) => {
            if !condition.is_empty() {
                return Err(invalid("a `presence` takes no value condition"));
            }
            (state, Statement::Presence(presence))
        }
        (None, Some(state), None) => {
            if condition.is_empty() {
                return Err(invalid(
                    "a `state` row needs a `presence` or a value condition",
                ));
            }
            (state, Statement::Value { optional: false })
        }
    })
}

/// The name a pattern cell spells literally, or `None` when it holds a
/// wildcard.
fn literal(pattern: &str) -> Result<Option<String>, Unavailable> {
    let mut name = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' | '?' => return Ok(None),
            '\\' => name.push(
                chars
                    .next()
                    .ok_or_else(|| invalid("a name pattern ends with a backslash"))?,
            ),
            other => name.push(other),
        }
    }
    if name.trim().is_empty() {
        return Err(invalid("a property set or property name is blank"));
    }
    Ok(Some(name))
}

fn parse_condition(
    row: Row<'_>,
    case_sensitive: bool,
    area_property: Option<PropertyRef<'_>>,
    volume_property: Option<PropertyRef<'_>>,
) -> Result<Condition, Unavailable> {
    let like = match row.text("value_like")? {
        Some(pattern) => Some((
            pattern.to_owned(),
            TextPattern::new(pattern, case_sensitive).map_err(invalid)?,
        )),
        None => None,
    };
    let one_of = match row.text("one_of")? {
        Some(values) => split_values("one_of", values, false)?,
        None => Vec::new(),
    };
    let one_of_like = match row.text("one_of_like")? {
        Some(patterns) => split_values("one_of_like", patterns, true)?
            .into_iter()
            .map(|pattern| {
                let compiled = TextPattern::new(&pattern, case_sensitive).map_err(invalid)?;
                Ok((pattern, compiled))
            })
            .collect::<Result<_, Unavailable>>()?,
        None => Vec::new(),
    };
    let contains = match row.text("contains")? {
        Some("") => return Err(invalid("`contains` is empty")),
        other => other.map(str::to_owned),
    };
    let range = parse_range(row, area_property, volume_property)?;
    Ok(Condition {
        like,
        one_of,
        one_of_like,
        contains,
        range,
        case_sensitive,
    })
}

/// A row's numeric range, if it declares a bound.
fn parse_range(
    row: Row<'_>,
    area_property: Option<PropertyRef<'_>>,
    volume_property: Option<PropertyRef<'_>>,
) -> Result<Option<Range>, Unavailable> {
    let minimum = row.number("minimum")?;
    let maximum = row.number("maximum")?;
    let unit = row.text("unit")?;
    let per = row.text("per")?;
    let decimals = row.integer("decimals")?;
    if minimum.is_none() && maximum.is_none() {
        if unit.is_some() || per.is_some() || decimals.is_some() {
            return Err(invalid(
                "`unit`, `per` and `decimals` need `minimum` or `maximum`",
            ));
        }
        return Ok(None);
    }
    if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
        return Err(invalid("minimum exceeds maximum"));
    }
    let decimals = match decimals {
        None => None,
        Some(value) if (0..=MAX_DECIMALS).contains(&value) => {
            Some(u32::try_from(value).expect("bounded above"))
        }
        Some(_) => {
            return Err(invalid(format!(
                "`decimals` must be between 0 and {MAX_DECIMALS}"
            )));
        }
    };
    let (scale, unit) = match unit {
        Some(unit) => {
            let (scale, dimension) = si_quantity(1.0, unit)?;
            (scale, Some((unit.to_owned(), dimension)))
        }
        None => (1.0, None),
    };
    let per = match per {
        None => None,
        Some("measured-area") => Some(Per::MeasuredArea),
        Some("stated-area") if area_property.is_some() => Some(Per::StatedArea),
        Some("stated-volume") if volume_property.is_some() => Some(Per::StatedVolume),
        Some("stated-area") => {
            return Err(invalid("`per` `stated-area` needs `area_property`"));
        }
        Some("stated-volume") => {
            return Err(invalid("`per` `stated-volume` needs `volume_property`"));
        }
        Some(other) => {
            return Err(invalid(format!(
                "`per` `{other}` is not `measured-area`, `stated-area` or `stated-volume`"
            )));
        }
    };
    let suffix = unit
        .as_ref()
        .map_or_else(String::new, |(unit, _)| format!(" {unit}"));
    let written = match (minimum, maximum) {
        (Some(low), Some(high)) => format!("between {low} and {high}{suffix}"),
        (Some(low), None) => format!("at least {low}{suffix}"),
        (None, Some(high)) => format!("at most {high}{suffix}"),
        (None, None) => unreachable!("a range has a bound"),
    };
    Ok(Some(Range {
        minimum,
        maximum,
        scale,
        unit,
        written,
        per,
        decimals,
    }))
}

/// Splits a list cell at unescaped `|`.
///
/// For plain values a backslash escapes the next character. For wildcard
/// patterns (`keep_escapes`) only `\|` is unescaped; every other escape is
/// kept for the pattern, so `\*` stays a literal star.
fn split_values(column: &str, cell: &str, keep_escapes: bool) -> Result<Vec<String>, Unavailable> {
    let mut values = vec![String::new()];
    let mut chars = cell.chars();
    while let Some(c) = chars.next() {
        match c {
            '|' => values.push(String::new()),
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or_else(|| invalid(format!("`{column}` ends with a backslash")))?;
                let value = values.last_mut().expect("never empty");
                if keep_escapes && escaped != '|' {
                    value.push('\\');
                }
                value.push(escaped);
            }
            other => values.last_mut().expect("never empty").push(other),
        }
    }
    if values.iter().any(String::is_empty) {
        return Err(invalid(format!("`{column}` has an empty value")));
    }
    Ok(values)
}

/// Grouped findings: (row, category heading, result, value key).
type GroupKey = (usize, String, &'static str, String);

/// Objects of one row, category and result that found one value.
struct Group {
    property: String,
    shown: String,
    detail: String,
    objects: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declared<'_>,
    subject: &Object,
    groups: Option<&mut BTreeMap<GroupKey, Group>>,
    evaluation: &mut CapabilityEvaluation,
) {
    let mut selected = Vec::new();
    let mut undecided = None;
    let matched = match_rows(&declared.rows, RowSelection::All, |row| {
        if row.ignored() {
            return RowTest::NoMatch;
        }
        let Some(selector) = &row.applies_to else {
            return RowTest::Match(0);
        };
        match selector_matches(context, selector, subject, &mut selected) {
            Selection::Match => RowTest::Match(0),
            Selection::NoMatch => RowTest::NoMatch,
            Selection::NotEvaluated(reason, message) => {
                undecided.get_or_insert((reason, message));
                RowTest::Undecided
            }
        }
    });
    let rows = match matched {
        Matched::Rows(rows) => rows,
        Matched::Undecided | Matched::Ambiguous(_) => {
            let (reason, message) = undecided.unwrap_or((
                NotEvaluatedReason::IncompleteEvidence,
                "the applicable rows cannot be decided".into(),
            ));
            evaluation.push_object_not_evaluated(
                subject.id.clone(),
                reason,
                format!("whether a requirement row applies cannot be decided: {message}"),
            );
            return;
        }
    };
    let mut failures = Vec::new();
    for (index, row) in rows {
        let Target::Exact { set, name } = &row.target else {
            continue;
        };
        let property = PropertyRef {
            set: set.as_deref(),
            name,
        };
        match check_row(context, declared, subject, row, property) {
            Ok(None) => {}
            Ok(Some(failure)) => failures.push((index, property.to_string(), failure)),
            Err((reason, message)) => evaluation.push_object_not_evaluated(
                subject.id.clone(),
                reason,
                format!("requirement row {index}: {message}"),
            ),
        }
    }
    if !failures.is_empty() {
        report(
            context, rule, declared, subject, failures, &selected, groups, evaluation,
        );
    }
}

/// Reports an object's failed rows, as findings of their own or into
/// `groups`, under the object's category when one is declared.
#[allow(
    clippy::too_many_arguments,
    reason = "one object's outcome and where it goes"
)]
fn report(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declared<'_>,
    subject: &Object,
    failures: Vec<(usize, String, Failure)>,
    selected: &[Evidence],
    mut groups: Option<&mut BTreeMap<GroupKey, Group>>,
    evaluation: &mut CapabilityEvaluation,
) {
    let (category, cited) = match declared.category {
        None => (String::new(), Vec::new()),
        Some(property) => match category_prefix(context, subject, property) {
            Ok(category) => category,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    reason,
                    format!("finding category {property} could not be read: {message}"),
                );
                return;
            }
        },
    };
    for (index, property, failure) in failures {
        let mut evidence = failure.evidence;
        evidence.extend(selected.iter().cloned());
        evidence.extend(cited.iter().cloned());
        let shown = display(failure.value.as_ref());
        match groups.as_deref_mut() {
            Some(groups) => {
                let key = failure.value.as_ref().map_or_else(
                    || "absent".to_owned(),
                    |value| value_key(value, false, true),
                );
                let group = groups
                    .entry((index, category.clone(), failure.result, key))
                    .or_insert_with(|| Group {
                        property,
                        shown,
                        detail: failure.detail,
                        objects: Vec::new(),
                        evidence: Vec::new(),
                    });
                group.objects.push(subject.id.clone());
                group.evidence.extend(evidence);
            }
            None => evaluation.push_finding(finding(
                rule,
                &subject.id,
                format!(
                    "{category}{}: {property} is {shown}{}{} (requirement row {index})",
                    failure.result,
                    failure
                        .quotient
                        .map_or_else(String::new, |q| format!(" ({q})")),
                    failure.detail
                ),
                evidence,
                Vec::new(),
            )),
        }
    }
}

/// How one row failed on one object.
struct Failure {
    /// The result a message begins with, such as `wrong value`.
    result: &'static str,
    /// The value found; `None` when the property is absent.
    value: Option<PropertyValue>,
    /// For a divided range, the quotient that was judged.
    quotient: Option<String>,
    /// What follows the value, such as `; required one of …`.
    detail: String,
    evidence: Vec<Evidence>,
}

const MISSING_PROPERTY: &str = "missing property";
const MISSING_VALUE: &str = "missing value";
const FORBIDDEN_PROPERTY: &str = "forbidden property present";
const FORBIDDEN_VALUE: &str = "forbidden value";
const WRONG_VALUE: &str = "wrong value";

fn check_row(
    context: &RuleContext<'_>,
    declared: &Declared<'_>,
    subject: &Object,
    row: &RequirementRow,
    property: PropertyRef<'_>,
) -> Result<Option<Failure>, Unavailable> {
    let resolved = resolve(context, subject, property)?;
    let mut evidence = resolved.evidence();
    let value = resolved.value();
    let fail = |result, detail: String, evidence| {
        Some(Failure {
            result,
            value: value.cloned(),
            quotient: None,
            detail,
            evidence,
        })
    };
    let empty = value.is_some() && undefined(value);
    let include = row.state == State::Include;
    let optional = match row.statement {
        Statement::Presence(presence) => {
            // An include row fails when its presence does not hold, an
            // exclude row when it does.
            let failure = match (include, presence, value.is_some(), empty) {
                (true, Presence::Defined | Presence::Empty | Presence::NotEmpty, false, _)
                | (false, Presence::Undefined, false, _) => Some((MISSING_PROPERTY, "")),
                (true, Presence::Undefined, true, _) | (false, Presence::Defined, true, _) => {
                    Some((FORBIDDEN_PROPERTY, ""))
                }
                (true, Presence::NotEmpty, true, true) | (false, Presence::Empty, true, true) => {
                    Some((MISSING_VALUE, ""))
                }
                (true, Presence::Empty, true, false) => Some((WRONG_VALUE, "; required empty")),
                (false, Presence::NotEmpty, true, false) => {
                    Some((FORBIDDEN_VALUE, ", which is not empty"))
                }
                _ => None,
            };
            return Ok(failure.and_then(|(result, detail)| fail(result, detail.into(), evidence)));
        }
        Statement::Value { optional } => optional,
    };
    let Some(found) = value.filter(|_| !empty) else {
        // A missing value cannot meet a condition: a row that must hold
        // fails unless the property is optional, one that must not passes.
        return Ok(if include && !optional {
            let result = if value.is_none() {
                MISSING_PROPERTY
            } else {
                MISSING_VALUE
            };
            fail(result, String::new(), evidence)
        } else {
            None
        });
    };
    let condition = &row.condition;
    let divisor = match condition.range.as_ref().and_then(|range| range.per) {
        Some(per) => Some(measure_divisor(
            context,
            declared,
            subject,
            per,
            &mut evidence,
        )?),
        None => None,
    };
    match meets(condition, found, divisor, include) {
        Holds::No(quotient) if include => Ok(Some(Failure {
            result: WRONG_VALUE,
            value: value.cloned(),
            quotient,
            detail: format!("; required {}", condition.describe()),
            evidence,
        })),
        Holds::Yes if !include => Ok(fail(
            FORBIDDEN_VALUE,
            format!(", which is {}", condition.describe()),
            evidence,
        )),
        Holds::Undecided(why) => Err((NotEvaluatedReason::IncompleteEvidence, why)),
        Holds::Yes | Holds::No(_) => Ok(None),
    }
}

/// Whether one value meets a condition.
enum Holds {
    Yes,
    /// It does not; for a divided range, the quotient that was judged.
    No(Option<String>),
    Undecided(String),
}

impl Holds {
    /// Both hold: a definite no decides, then an undecided part.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (no @ Self::No(_), _) | (_, no @ Self::No(_)) => no,
            (open @ Self::Undecided(_), _) | (_, open @ Self::Undecided(_)) => open,
            (Self::Yes, Self::Yes) => Self::Yes,
        }
    }
}

/// Whether a defined value meets a row's conditions.
///
/// `contains` judges the whole value. The other conditions judge a list, a
/// bounded value or a table by its stated values: for a row that must hold
/// every one must meet them, for one that must not a single one meeting
/// them is enough. A bounded value open on a side a range limits fails a
/// row that must hold.
fn meets(
    condition: &Condition,
    value: &PropertyValue,
    divisor: Option<Divisor>,
    every: bool,
) -> Holds {
    let whole = match &condition.contains {
        Some(wanted) => contains(condition, value, wanted),
        None => Holds::Yes,
    };
    if !condition.per_element() {
        return whole;
    }
    // A range holds every value between its bounds: one open on the side a
    // bound limits has values beyond it.
    if let (PropertyValue::Bounded { lower, upper, .. }, Some(range), true) =
        (value, &condition.range, every)
        && ((range.minimum.is_some() && lower.is_none())
            || (range.maximum.is_some() && upper.is_none()))
    {
        return whole.and(Holds::No(None));
    }
    let elements: Vec<&PropertyValue> = value.stated_values().unwrap_or_else(|| vec![value]);
    let mut open = None;
    let mut quantified = if every { Holds::Yes } else { Holds::No(None) };
    for element in elements {
        match (holds(condition, element, divisor), every) {
            (no @ Holds::No(_), true) => {
                quantified = no;
                break;
            }
            (Holds::Yes, false) => {
                quantified = Holds::Yes;
                break;
            }
            (Holds::Undecided(why), _) => {
                open.get_or_insert(why);
            }
            _ => {}
        }
    }
    let quantified = match (quantified, open) {
        // A definite answer the open elements cannot change.
        (decided @ Holds::No(_), _) if every => decided,
        (Holds::Yes, _) if !every => Holds::Yes,
        (_, Some(why)) => Holds::Undecided(why),
        (decided, None) => decided,
    };
    whole.and(quantified)
}

/// A value's text form for text conditions: text, booleans and integers.
fn text_of(value: &PropertyValue) -> Result<String, String> {
    match value {
        PropertyValue::String(text) => Ok(text.clone()),
        PropertyValue::Boolean(value) => Ok(value.to_string()),
        PropertyValue::Integer(value) => Ok(value.to_string()),
        other => Err(format!(
            "{} is not text, a boolean or an integer",
            display(Some(other))
        )),
    }
}

/// `contains`: a substring of a text value, or an element of a list value.
fn contains(condition: &Condition, value: &PropertyValue, wanted: &str) -> Holds {
    let wanted = condition.fold(wanted);
    match value {
        PropertyValue::String(text) => {
            if condition.fold(text).contains(&wanted) {
                Holds::Yes
            } else {
                Holds::No(None)
            }
        }
        PropertyValue::List(elements) => {
            let mut open = None;
            for element in elements {
                match text_of(element) {
                    Ok(text) if condition.fold(&text) == wanted => return Holds::Yes,
                    Ok(_) => {}
                    Err(why) => {
                        open.get_or_insert(why);
                    }
                }
            }
            open.map_or(Holds::No(None), Holds::Undecided)
        }
        other => Holds::Undecided(format!(
            "{} is neither text nor a list",
            display(Some(other))
        )),
    }
}

/// An area or volume interval `[lower, upper]` in canonical SI units.
type Divisor = (f64, f64);

fn measure_divisor(
    context: &RuleContext<'_>,
    declared: &Declared<'_>,
    subject: &Object,
    per: Per,
    evidence: &mut Vec<Evidence>,
) -> Result<Divisor, Unavailable> {
    let (property, dimension) = match per {
        Per::MeasuredArea => {
            let area = footprint(context, &subject.id)?;
            evidence.push(area.evidence().clone());
            return positive(area.lower_square_metres(), area.upper_square_metres());
        }
        Per::StatedArea => (declared.area_property, QuantityDimension::Area),
        Per::StatedVolume => (declared.volume_property, QuantityDimension::Volume),
    };
    let property = property.expect("checked when the row was read");
    let resolved = resolve(context, subject, property)?;
    evidence.extend(resolved.evidence());
    match resolved.value() {
        Some(PropertyValue::Quantity {
            value,
            dimension: stated,
        }) if *stated == dimension && value.is_finite() => positive(*value, *value),
        other => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{property} states no {} ({})",
                match dimension {
                    QuantityDimension::Area => "area",
                    _ => "volume",
                },
                display(other)
            ),
        )),
    }
}

fn positive(lower: f64, upper: f64) -> Result<Divisor, Unavailable> {
    if lower > 0.0 {
        Ok((lower, upper))
    } else {
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the divisor is {}, which is not positive",
                shown(lower, upper)
            ),
        ))
    }
}

/// Whether one scalar value meets a row's text conditions: `value_like`,
/// `one_of` and `one_of_like`.
fn text_holds(condition: &Condition, value: &PropertyValue) -> Holds {
    if condition.like.is_none() && condition.one_of.is_empty() && condition.one_of_like.is_empty() {
        return Holds::Yes;
    }
    let text = match text_of(value) {
        Ok(text) => text,
        Err(why) => return Holds::Undecided(why),
    };
    let like = |compiled: &TextPattern| matches!(compiled.test(&text), RowTest::Match(_));
    let wanted = condition.fold(&text);
    let met = condition
        .like
        .as_ref()
        .is_none_or(|(_, compiled)| like(compiled))
        && (condition.one_of.is_empty()
            || condition
                .one_of
                .iter()
                .any(|value| condition.fold(value) == wanted))
        && (condition.one_of_like.is_empty()
            || condition
                .one_of_like
                .iter()
                .any(|(_, compiled)| like(compiled)));
    if met { Holds::Yes } else { Holds::No(None) }
}

/// Whether one scalar value meets a row's per-element conditions.
fn holds(condition: &Condition, value: &PropertyValue, divisor: Option<Divisor>) -> Holds {
    match text_holds(condition, value) {
        Holds::Yes => {}
        other => return other,
    }
    let Some(range) = &condition.range else {
        return Holds::Yes;
    };
    let number = match (value, &range.unit) {
        (
            PropertyValue::Quantity {
                value: number,
                dimension,
            },
            Some((unit, wanted)),
        ) => {
            if dimension != wanted {
                return Holds::Undecided(format!(
                    "{} is not in the dimension of `{unit}`",
                    display(Some(value))
                ));
            }
            *number
        }
        (PropertyValue::Quantity { .. }, None) => {
            return Holds::Undecided(format!(
                "{} is a quantity, but the row's range declares no unit",
                display(Some(value))
            ));
        }
        (PropertyValue::Decimal(number), None) => *number,
        (PropertyValue::Integer(number), None) => match exact_f64(*number) {
            Some(number) => number,
            None => return Holds::Undecided(format!("{number} cannot be compared exactly")),
        },
        (PropertyValue::Decimal(_) | PropertyValue::Integer(_), Some((unit, _))) => {
            return Holds::Undecided(format!(
                "{} states no unit, but the row's range is in `{unit}`",
                display(Some(value))
            ));
        }
        (other, _) => {
            return Holds::Undecided(format!("{} is not a number", display(Some(other))));
        }
    };
    let (lower, upper) = match divisor {
        None => (number, number),
        // The divisor is positive, so the quotient's order follows the sign.
        Some((low, high)) if number >= 0.0 => (number / high, number / low),
        Some((low, high)) => (number / low, number / high),
    };
    let unit = range
        .unit
        .as_ref()
        .map_or_else(String::new, |(_, dimension)| {
            format!(" {}", dimension.unit_symbol())
        });
    let per = range
        .per
        .map_or_else(String::new, |per| format!(" {}", per.describe()));
    let verdict = match range.decimals {
        // Bounds in canonical SI units, so an exact value is not rescaled.
        None => judge(
            lower,
            upper,
            range.minimum.map(|bound| bound * range.scale),
            range.maximum.map(|bound| bound * range.scale),
        ),
        // Rounding reads the value in the row's unit, where its bounds are.
        Some(decimals) => judge(
            round_decimal(lower / range.scale, decimals),
            round_decimal(upper / range.scale, decimals),
            range.minimum,
            range.maximum,
        ),
    };
    match verdict {
        Verdict::Pass => Holds::Yes,
        // Only a quotient differs from the value the message shows.
        Verdict::Fail(_) => Holds::No(
            range
                .per
                .is_some()
                .then(|| format!("{}{unit}{per}", shown(lower, upper))),
        ),
        Verdict::Undecided(bound) => Holds::Undecided(format!(
            "{}{unit}{per} straddles the bound {bound} ({})",
            shown(lower, upper),
            match (range.decimals, &range.unit) {
                (Some(_), Some((unit, _))) => format!("in `{unit}`"),
                (Some(_), None) => "unit-free".to_owned(),
                (None, _) => "in canonical SI units".to_owned(),
            }
        )),
    }
}
