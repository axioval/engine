//! Tables of property requirements: which properties an object must, may or
//! must not carry, and the values they may hold.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NamePattern, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, PropertyEnumeration, RuleCapability, RuleContext,
    TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{
    Evidence, Object, ObjectId, Property, PropertyValue, QuantityDimension, TemporalPrecision,
};

use crate::plan_area::{Bound, Verdict, body_volume, face_area, footprint, judge_bounds, shown};
use crate::selection::{
    NameSpec, Selection, enumerate, select_objects, selector_matches, sets_without_match,
    xsd_name_pattern,
};
use crate::support::table::{Matched, Row, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    MAX_DECIMALS, Parameters, PropertyRef, Unavailable, category_prefix, display, exact_f64,
    finding, invalid, resolve, round_decimal, si_quantity, temporal_order, undefined, value_key,
};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("applies_to", ColumnKind::Selector),
    TableColumn::optional("property_set", ColumnKind::TextPattern),
    TableColumn::optional("property", ColumnKind::TextPattern),
    TableColumn::optional("property_set_pattern", ColumnKind::String),
    TableColumn::optional("property_pattern", ColumnKind::String),
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
    TableColumn::optional("minimum_exclusive", ColumnKind::Boolean),
    TableColumn::optional("maximum_exclusive", ColumnKind::Boolean),
    TableColumn::optional("minimum_date", ColumnKind::Date),
    TableColumn::optional("maximum_date", ColumnKind::Date),
    TableColumn::optional("minimum_date_time", ColumnKind::DateTime),
    TableColumn::optional("maximum_date_time", ColumnKind::DateTime),
    TableColumn::optional("precision", ColumnKind::String),
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
/// object's `measured-area` (plan footprint), `measured-volume` (certified
/// body volume), `measured-face-area` (its largest plane face, such as a
/// wall's side), `stated-area` (`area_property`) or `stated-volume`
/// (`volume_property`), and optionally rounded to `decimals` in the row's
/// unit before it is bounded. A date range bounds a date or date-time value
/// by `minimum_date`/`maximum_date` or `minimum_date_time`/
/// `maximum_date_time`, compared chronologically (`precision` `day`
/// compares a date-time with a date by its day). `minimum_exclusive` and
/// `maximum_exclusive` make either kind of bound exclude its own value, so
/// `> 0` fails on 0.
/// `applies_to` restricts a row to the objects a selector picks, such as one
/// exact class or a class with its subtypes; a blank cell applies the row to
/// every selected object.
///
/// Each row yields at most one finding per object, whose message begins with
/// its result: `missing property set`, `missing property`, `missing value`,
/// `forbidden property set present`, `forbidden property present`,
/// `forbidden value` or `wrong value`. With `category_property`
/// each finding starts with the object's category in brackets; with
/// `group_by_value` the findings of one row, category and result that found
/// the same value are one finding relating all their objects.
///
/// `property_set` and `property` hold exact names or wildcard patterns;
/// `property_set_pattern` and `property_pattern` hold XML Schema patterns
/// instead, as IDS names sets and properties (`Pset_.*Common`). A pattern
/// matches the whole name the source states, never a concept. A row with a
/// pattern is checked against every matching property, enumerated exactly
/// through the property service: an included statement must hold for each
/// and needs one to match, an excluded one must hold for none, and the
/// first failing property (by set and name) is reported. A row naming its
/// set as well needs a match in every set it names that holds a property,
/// as IDS requires. A row naming a set
/// without a property asks whether the set holds a property. A required
/// property whose named set holds none is a `missing property set`. A
/// source that cannot enumerate leaves a pattern or set row not evaluated,
/// and keeps `missing property` for an exact one.
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
        for ((index, category, result, ..), group) in groups.into_iter().flatten() {
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
    /// Every property whose set and name match, enumerated exactly; with
    /// `property` `Any`, a set-only row asking for the set's presence.
    Matched {
        set: NameSel,
        property: NameSel,
        shown: String,
    },
}

