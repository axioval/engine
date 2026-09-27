//! Tables of property requirements: which properties an object must, may or
//! must not carry, and the values they may hold.

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, PropertyValue, QuantityDimension};

use crate::plan_area::{Verdict, footprint, judge, shown};
use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::table::{Matched, Row, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    Parameters, PropertyRef, Unavailable, display, exact_f64, finding, invalid, resolve,
    si_quantity, undefined,
};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("applies_to", ColumnKind::Selector),
    TableColumn::optional("property_set", ColumnKind::TextPattern),
    TableColumn::optional("property", ColumnKind::TextPattern),
    TableColumn::required("requirement", ColumnKind::String),
    TableColumn::optional("value_like", ColumnKind::TextPattern),
    TableColumn::optional("one_of", ColumnKind::String),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
    TableColumn::optional("unit", ColumnKind::String),
    TableColumn::optional("per", ColumnKind::String),
];

/// Checks each selected object against every row of a `requirements` table
/// that applies to it.
///
/// A row names a property by `property_set` and `property`, states whether
/// it is `required`, `optional` or `forbidden`, and may constrain its value:
/// `value_like` (a whole-value wildcard pattern), `one_of` (values separated
/// by `|`, a backslash escaping the next character) and a numeric range
/// `minimum`/`maximum` in `unit`, optionally `per` the object's
/// `measured-area` (plan footprint), `stated-area` (`area_property`) or
/// `stated-volume` (`volume_property`). `applies_to` restricts a row to the
/// objects a selector picks, such as one exact class or a class with its
/// subtypes; a blank cell applies the row to every selected object.
///
/// Each row yields at most one finding, whose message begins with its
/// result: `missing property`, `missing value`, `forbidden property
/// present`, `forbidden value` or `wrong value`.
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
            if let Target::Unsupported(why) = &requirement.target {
                evaluation.push_not_evaluated(
                    NotEvaluatedReason::MissingService,
                    format!("property-requirements row {index}: {why}"),
                );
            }
        }
        for subject in subjects {
            check(context, rule, &declared, subject, &mut evaluation);
        }
        evaluation
    }
}

/// Whether a row's property must, may or must not be present.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Requirement {
    Required,
    Optional,
    Forbidden,
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

/// A numeric range: bounds in canonical SI units of `dimension`, or unit-free.
struct Range {
    minimum: Option<f64>,
    maximum: Option<f64>,
    /// The declared unit as written and its dimension; `None` for a number.
    unit: Option<(String, QuantityDimension)>,
    /// The declared bounds as written, for messages.
    written: String,
    per: Option<Per>,
}

/// The conditions a row puts on a value; all must hold.
#[derive(Default)]
struct Condition {
    like: Option<(String, TextPattern)>,
    one_of: Vec<String>,
    range: Option<Range>,
    case_sensitive: bool,
}

impl Condition {
    fn is_empty(&self) -> bool {
        self.like.is_none() && self.one_of.is_empty() && self.range.is_none()
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some((pattern, _)) = &self.like {
            parts.push(format!("like `{pattern}`"));
        }
        if !self.one_of.is_empty() {
            parts.push(format!(
                "one of {}",
                self.one_of
                    .iter()
                    .map(|value| format!("`{value}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(range) = &self.range {
            let mut text = range.written.clone();
            if let Some(per) = range.per {
                text.push(' ');
                text.push_str(per.describe());
            }
            parts.push(text);
        }
        parts.join(" and ")
    }
}

/// One row of the requirements table.
struct RequirementRow {
    applies_to: Option<Selector>,
    target: Target,
    requirement: Requirement,
    condition: Condition,
}

struct Declared<'a> {
    rows: Vec<RequirementRow>,
    area_property: Option<PropertyRef<'a>>,
    volume_property: Option<PropertyRef<'a>>,
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
    })
}

fn parse_row(
    row: Row<'_>,
    case_sensitive: bool,
    area_property: Option<PropertyRef<'_>>,
    volume_property: Option<PropertyRef<'_>>,
) -> Result<RequirementRow, Unavailable> {
    let requirement = match row.text("requirement")? {
        Some("required") => Requirement::Required,
        Some("optional") => Requirement::Optional,
        Some("forbidden") => Requirement::Forbidden,
        Some(other) => {
            return Err(invalid(format!(
                "requirement `{other}` is not `required`, `optional` or `forbidden`"
            )));
        }
        None => return Err(invalid("column `requirement` is required")),
    };
    let set = row.text("property_set")?;
    let name = row.text("property")?;
    let condition = parse_condition(row, case_sensitive, area_property, volume_property)?;
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
        requirement,
        condition,
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
        Some(values) => split_values(values)?,
        None => Vec::new(),
    };
    let minimum = row.number("minimum")?;
    let maximum = row.number("maximum")?;
    let unit = row.text("unit")?;
    let per = row.text("per")?;
    let range = if minimum.is_none() && maximum.is_none() {
        if unit.is_some() || per.is_some() {
            return Err(invalid("`unit` and `per` need `minimum` or `maximum`"));
        }
        None
    } else {
        if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
            return Err(invalid("minimum exceeds maximum"));
        }
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
        Some(Range {
            minimum: minimum.map(|value| value * scale),
            maximum: maximum.map(|value| value * scale),
            unit,
            written,
            per,
        })
    };
    Ok(Condition {
        like,
        one_of,
        range,
        case_sensitive,
    })
}

