//! Limits looked up in a keyed table: the applicable row is chosen by key
//! values read from the object or from objects related to it.

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};

use crate::plan_area::{Verdict, footprint, judge, shown};
use crate::selection::select_objects;
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, exact_f64, finding, invalid, resolve,
    undefined,
};

/// How many keys a table may be keyed by.
const KEYS: usize = 4;

const KEY_COLUMNS: [&str; KEYS] = ["key_1", "key_2", "key_3", "key_4"];
const KEY_PATHS: [&str; KEYS] = ["key_1_path", "key_2_path", "key_3_path", "key_4_path"];

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("key_1", ColumnKind::TextPattern),
    TableColumn::optional("key_2", ColumnKind::TextPattern),
    TableColumn::optional("key_3", ColumnKind::TextPattern),
    TableColumn::optional("key_4", ColumnKind::TextPattern),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
];

/// Where one key's value is read: a property of the object, or of the
/// objects a relationship path reaches from it.
struct KeySource<'a> {
    property: PropertyRef<'a>,
    path: Option<Traversal<'a>>,
}

impl KeySource<'_> {
    fn describe(&self) -> String {
        match &self.path {
            None => self.property.to_string(),
            Some(path) => format!("{} (via {})", self.property, path.relationship),
        }
    }
}

/// One row of the limit table, its key patterns compiled.
struct Limit {
    keys: [Option<TextPattern>; KEYS],
    minimum: Option<f64>,
    maximum: Option<f64>,
}

/// The quantity each row limits.
enum Quantity<'a> {
    PlanArea,
    Property(PropertyRef<'a>),
}

/// A key value as rows match it, or why it is unknown.
enum Key {
    Known(String),
    Unknown(String),
}

/// Checks a quantity of each object against the limits of the single row of
/// a keyed table that applies to it.
///
/// Each of up to four keys `key_1` … `key_4` is a property reference, read
/// from the object itself or, with `key_<n>_path`, from the objects that
/// relationship path reaches from it (the building's fire class, the storey's
/// sprinkler flag). A key's value is matched as text: a string as stated, a
/// boolean as `true` or `false`, an integer in decimal. Each row of `limits`
/// gives a text pattern per key (a blank cell accepts any value) and an
/// optional `minimum` and `maximum`. The single most specific matching row
/// applies; a row without bounds applies no limit.
///
/// `quantity` names what is limited: `plan-area`, the object's measured
/// footprint in square metres, or `property`, the number or quantity stated
/// by `quantity_property`, in canonical SI units.
///
/// No matching row is a "no limit defined" finding. A key that is absent,
/// null, blank, of another type, reached on no object or on objects that
/// disagree is unknown; when a row that tests it could apply, the object is
/// not evaluated, and so it is when rows tie for most specific. A measured
/// area straddling a bound is not evaluated.
pub struct KeyedLimit;

impl RuleCapability for KeyedLimit {
    fn id(&self) -> &'static str {
        "axioval:capability.keyed-limit"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("limits", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::required("quantity", ParameterType::String),
            ParameterDescriptor::optional("quantity_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ];
        for (index, (key, path)) in KEY_COLUMNS.iter().zip(KEY_PATHS).enumerate() {
            parameters.push(if index == 0 {
                ParameterDescriptor::required(*key, ParameterType::PropertyReference)
            } else {
                ParameterDescriptor::optional(*key, ParameterType::PropertyReference)
            });
            parameters.push(ParameterDescriptor::optional(
                path,
                ParameterType::StringList,
            ));
        }
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (keys, limits, quantity) = match parse(&Parameters(rule)) {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("keyed-limit: {message}"),
                );
            }
        };
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        for subject in subjects {
            match check(context, rule, &keys, &limits, &quantity, subject) {
                Ok(Some(found)) => evaluation.push_finding(found),
                Ok(None) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

type Parsed<'a> = (Vec<Option<KeySource<'a>>>, Vec<Limit>, Quantity<'a>);

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Parsed<'a>, Unavailable> {
    let mut keys = Vec::with_capacity(KEYS);
    for (key, path) in KEY_COLUMNS.iter().zip(KEY_PATHS) {
        let property = parameters.property(key)?;
        let path = parameters.strings(path)?;
        keys.push(match (property, path) {
            (Some(property), path) => Some(KeySource {
                property,
                path: path.map(Traversal::path).transpose()?,
            }),
            (None, Some(_)) => {
                return Err(invalid(format!("`{key}_path` is declared without `{key}`")));
            }
            (None, None) => None,
        });
    }
    if keys[0].is_none() {
        return Err(invalid("parameter `key_1` is required"));
    }
    let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
    let rows = parameters
        .table("limits")?
        .ok_or_else(|| invalid("parameter `limits` is required"))?;
    let mut limits = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let mut patterns: [Option<TextPattern>; KEYS] = Default::default();
        for (slot, column) in KEY_COLUMNS.iter().enumerate() {
            patterns[slot] = row.pattern(column, case_sensitive)?;
            if patterns[slot].is_some() && keys[slot].is_none() {
                return Err(invalid(format!(
                    "limit row {index} keys `{column}`, which the rule does not declare"
                )));
            }
        }
        let minimum = row.number("minimum")?;
        let maximum = row.number("maximum")?;
        if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
            return Err(invalid(format!(
                "limit row {index}: minimum exceeds maximum"
            )));
        }
        limits.push(Limit {
            keys: patterns,
            minimum,
            maximum,
        });
    }
    let property = parameters.property("quantity_property")?;
    let quantity = match (parameters.required_string("quantity")?, property) {
        ("plan-area", None) => Quantity::PlanArea,
        ("property", Some(property)) => Quantity::Property(property),
        ("property", None) => {
            return Err(invalid("`quantity` `property` needs `quantity_property`"));
        }
        ("plan-area", Some(_)) => {
            return Err(invalid("`quantity_property` applies only to `property`"));
        }
        (other, _) => return Err(invalid(format!("quantity `{other}` is unsupported"))),
    };
    Ok((keys, limits, quantity))
}