/// A set or property name of a row: any, exact, or a pattern as written.
enum NameSel {
    Any,
    Exact(String),
    Pattern(NamePattern, String),
}

impl NameSel {
    fn spec(&self) -> NameSpec<'_> {
        match self {
            Self::Any => NameSpec::Any,
            Self::Exact(name) => NameSpec::Exact(name),
            Self::Pattern(pattern, _) => NameSpec::Pattern(pattern),
        }
    }

    fn shown(&self) -> &str {
        match self {
            Self::Any => "*",
            Self::Exact(name) => name,
            Self::Pattern(_, shown) => shown,
        }
    }
}

/// What a row's numeric bounds are divided by.
#[derive(Clone, Copy)]
enum Per {
    MeasuredArea,
    MeasuredVolume,
    MeasuredFaceArea,
    StatedArea,
    StatedVolume,
}

impl Per {
    fn describe(self) -> &'static str {
        match self {
            Self::MeasuredArea => "per m² of measured plan area",
            Self::MeasuredVolume => "per m³ of measured volume",
            Self::MeasuredFaceArea => "per m² of measured face area",
            Self::StatedArea => "per m² of stated area",
            Self::StatedVolume => "per m³ of stated volume",
        }
    }
}

/// A numeric range: bounds as written in `unit`, or unit-free.
struct Range {
    minimum: Option<f64>,
    maximum: Option<f64>,
    minimum_exclusive: bool,
    maximum_exclusive: bool,
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

/// A date range: bounds as `date` or `dateTime` values, compared in time.
struct Dates {
    minimum: Option<PropertyValue>,
    maximum: Option<PropertyValue>,
    minimum_exclusive: bool,
    maximum_exclusive: bool,
    precision: Option<TemporalPrecision>,
    /// The declared bounds as written, for messages.
    written: String,
}

/// The conditions a row puts on a value; all must hold.
#[derive(Default)]
struct Condition {
    like: Option<(String, TextPattern)>,
    one_of: Vec<String>,
    one_of_like: Vec<(String, TextPattern)>,
    contains: Option<String>,
    range: Option<Range>,
    dates: Option<Dates>,
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
            || self.dates.is_some()
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
        if let Some(dates) = &self.dates {
            parts.push(dates.written.clone());
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
    let (state, mut statement) = parse_statement(row, &condition)?;
    let set = name_cell(row, "property_set", "property_set_pattern")?;
    let name = name_cell(row, "property", "property_pattern")?;
    let target = match (set, name) {
        (None, None) => return Err(invalid("names neither a property set nor a property")),
        (Some(set), None) => {
            if !condition.is_empty() {
                return Err(invalid(
                    "a value condition needs a property, not only a property set",
                ));
            }
            // A set is present when it holds a property: `required` asks
            // for that, and only presence or absence can be stated of it.
            statement = match (statement, row.text("requirement")?.is_some()) {
                (Statement::Presence(Presence::NotEmpty), true) => {
                    Statement::Presence(Presence::Defined)
                }
                (Statement::Presence(Presence::Empty | Presence::NotEmpty), _) => {
                    return Err(invalid(
                        "a property set without a property states only `defined` or `undefined`",
                    ));
                }
                (other, _) => other,
            };
            Target::Matched {
                shown: set.shown().to_owned(),
                set,
                property: NameSel::Any,
            }
        }
        (set, Some(NameSel::Exact(name))) if matches!(set, None | Some(NameSel::Exact(_))) => {
            Target::Exact {
                set: match set {
                    Some(NameSel::Exact(set)) => Some(set),
                    _ => None,
                },
                name,
            }
        }
        (set, Some(property)) => {
            let set = set.unwrap_or(NameSel::Any);
            Target::Matched {
                shown: match &set {
                    NameSel::Any => property.shown().to_owned(),
                    set => format!("{}.{}", set.shown(), property.shown()),
                },
                set,
                property,
            }
        }
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

/// A set or property name from its wildcard cell or its XML Schema
/// pattern cell, never both; `None` when neither is given.
fn name_cell(row: Row<'_>, wildcard: &str, pattern: &str) -> Result<Option<NameSel>, Unavailable> {
    match (row.text(wildcard)?, row.text(pattern)?) {
        (Some(_), Some(_)) => Err(invalid(format!(
            "declare `{wildcard}` or `{pattern}`, not both"
        ))),
        (Some(cell), None) => Ok(Some(match literal(cell)? {
            Some(name) => NameSel::Exact(name),
            None => NameSel::Pattern(
                NamePattern::new(wildcard_regex(cell)?)
                    .map_err(|error| invalid(error.to_string()))?,
                cell.to_owned(),
            ),
        })),
        (None, Some(cell)) => Ok(Some(NameSel::Pattern(
            xsd_name_pattern(cell)
                .map_err(|why| invalid(format!("`{pattern}` {cell:?}: {why}")))?,
            format!("/{cell}/"),
        ))),
        (None, None) => Ok(None),
    }
}

/// A wildcard cell as a regular expression: `*` any run, `?` one
/// character, a backslash making the next character literal.
fn wildcard_regex(cell: &str) -> Result<String, Unavailable> {
    let mut out = String::with_capacity(cell.len() + 8);
    let mut chars = cell.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str("(?s:.*)"),
            '?' => out.push_str("(?s:.)"),
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or_else(|| invalid("a name pattern ends with a backslash"))?;
                out.push_str(&regex::escape(&escaped.to_string()));
            }
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    Ok(out)
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
    let dates = parse_dates(row)?;
    if range.is_some() && dates.is_some() {
        return Err(invalid("a row bounds either a number or a date, not both"));
    }
    if range.is_none() && dates.is_none() {
        for column in ["minimum_exclusive", "maximum_exclusive"] {
            if row.boolean(column)?.is_some() {
                return Err(invalid(format!("`{column}` needs its bound")));
            }
        }
    }
    Ok(Condition {
        like,
        one_of,
        one_of_like,
        contains,
        range,
        dates,
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
    let (minimum_exclusive, maximum_exclusive) =
        exclusive_flags(row, minimum.is_some(), maximum.is_some())?;
    if (minimum_exclusive || maximum_exclusive)
        && matches!((minimum, maximum), (Some(low), Some(high)) if low >= high)
    {
        return Err(invalid(
            "an exclusive range between equal bounds holds no value",
        ));
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
        Some("measured-volume") => Some(Per::MeasuredVolume),
        Some("measured-face-area") => Some(Per::MeasuredFaceArea),
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
                "`per` `{other}` is not `measured-area`, `measured-volume`, `measured-face-area`, `stated-area` or `stated-volume`"
            )));
        }
    };
    let suffix = unit
        .as_ref()
        .map_or_else(String::new, |(unit, _)| format!(" {unit}"));
    let written = written_bounds(
        minimum.map(|low| (low.to_string(), minimum_exclusive)),
        maximum.map(|high| (high.to_string(), maximum_exclusive)),
        &suffix,
    );
    Ok(Some(Range {
        minimum,
        maximum,
        minimum_exclusive,
        maximum_exclusive,
        scale,
        unit,
        written,
        per,
        decimals,
    }))
}

/// The `minimum_exclusive` and `maximum_exclusive` flags of a row, each
/// only beside its bound.
fn exclusive_flags(
    row: Row<'_>,
    minimum: bool,
    maximum: bool,
) -> Result<(bool, bool), Unavailable> {
    let flag = |column: &str, bound: bool| match row.boolean(column)? {
        Some(_) if !bound => Err(invalid(format!("`{column}` needs its bound"))),
        other => Ok(other.unwrap_or(false)),
    };
    Ok((
        flag("minimum_exclusive", minimum)?,
        flag("maximum_exclusive", maximum)?,
    ))
}

/// Bounds as a reviewer reads them.
fn written_bounds(
    minimum: Option<(String, bool)>,
    maximum: Option<(String, bool)>,
    suffix: &str,
) -> String {
    let low = |(value, exclusive): (String, bool)| {
        if exclusive {
            format!("more than {value}{suffix}")
        } else {
            format!("at least {value}{suffix}")
        }
    };
    let high = |(value, exclusive): (String, bool)| {
        if exclusive {
            format!("less than {value}{suffix}")
        } else {
            format!("at most {value}{suffix}")
        }
    };
    match (minimum, maximum) {
        (Some((low, false)), Some((high, false))) => format!("between {low} and {high}{suffix}"),
        (Some(minimum), Some(maximum)) => format!("{} and {}", low(minimum), high(maximum)),
        (Some(minimum), None) => low(minimum),
        (None, Some(maximum)) => high(maximum),
        (None, None) => unreachable!("a range has a bound"),
    }
}

/// A row's date range, if it declares a date or date-time bound.
fn parse_dates(row: Row<'_>) -> Result<Option<Dates>, Unavailable> {
    let bound = |date: &str, date_time: &str| -> Result<Option<PropertyValue>, Unavailable> {
        match (row.temporal(date)?, row.temporal(date_time)?) {
            (Some(_), Some(_)) => Err(invalid(format!(
                "`{date}` and `{date_time}` are one bound; state one"
            ))),
            (one, other) => Ok(one.or(other)),
        }
    };
    let minimum = bound("minimum_date", "minimum_date_time")?;
    let maximum = bound("maximum_date", "maximum_date_time")?;
    let precision = match row.text("precision")? {
        None => None,
        Some("day") => Some(TemporalPrecision::Day),
        Some(other) => {
            return Err(invalid(format!(
                "precision `{other}` is unsupported; the only precision is `day`"
            )));
        }
    };
    if minimum.is_none() && maximum.is_none() {
        if precision.is_some() {
            return Err(invalid("`precision` needs a date bound"));
        }
        return Ok(None);
    }
    let (minimum_exclusive, maximum_exclusive) =
        exclusive_flags(row, minimum.is_some(), maximum.is_some())?;
    if let (Some(low), Some(high)) = (&minimum, &maximum) {
        match temporal_order(low, high, precision) {
            Some(Ok(Ordering::Greater)) => return Err(invalid("minimum exceeds maximum")),
            Some(Ok(Ordering::Equal)) if minimum_exclusive || maximum_exclusive => {
                return Err(invalid(
                    "an exclusive range between equal bounds holds no value",
                ));
            }
            Some(Err(why)) => return Err(invalid(why)),
            _ => {}
        }
    }
    let written = written_bounds(
        minimum
            .as_ref()
            .map(|low| (display(Some(low)), minimum_exclusive)),
        maximum
            .as_ref()
            .map(|high| (display(Some(high)), maximum_exclusive)),
        "",
    );
    Ok(Some(Dates {
        minimum,
        maximum,
        minimum_exclusive,
        maximum_exclusive,
        precision,
        written,
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

/// Grouped findings: (row, category heading, result, property, value key).
type GroupKey = (usize, String, &'static str, String, String);

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
        let (checked, set) = match &row.target {
            Target::Exact { set, name } => {
                let property = PropertyRef {
                    set: set.as_deref(),
                    name,
                };
                (
                    check_row(context, declared, subject, row, property)
                        .map(|failure| failure.map(|failure| (property.to_string(), failure))),
                    set.as_deref().map(|set| (NameSpec::Exact(set), set)),
                )
            }
            Target::Matched {
                set,
                property,
                shown,
            } => (
                check_matched_row(context, declared, subject, row, (set, property, shown)),
                Some((set.spec(), set.shown())).filter(|(set, _)| !matches!(set, NameSpec::Any)),
            ),
        };
        match checked {
            Ok(None) => {}
            Ok(Some((property, failure))) => {
                let (property, failure) = missing_set(context, subject, set, property, failure);
                failures.push((index, property, failure));
            }
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
        let shown = failure
            .shown
            .clone()
            .unwrap_or_else(|| display(failure.value.as_ref()));
        match groups.as_deref_mut() {
            Some(groups) => {
                let key = failure.value.as_ref().map_or_else(
                    || "absent".to_owned(),
                    |value| value_key(value, false, true),
                );
                let group = groups
                    .entry((
                        index,
                        category.clone(),
                        failure.result,
                        property.clone(),
                        key,
                    ))
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
    /// What the message says was found, when not the value (a set).
    shown: Option<String>,
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
const MISSING_SET: &str = "missing property set";
const FORBIDDEN_SET: &str = "forbidden property set present";
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
            shown: None,
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
            shown: None,
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
    let (limits_below, limits_above) = match (&condition.range, &condition.dates) {
        (Some(range), _) => (range.minimum.is_some(), range.maximum.is_some()),
        (None, Some(dates)) => (dates.minimum.is_some(), dates.maximum.is_some()),
        (None, None) => (false, false),
    };
    if let (PropertyValue::Bounded { lower, upper, .. }, true) = (value, every)
        && ((limits_below && lower.is_none()) || (limits_above && upper.is_none()))
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
        Per::MeasuredVolume => {
            let volume = body_volume(context, &subject.id)?;
            evidence.push(volume.evidence().clone());
            let interval = volume.volume();
            return positive(interval.lower_cubic_metres(), interval.upper_cubic_metres());
        }
        Per::MeasuredFaceArea => {
            let area = face_area(context, &subject.id)?;
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
    if let Some(dates) = &condition.dates {
        return date_holds(dates, value);
    }
    match &condition.range {
        Some(range) => range_holds(range, value, divisor),
        None => Holds::Yes,
    }
}

/// Whether one number or quantity, divided by `divisor`, lies within a row's
/// numeric range.
fn range_holds(range: &Range, value: &PropertyValue, divisor: Option<Divisor>) -> Holds {
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
    let bounds = |scale: f64| {
        (
            range.minimum.map(|value| Bound {
                value: value * scale,
                exclusive: range.minimum_exclusive,
            }),
            range.maximum.map(|value| Bound {
                value: value * scale,
                exclusive: range.maximum_exclusive,
            }),
        )
    };
    let verdict = match range.decimals {
        // Bounds in canonical SI units, so an exact value is not rescaled.
        None => {
            let (minimum, maximum) = bounds(range.scale);
            judge_bounds(lower, upper, minimum, maximum)
        }
        // Rounding reads the value in the row's unit, where its bounds are.
        Some(decimals) => {
            let (minimum, maximum) = bounds(1.0);
            judge_bounds(
                round_decimal(lower / range.scale, decimals),
                round_decimal(upper / range.scale, decimals),
                minimum,
                maximum,
            )
        }
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

/// Whether one date or date-time lies within a row's date range.
fn date_holds(dates: &Dates, value: &PropertyValue) -> Holds {
    let within = |bound: Option<&PropertyValue>, exclusive: bool, outside: Ordering| {
        let Some(bound) = bound else {
            return Holds::Yes;
        };
        match temporal_order(value, bound, dates.precision) {
            None => Holds::Undecided(format!(
                "{} is not a date or a date-time",
                display(Some(value))
            )),
            Some(Err(why)) => Holds::Undecided(why),
            Some(Ok(order)) if order == outside || (exclusive && order == Ordering::Equal) => {
                Holds::No(None)
            }
            Some(Ok(_)) => Holds::Yes,
        }
    };
    within(
        dates.minimum.as_ref(),
        dates.minimum_exclusive,
        Ordering::Less,
    )
    .and(within(
        dates.maximum.as_ref(),
        dates.maximum_exclusive,
        Ordering::Greater,
    ))
}

/// Refines a missing property to a missing set when the row names a set and
/// the object has no property in it at all.
///
/// The finding stands either way; a set the source cannot enumerate keeps
/// the plain `missing property`.
fn missing_set(
    context: &RuleContext<'_>,
    subject: &Object,
    set: Option<(NameSpec<'_>, &str)>,
    property: String,
    mut failure: Failure,
) -> (String, Failure) {
    let Some((set, shown)) = set.filter(|_| failure.result == MISSING_PROPERTY) else {
        return (property, failure);
    };
    match enumerate(context, subject, set, NameSpec::Any) {
        // A set the object carries without members is present, not missing.
        Ok(enumeration)
            if enumeration.properties().is_empty() && enumeration.empty_sets().is_empty() =>
        {
            failure.result = MISSING_SET;
            failure.evidence.push(enumeration.evidence().clone());
            (shown.to_owned(), failure)
        }
        _ => (property, failure),
    }
}

/// The properties a row matched on one object, with the row's name as
/// written for failures about none of them.
struct Matches<'e> {
    enumeration: &'e PropertyEnumeration,
    shown: &'e str,
}

/// A failure with the property it names.
type Named = Option<(String, Failure)>;

impl Matches<'_> {
    fn properties(&self) -> &[Property] {
        self.enumeration.properties()
    }

    /// A failure about `found`, or about the row's name when `None`.
    fn failure(
        &self,
        result: &'static str,
        found: Option<&Property>,
        detail: &str,
    ) -> (String, Failure) {
        let mut evidence = vec![self.enumeration.evidence().clone()];
        if let Some(found) = found {
            evidence.extend(found.evidence.iter().cloned());
        }
        (
            found.map_or_else(
                || self.shown.to_owned(),
                |found| format!("{}.{}", found.property_set, found.name),
            ),
            Failure {
                shown: None,
                result,
                value: found.map(|found| found.value.clone()),
                quotient: None,
                detail: detail.to_owned(),
                evidence,
            },
        )
    }
}

fn is_empty(found: &Property) -> bool {
    undefined(Some(&found.value))
}

/// One row naming its set or property by pattern, or a set alone, on one
/// object: every matched property must meet an included statement and none
/// may meet an excluded one. The first failing property is reported, in
/// set and name order.
fn check_matched_row(
    context: &RuleContext<'_>,
    declared: &Declared<'_>,
    subject: &Object,
    row: &RequirementRow,
    (set, property, shown): (&NameSel, &NameSel, &str),
) -> Result<Named, Unavailable> {
    let enumeration = enumerate(context, subject, set.spec(), property.spec())?;
    let matched = Matches {
        enumeration: &enumeration,
        shown,
    };
    let include = row.state == State::Include;
    let set_only = matches!(property, NameSel::Any);
    // A statement that needs a match needs one in every set the row names.
    let needs_match = include
        && match row.statement {
            Statement::Presence(presence) => presence != Presence::Undefined,
            Statement::Value { optional } => !optional,
        };
    if needs_match && !set_only && !matches!(set, NameSel::Any) && !matched.properties().is_empty()
    {
        let (missing, evidence) = sets_without_match(context, subject, set.spec(), &enumeration)?;
        if let Some(missing) = missing.first() {
            let (_, mut failure) = matched.failure(MISSING_PROPERTY, None, "");
            failure.evidence.push(evidence);
            return Ok(Some((format!("{missing}.{}", property.shown()), failure)));
        }
    }
    match row.statement {
        Statement::Presence(presence) if set_only => Ok(set_presence(&matched, include, presence)),
        Statement::Presence(presence) => Ok(matched_presence(&matched, include, presence)),
        // An optional set-only row states nothing.
        Statement::Value { .. } if set_only => Ok(None),
        Statement::Value { optional } => {
            matched_values(context, declared, subject, row, &matched, optional)
        }
    }
}

/// A set-only row: the set is present when it holds a property.
fn set_presence(matched: &Matches<'_>, include: bool, presence: Presence) -> Named {
    let present = !matched.properties().is_empty();
    match (include, presence, present) {
        (true, Presence::Defined, false) | (false, Presence::Undefined, false) => {
            Some(matched.failure(MISSING_SET, None, ""))
        }
        (true, Presence::Undefined, true) | (false, Presence::Defined, true) => {
            let (label, mut failure) = matched.failure(FORBIDDEN_SET, None, "");
            let count = matched.properties().len();
            failure.shown = Some(format!(
                "present with {count} propert{}",
                if count == 1 { "y" } else { "ies" }
            ));
            Some((label, failure))
        }
        _ => None,
    }
}

/// A presence statement over every matched property: an included one holds
/// for each and needs one to match, an excluded one for none.
fn matched_presence(matched: &Matches<'_>, include: bool, presence: Presence) -> Named {
    let found = matched.properties();
    if found.is_empty() {
        let missing = if include {
            presence != Presence::Undefined
        } else {
            presence == Presence::Undefined
        };
        return if missing {
            Some(matched.failure(MISSING_PROPERTY, None, ""))
        } else {
            None
        };
    }
    let first_empty = found.iter().find(|found| is_empty(found));
    let first_filled = found.iter().find(|found| !is_empty(found));
    match (include, presence) {
        (true, Presence::Undefined) | (false, Presence::Defined) => {
            Some(matched.failure(FORBIDDEN_PROPERTY, found.first(), ""))
        }
        (true, Presence::NotEmpty) | (false, Presence::Empty) => {
            first_empty.map(|found| matched.failure(MISSING_VALUE, Some(found), ""))
        }
        (true, Presence::Empty) => {
            first_filled.map(|found| matched.failure(WRONG_VALUE, Some(found), "; required empty"))
        }
        (false, Presence::NotEmpty) => first_filled
            .map(|found| matched.failure(FORBIDDEN_VALUE, Some(found), ", which is not empty")),
        (true, Presence::Defined) | (false, Presence::Undefined) => None,
    }
}

/// Value conditions over every matched property: an included row needs one
/// to match and each to meet them, an excluded row fails on the first that
/// meets them. A property the conditions cannot judge leaves the row open
/// unless another decides it.
fn matched_values(
    context: &RuleContext<'_>,
    declared: &Declared<'_>,
    subject: &Object,
    row: &RequirementRow,
    matched: &Matches<'_>,
    optional: bool,
) -> Result<Named, Unavailable> {
    let include = row.state == State::Include;
    if matched.properties().is_empty() {
        return Ok(if include && !optional {
            Some(matched.failure(MISSING_PROPERTY, None, ""))
        } else {
            None
        });
    }
    let condition = &row.condition;
    let mut divided = Vec::new();
    let divisor = match condition.range.as_ref().and_then(|range| range.per) {
        Some(per) => Some(measure_divisor(
            context,
            declared,
            subject,
            per,
            &mut divided,
        )?),
        None => None,
    };
    let mut open = None;
    for found in matched.properties() {
        if is_empty(found) {
            if include && !optional {
                return Ok(Some(matched.failure(MISSING_VALUE, Some(found), "")));
            }
            continue;
        }
        let judged = match meets(condition, &found.value, divisor, include) {
            Holds::No(quotient) if include => {
                let (label, mut failure) = matched.failure(
                    WRONG_VALUE,
                    Some(found),
                    &format!("; required {}", condition.describe()),
                );
                failure.quotient = quotient;
                Some((label, failure))
            }
            Holds::Yes if !include => Some(matched.failure(
                FORBIDDEN_VALUE,
                Some(found),
                &format!(", which is {}", condition.describe()),
            )),
            Holds::Undecided(why) => {
                open.get_or_insert(why);
                None
            }
            Holds::Yes | Holds::No(_) => None,
        };
        if let Some((label, mut failure)) = judged {
            failure.evidence.extend(divided);
            return Ok(Some((label, failure)));
        }
    }
    match open {
        Some(why) => Err((NotEvaluatedReason::IncompleteEvidence, why)),
        None => Ok(None),
    }
}