/// Splits a `one_of` cell at unescaped `|`.
fn split_values(cell: &str) -> Result<Vec<String>, Unavailable> {
    let mut values = vec![String::new()];
    let mut chars = cell.chars();
    while let Some(c) = chars.next() {
        match c {
            '|' => values.push(String::new()),
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or_else(|| invalid("`one_of` ends with a backslash"))?;
                values.last_mut().expect("never empty").push(escaped);
            }
            other => values.last_mut().expect("never empty").push(other),
        }
    }
    if values.iter().any(String::is_empty) {
        return Err(invalid("`one_of` has an empty value"));
    }
    Ok(values)
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declared<'_>,
    subject: &Object,
    evaluation: &mut CapabilityEvaluation,
) {
    let mut selected = Vec::new();
    let mut undecided = None;
    let matched = match_rows(&declared.rows, RowSelection::All, |row| {
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
            Ok(Some((message, mut evidence))) => {
                evidence.extend(selected.iter().cloned());
                evaluation.push_finding(finding(
                    rule,
                    &subject.id,
                    format!("{message} (requirement row {index})"),
                    evidence,
                    Vec::new(),
                ));
            }
            Err((reason, message)) => evaluation.push_object_not_evaluated(
                subject.id.clone(),
                reason,
                format!("requirement row {index}: {message}"),
            ),
        }
    }
}

/// A finding's message and evidence.
type Outcome = Option<(String, Vec<Evidence>)>;

fn check_row(
    context: &RuleContext<'_>,
    declared: &Declared<'_>,
    subject: &Object,
    row: &RequirementRow,
    property: PropertyRef<'_>,
) -> Result<Outcome, Unavailable> {
    let resolved = resolve(context, subject, property)?;
    let mut evidence = resolved.evidence();
    let condition = &row.condition;
    let Some(value) = resolved.value() else {
        return Ok((row.requirement == Requirement::Required)
            .then(|| (format!("missing property: {property} is absent"), evidence)));
    };
    let shown_value = display(Some(value));
    if row.requirement == Requirement::Forbidden && condition.is_empty() {
        return Ok(Some((
            format!("forbidden property present: {property} is {shown_value}"),
            evidence,
        )));
    }
    if undefined(Some(value)) {
        return Ok((row.requirement == Requirement::Required).then(|| {
            (
                format!("missing value: {property} is {shown_value}"),
                evidence,
            )
        }));
    }
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
    let test = |element: &PropertyValue| holds(condition, element, divisor);
    let elements: Vec<&PropertyValue> = match value {
        PropertyValue::List(elements) => elements.iter().collect(),
        scalar => vec![scalar],
    };
    if row.requirement == Requirement::Forbidden {
        // A forbidden value anywhere in a list is present.
        let mut open = None;
        for element in elements {
            match test(element) {
                Holds::Yes => {
                    return Ok(Some((
                        format!(
                            "forbidden value: {property} is {shown_value}, which is {}",
                            condition.describe()
                        ),
                        evidence,
                    )));
                }
                Holds::No(_) => {}
                Holds::Undecided(why) => {
                    open.get_or_insert(why);
                }
            }
        }
        return match open {
            Some(why) => Err((NotEvaluatedReason::IncompleteEvidence, why)),
            None => Ok(None),
        };
    }
    // A required or optional value holds only when every element does.
    let mut open = None;
    for element in elements {
        match test(element) {
            Holds::Yes => {}
            Holds::No(quotient) => {
                let quotient = quotient.map_or_else(String::new, |text| format!(" ({text})"));
                return Ok(Some((
                    format!(
                        "wrong value: {property} is {shown_value}{quotient}; required {}",
                        condition.describe()
                    ),
                    evidence,
                )));
            }
            Holds::Undecided(why) => {
                open.get_or_insert(why);
            }
        }
    }
    match open {
        Some(why) => Err((NotEvaluatedReason::IncompleteEvidence, why)),
        None => Ok(None),
    }
}

/// Whether one scalar value meets a condition.
enum Holds {
    Yes,
    /// It does not; for a divided range, the quotient that was judged.
    No(Option<String>),
    Undecided(String),
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

fn holds(condition: &Condition, value: &PropertyValue, divisor: Option<Divisor>) -> Holds {
    if condition.like.is_some() || !condition.one_of.is_empty() {
        let text = match value {
            PropertyValue::String(text) => text.clone(),
            PropertyValue::Boolean(value) => value.to_string(),
            PropertyValue::Integer(value) => value.to_string(),
            other => {
                return Holds::Undecided(format!(
                    "{} is not text, a boolean or an integer",
                    display(Some(other))
                ));
            }
        };
        if let Some((_, compiled)) = &condition.like
            && !matches!(compiled.test(&text), RowTest::Match(_))
        {
            return Holds::No(None);
        }
        if !condition.one_of.is_empty() {
            let fold = |text: &str| {
                if condition.case_sensitive {
                    text.to_owned()
                } else {
                    text.to_lowercase()
                }
            };
            let wanted = fold(&text);
            if !condition.one_of.iter().any(|value| fold(value) == wanted) {
                return Holds::No(None);
            }
        }
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
    match judge(lower, upper, range.minimum, range.maximum) {
        Verdict::Pass => Holds::Yes,
        // Only a quotient differs from the value the message shows.
        Verdict::Fail(_) => Holds::No(
            range
                .per
                .is_some()
                .then(|| format!("{}{unit}{per}", shown(lower, upper))),
        ),
        Verdict::Undecided(bound) => Holds::Undecided(format!(
            "{}{unit}{per} straddles the bound {bound} (in canonical SI units)",
            shown(lower, upper)
        )),
    }
}