/// The key values of one object, with the evidence and the objects they
/// were read from.
struct Keys {
    values: Vec<Option<Key>>,
    evidence: Vec<Evidence>,
    sources: Vec<ObjectId>,
}

impl Keys {
    fn read(
        context: &RuleContext<'_>,
        declared: &[Option<KeySource<'_>>],
        object: &Object,
    ) -> Result<Self, Unavailable> {
        let mut keys = Self {
            values: Vec::with_capacity(declared.len()),
            evidence: Vec::new(),
            sources: Vec::new(),
        };
        for source in declared {
            let value = match source {
                Some(source) => Some(keys.value(context, source, object)?),
                None => None,
            };
            keys.values.push(value);
        }
        keys.sources.sort();
        keys.sources.dedup();
        Ok(keys)
    }

    fn value(
        &mut self,
        context: &RuleContext<'_>,
        source: &KeySource<'_>,
        object: &Object,
    ) -> Result<Key, Unavailable> {
        let holders = match &source.path {
            None => vec![object.id.clone()],
            Some(path) => {
                let everything: Vec<&Object> = context.project.objects().collect();
                let (reached, cited) = path.related(context, &object.id, &everything)?;
                self.evidence.extend(cited);
                if reached.is_empty() {
                    return Ok(Key::Unknown(format!(
                        "{} reaches no object",
                        path.relationship
                    )));
                }
                reached
            }
        };
        let mut found: Option<(String, ObjectId)> = None;
        for holder in holders {
            let target = context
                .project
                .object(&holder)
                .ok_or_else(|| invalid(format!("{holder} is not in the project")))?;
            let resolved = resolve(context, target, source.property)?;
            self.evidence.extend(resolved.evidence());
            if holder != object.id {
                self.sources.push(holder.clone());
            }
            let text = match resolved.value() {
                value if undefined(value) => {
                    return Ok(Key::Unknown(format!(
                        "{} of {holder} is {}",
                        source.property,
                        display(value)
                    )));
                }
                Some(PropertyValue::String(text)) => text.clone(),
                Some(PropertyValue::Boolean(value)) => value.to_string(),
                Some(PropertyValue::Integer(value)) => value.to_string(),
                other => {
                    return Ok(Key::Unknown(format!(
                        "{} of {holder} is {}, not text, a boolean or an integer",
                        source.property,
                        display(other)
                    )));
                }
            };
            match &found {
                Some((held, first)) if *held != text => {
                    return Ok(Key::Unknown(format!(
                        "{} differs between {first} (`{held}`) and {holder} (`{text}`)",
                        source.property
                    )));
                }
                Some(_) => {}
                None => found = Some((text, holder)),
            }
        }
        Ok(found.map_or_else(
            || Key::Unknown(format!("{} has no value", source.property)),
            |(text, _)| Key::Known(text),
        ))
    }

    /// Whether `limit` applies to these keys, weighted by specificity.
    fn test(&self, limit: &Limit) -> RowTest {
        self.values
            .iter()
            .zip(&limit.keys)
            .fold(RowTest::Match(0), |outcome, (value, pattern)| {
                outcome.and(match (pattern, value) {
                    (None, _) => RowTest::Match(0),
                    (Some(pattern), Some(Key::Known(text))) => pattern.test(text),
                    (Some(_), Some(Key::Unknown(_)) | None) => RowTest::Undecided,
                })
            })
    }

    /// The keys as a reviewer reads them: each property and its value.
    fn describe(&self, declared: &[Option<KeySource<'_>>]) -> String {
        declared
            .iter()
            .zip(&self.values)
            .filter_map(|(source, value)| {
                let source = source.as_ref()?;
                Some(match value {
                    Some(Key::Known(text)) => format!("{} `{text}`", source.describe()),
                    _ => format!("{} unknown", source.describe()),
                })
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn unknown(&self) -> Vec<&str> {
        self.values
            .iter()
            .filter_map(|value| match value {
                Some(Key::Unknown(why)) => Some(why.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// A measured or stated quantity as an interval, its unit and evidence.
struct Measured {
    lower: f64,
    upper: f64,
    unit: String,
    what: String,
    evidence: Vec<Evidence>,
}

fn measure(
    context: &RuleContext<'_>,
    quantity: &Quantity<'_>,
    object: &Object,
) -> Result<Measured, Unavailable> {
    match quantity {
        Quantity::PlanArea => {
            let area = footprint(context, &object.id)?;
            Ok(Measured {
                lower: area.lower_square_metres(),
                upper: area.upper_square_metres(),
                unit: " m²".into(),
                what: "plan area".into(),
                evidence: vec![area.evidence().clone()],
            })
        }
        Quantity::Property(property) => {
            let resolved = resolve(context, object, *property)?;
            let (value, unit) = match resolved.value() {
                Some(PropertyValue::Quantity { value, dimension }) if value.is_finite() => {
                    (*value, format!(" {}", dimension.unit_symbol()))
                }
                Some(PropertyValue::Decimal(value)) if value.is_finite() => (*value, String::new()),
                Some(PropertyValue::Integer(value)) => (
                    exact_f64(*value).ok_or_else(|| {
                        (
                            NotEvaluatedReason::InvalidEvidence,
                            format!("{property} {value} cannot be compared exactly"),
                        )
                    })?,
                    String::new(),
                ),
                other => {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "{property} states no number or quantity ({})",
                            display(other)
                        ),
                    ));
                }
            };
            Ok(Measured {
                lower: value,
                upper: value,
                unit,
                what: property.to_string(),
                evidence: resolved.evidence(),
            })
        }
    }
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &[Option<KeySource<'_>>],
    limits: &[Limit],
    quantity: &Quantity<'_>,
    subject: &Object,
) -> Result<Option<axioval_ir::Finding>, Unavailable> {
    let keys = Keys::read(context, declared, subject)?;
    let (index, limit) =
        match match_rows(limits, RowSelection::MostSpecific, |limit| keys.test(limit)) {
            Matched::Rows(rows) => match rows.first() {
                Some(&(index, limit)) => (index, limit),
                None => {
                    return Ok(Some(finding(
                        rule,
                        &subject.id,
                        format!("no limit defined for {}", keys.describe(declared)),
                        keys.evidence,
                        keys.sources,
                    )));
                }
            },
            Matched::Undecided => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the applicable limit row cannot be decided: {}",
                        keys.unknown().join("; ")
                    ),
                ));
            }
            Matched::Ambiguous(rows) => {
                return Err(invalid(format!(
                    "limit rows {} apply equally to {}",
                    rows.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    keys.describe(declared)
                )));
            }
        };
    if limit.minimum.is_none() && limit.maximum.is_none() {
        return Ok(None);
    }
    let measured = measure(context, quantity, subject)?;
    let unit = &measured.unit;
    match judge(measured.lower, measured.upper, limit.minimum, limit.maximum) {
        Verdict::Pass => Ok(None),
        Verdict::Fail(bound) => {
            let described = keys.describe(declared);
            let mut evidence = keys.evidence;
            evidence.extend(measured.evidence);
            Ok(Some(finding(
                rule,
                &subject.id,
                format!(
                    "{} is {}{unit}; required {bound}{unit} (limit row {index}: {described})",
                    measured.what,
                    shown(measured.lower, measured.upper),
                ),
                evidence,
                keys.sources,
            )))
        }
        Verdict::Undecided(bound) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} is {}{unit}, which straddles the bound {bound}{unit} (limit row {index})",
                measured.what,
                shown(measured.lower, measured.upper),
            ),
        )),
    }
}
