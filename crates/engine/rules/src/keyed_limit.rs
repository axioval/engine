//! Limits looked up in a keyed table: the applicable row is chosen by key
//! values read from the object or from objects related to it.

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, Deviation, DoorLeavesError, NotEvaluatedReason,
    ObjectFrameServiceHandle, ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
    TableColumn, VerticalExtent,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue};

use crate::door_swing;
use crate::level_spacing::{extent, extents};
use crate::light_area::{LightArea, length};
use crate::plan_area::{Verdict, deviation, footprint, judge, shown};
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
    /// The object's bottom above the bottom of each object `floor_path`
    /// reaches from it.
    SillHeight(Traversal<'a>),
    /// A door's clear width: stated, or its overall width less a deduction
    /// the rule states.
    ClearWidth(ClearWidth<'a>),
}

/// Where a clear width comes from, in the order the steps are tried.
///
/// The deduction is the rule author's declared approximation of the frame,
/// lining and leaf that narrow the overall width, not a measurement: every
/// width derived with it says so in its message and cites an inexact
/// evidence entry naming the deduction.
struct ClearWidth<'a> {
    /// The clear width the object states, tried first.
    stated: Option<PropertyRef<'a>>,
    /// Whether to derive it next from the door's leaves and lining.
    leaves: bool,
    /// The overall width and the deduction from it, in metres.
    derived: Option<(PropertyRef<'a>, f64)>,
}

impl<'a> ClearWidth<'a> {
    /// The declared steps: at least one, the overall width and its
    /// deduction only together.
    fn declared(
        stated: Option<PropertyRef<'a>>,
        leaves: bool,
        overall: Option<PropertyRef<'a>>,
        deduction: Option<f64>,
    ) -> Result<Self, Unavailable> {
        let derived = match (overall, deduction) {
            (Some(overall), Some(deduction)) => Some((overall, deduction)),
            (None, None) => None,
            _ => {
                return Err(invalid(
                    "`overall_width` and `width_deduction` are declared together",
                ));
            }
        };
        if stated.is_none() && !leaves && derived.is_none() {
            return Err(invalid(
                "`quantity` `clear-width` needs `quantity_property`, \
                 `clear_width_from_leaves`, or `overall_width` with `width_deduction`",
            ));
        }
        Ok(Self {
            stated,
            leaves,
            derived,
        })
    }

    /// The clear width of `object`: the stated one when present, else the
    /// overall width less the deduction. Only an exact absence moves on to
    /// the next step; a stated value that is not a positive length stops the
    /// chain rather than being replaced by an approximation.
    fn measure(&self, context: &RuleContext<'_>, object: &Object) -> Result<Measured, Unavailable> {
        let mut evidence = Vec::new();
        if let Some(stated) = self.stated {
            if let Some(width) = LightArea::length(context, object, stated, &mut evidence)? {
                evidence.push(Self::record(&object.id, "stated", true));
                return Ok(Measured {
                    lower: width,
                    upper: width,
                    unit: " m".into(),
                    what: format!("clear width ({stated})"),
                    evidence,
                });
            }
        }
        if self.leaves
            && let Some(measured) = Self::from_leaves(context, object, &mut evidence)?
        {
            return Ok(measured);
        }
        let Some((overall, deduction)) = self.derived else {
            let mut absent: Vec<String> = self.stated.iter().map(ToString::to_string).collect();
            if self.leaves {
                absent.push("the lining and leaf thicknesses".into());
            }
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} absent and the rule states no deduction to derive it",
                    absent.join(" and ")
                ),
            ));
        };
        let Some(width) = LightArea::length(context, object, overall, &mut evidence)? else {
            let stated = self
                .stated
                .map_or_else(String::new, |stated| format!("{stated} and "));
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{stated}{overall} are absent, so no clear width can be derived"),
            ));
        };
        let (lower, upper) = difference(width, deduction);
        if upper <= 0.0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{overall} {} m less the deduction {} m leaves no clear width",
                    shown(width, width),
                    shown(deduction, deduction)
                ),
            ));
        }
        evidence.push(Self::record(
            &object.id,
            &format!("overall-width-less-deduction;deduction={deduction}"),
            false,
        ));
        Ok(Measured {
            lower,
            upper,
            unit: " m".into(),
            what: format!(
                "clear width ({overall} {} m less the rule's deduction {} m, an approximation)",
                shown(width, width),
                shown(deduction, deduction)
            ),
            evidence,
        })
    }

    /// The clear width of a door whose leaves all swing, from what it
    /// states: the overall width less the lining on both jambs and the
    /// thickness of every hinged leaf standing open in the opening. `None`
    /// (move on) when the source states no leaves, no lining thickness or
    /// a leaf thickness, or when a leaf slides, rolls or is fixed, so that
    /// this derivation does not apply.
    fn from_leaves(
        context: &RuleContext<'_>,
        object: &Object,
        evidence: &mut Vec<Evidence>,
    ) -> Result<Option<Measured>, Unavailable> {
        let Some(frames) = context.services.get::<ObjectFrameServiceHandle>() else {
            return Err((
                NotEvaluatedReason::MissingService,
                "the object-frame service is not registered, so the door's leaves are unknown"
                    .into(),
            ));
        };
        let leaves = match frames.leaves(&object.id) {
            Ok(leaves) => leaves,
            Err(DoorLeavesError::NotStated(_)) => return Ok(None),
            Err(error) => {
                return Err((
                    door_swing::reason(&error),
                    format!("the door's leaves are unknown: {error}"),
                ));
            }
        };
        if leaves
            .leaves()
            .iter()
            .any(|leaf| !leaf.motion().is_hinged())
        {
            return Ok(None);
        }
        let Some(lining) = leaves.lining_thickness_metres() else {
            return Ok(None);
        };
        let mut depth = 0.0;
        for leaf in leaves.hinged() {
            let Some(leaf_depth) = leaf.depth_metres() else {
                return Ok(None);
            };
            depth += leaf_depth;
        }
        let overall = leaves.overall_width_metres();
        let deduction = 2.0 * lining + depth;
        let (lower, upper) = difference(overall, deduction);
        if upper <= 0.0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the overall width {} m less its lining and leaves {} m leaves no clear width",
                    shown(overall, overall),
                    shown(deduction, deduction)
                ),
            ));
        }
        evidence.push(leaves.evidence().clone());
        evidence.push(Self::record(&object.id, "lining-and-leaves", false));
        Ok(Some(Measured {
            lower,
            upper,
            unit: " m".into(),
            what: format!(
                "clear width (overall width {} m less 2 × {} m lining and {} m of open leaf, as \
                 the door states them)",
                shown(overall, overall),
                shown(lining, lining),
                shown(depth, depth)
            ),
            evidence: std::mem::take(evidence),
        }))
    }

    /// The evidence entry recording which step produced a clear width; the
    /// deduction step is an approximation and so never exact.
    fn record(object: &ObjectId, step: &str, exact: bool) -> Evidence {
        let mut evidence = Evidence::exact(
            object.source.clone(),
            format!("axioval:derived.clear-width:{object}:step={step}"),
        );
        evidence.exact = exact;
        evidence
    }
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
/// footprint in square metres, `property`, the number or quantity stated
/// by `quantity_property`, in canonical SI units, or `sill-height`, the
/// object's bottom elevation above the bottom of each object `floor_path`
/// reaches from it (a window's spaces), in metres, or `clear-width`, a
/// door's clear width in metres: the length `quantity_property` states,
/// else, with `clear_width_from_leaves`, the overall width less the lining
/// on both jambs and every open hinged leaf's thickness as the door's
/// leaves state them, else the length `overall_width` states less the
/// rule's `width_deduction`; each step only after an exact absence. The deduction is the rule author's declared
/// approximation of frame and lining; a width derived with it says so and
/// cites an inexact `axioval:derived.clear-width` evidence entry.
///
/// A sill height is judged per reached floor, each against the one row the
/// keys select: a window too high above any one of its spaces' floors is a
/// finding naming that space, and a window between spaces whose floors lie
/// at different elevations is judged against each. Elevations are
/// intervals; a sill height straddling a bound, or a floor that cannot be
/// measured, leaves the window not evaluated unless another floor already
/// fails it.
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

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("limits", ParameterType::Table(COLUMNS)),
            ParameterDescriptor::required("quantity", ParameterType::String),
            ParameterDescriptor::optional("quantity_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("floor_path", ParameterType::StringList),
            ParameterDescriptor::optional("overall_width", ParameterType::PropertyReference),
            ParameterDescriptor::optional("width_deduction", ParameterType::Quantity),
            ParameterDescriptor::optional("clear_width_from_leaves", ParameterType::Boolean),
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
                Ok(Some((found, deviation))) => evaluation.push_finding_deviating(found, deviation),
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
    let floor = parameters.strings("floor_path")?;
    let named = parameters.required_string("quantity")?;
    let overall = parameters.property("overall_width")?;
    let deduction = length(parameters, "width_deduction")?;
    let from_leaves = parameters
        .boolean("clear_width_from_leaves")?
        .unwrap_or(false);
    if named != "clear-width" && (overall.is_some() || deduction.is_some() || from_leaves) {
        return Err(invalid(format!(
            "`overall_width`, `width_deduction` and `clear_width_from_leaves` apply only to \
             `clear-width`, not `{named}`"
        )));
    }
    let quantity = match (named, property, floor) {
        ("clear-width", _, Some(_)) => {
            return Err(invalid(
                "`floor_path` applies only to `sill-height`, not `clear-width`",
            ));
        }
        ("clear-width", stated, None) => Quantity::ClearWidth(ClearWidth::declared(
            stated,
            from_leaves,
            overall,
            deduction,
        )?),
        ("plan-area", None, None) => Quantity::PlanArea,
        ("property", Some(property), None) => Quantity::Property(property),
        ("sill-height", None, Some(path)) => Quantity::SillHeight(Traversal::path(path)?),
        ("property", None, _) => {
            return Err(invalid("`quantity` `property` needs `quantity_property`"));
        }
        ("sill-height", _, None) => {
            return Err(invalid("`quantity` `sill-height` needs `floor_path`"));
        }
        (other @ ("plan-area" | "sill-height"), Some(_), _) => {
            return Err(invalid(format!(
                "`quantity_property` applies only to `property` and `clear-width`, not `{other}`"
            )));
        }
        (other @ ("plan-area" | "property"), _, Some(_)) => {
            return Err(invalid(format!(
                "`floor_path` applies only to `sill-height`, not `{other}`"
            )));
        }
        (other, _, _) => return Err(invalid(format!("quantity `{other}` is unsupported"))),
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
        Quantity::SillHeight(_) => Err(invalid("a sill height is judged per floor")),
        Quantity::ClearWidth(clear) => clear.measure(context, object),
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
) -> Result<Option<Graded>, Unavailable> {
    let keys = Keys::read(context, declared, subject)?;
    let (index, limit) =
        match match_rows(limits, RowSelection::MostSpecific, |limit| keys.test(limit)) {
            Matched::Rows(rows) => match rows.first() {
                Some(&(index, limit)) => (index, limit),
                None => {
                    return Ok(Some((
                        finding(
                            rule,
                            &subject.id,
                            format!("no limit defined for {}", keys.describe(declared)),
                            keys.evidence,
                            keys.sources,
                        ),
                        None,
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
    if let Quantity::SillHeight(floor) = quantity {
        let described = format!("limit row {index}: {}", keys.describe(declared));
        return sill_height(context, rule, floor, subject, limit, &described, keys);
    }
    let measured = measure(context, quantity, subject)?;
    let unit = &measured.unit;
    let verdict = if matches!(quantity, Quantity::ClearWidth(_)) {
        judge_as_displayed(measured.lower, measured.upper, limit.minimum, limit.maximum)
    } else {
        judge(measured.lower, measured.upper, limit.minimum, limit.maximum)
    };
    match verdict {
        Verdict::Pass => Ok(None),
        Verdict::Fail(bound) => {
            let described = keys.describe(declared);
            let mut evidence = keys.evidence;
            evidence.extend(measured.evidence);
            Ok(Some((
                finding(
                    rule,
                    &subject.id,
                    format!(
                        "{} is {}{unit}; required {bound}{unit} (limit row {index}: {described})",
                        measured.what,
                        shown(measured.lower, measured.upper),
                    ),
                    evidence,
                    keys.sources,
                ),
                deviation(measured.lower, measured.upper, limit.minimum, limit.maximum),
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

/// `minuend - subtrahend` as an interval sure to hold the exact difference: the
/// rounded difference, widened by one step where rounding moved it.
fn difference(minuend: f64, subtrahend: f64) -> (f64, f64) {
    let rounded = minuend - subtrahend;
    // Two-sum: the exact difference is `rounded + error`.
    let back = rounded - minuend;
    let error = (minuend - (rounded - back)) + (-subtrahend - back);
    if error > 0.0 {
        (rounded, rounded.next_up())
    } else if error < 0.0 {
        (rounded.next_down(), rounded)
    } else {
        (rounded, rounded)
    }
}

/// Judges an interval of lengths read as the decimals they display: an end
/// within a few units in the last place of a bound meets it, so a 1 m door
/// less a 0.1 m deduction meets a 0.9 m minimum although the binary
/// difference falls a rounding step short of it.
fn judge_as_displayed(
    lower: f64,
    upper: f64,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Verdict {
    let snap = |value: f64| {
        [minimum, maximum]
            .into_iter()
            .flatten()
            .find(|bound| {
                (value - bound).abs() <= 4.0 * f64::EPSILON * value.abs().max(bound.abs())
            })
            .unwrap_or(value)
    };
    judge(snap(lower), snap(upper), minimum, maximum)
}

/// The sill height of `window` above the bottom of `floor`, as an interval.
fn sill_interval(window: &VerticalExtent, floor: &VerticalExtent) -> (f64, f64) {
    let (lower, _) = difference(
        window.bottom().lower_metres(),
        floor.bottom().upper_metres(),
    );
    let (_, upper) = difference(
        window.bottom().upper_metres(),
        floor.bottom().lower_metres(),
    );
    (lower, upper)
}

/// Judges the sill height of `subject` above each floor `path` reaches from
/// it against `limit`. One failing floor is a finding; otherwise a floor that
/// cannot be measured or straddles a bound leaves the subject not evaluated.
fn sill_height(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    path: &Traversal<'_>,
    subject: &Object,
    limit: &Limit,
    described: &str,
    keys: Keys,
) -> Result<Option<Graded>, Unavailable> {
    let service = extents(context)?;
    let window = extent(service, &subject.id)?;
    let everything: Vec<&Object> = context.project.objects().collect();
    let (floors, cited) = path.related(context, &subject.id, &everything)?;
    if floors.is_empty() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{} reaches no floor to measure from", path.relationship),
        ));
    }
    let mut failed = Vec::new();
    let mut worst: Option<Deviation> = None;
    let mut undecided = Vec::new();
    let mut evidence = keys.evidence;
    let mut related = keys.sources;
    for floor in floors {
        let measured = match extent(service, &floor) {
            Ok(measured) => measured,
            Err((_, why)) => {
                undecided.push(format!("the floor of {floor} cannot be measured: {why}"));
                continue;
            }
        };
        let (lower, upper) = sill_interval(&window, &measured);
        let height = shown(lower, upper);
        match judge(lower, upper, limit.minimum, limit.maximum) {
            Verdict::Pass => {}
            Verdict::Fail(bound) => {
                failed.push(format!(
                    "sill height above the floor of {floor} is {height} m; required {bound} m"
                ));
                let missed = deviation(lower, upper, limit.minimum, limit.maximum);
                worst = match (worst, missed) {
                    (Some(worst), Some(missed)) => Some(worst.worst(missed)),
                    (worst, missed) => worst.or(missed),
                };
                evidence.push(measured.evidence().clone());
                related.push(floor);
            }
            Verdict::Undecided(bound) => undecided.push(format!(
                "sill height above the floor of {floor} is {height} m, which straddles the \
                 bound {bound} m"
            )),
        }
    }
    if failed.is_empty() {
        return if undecided.is_empty() {
            Ok(None)
        } else {
            Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} ({described})", undecided.join("; ")),
            ))
        };
    }
    evidence.push(window.evidence().clone());
    evidence.extend(cited);
    related.sort();
    related.dedup();
    Ok(Some((
        finding(
            rule,
            &subject.id,
            format!("{} ({described})", failed.join("; ")),
            evidence,
            related,
        ),
        worst,
    )))
}

/// A finding and how far its value misses the bound, when it has one.
type Graded = (axioval_ir::Finding, Option<Deviation>);

#[cfg(test)]
mod tests {
    use super::difference;

    #[test]
    #[allow(clippy::float_cmp)]
    fn a_rounded_difference_is_widened_to_hold_the_exact_one() {
        // 1.0 - 0.1 rounds up to the nearest double to 0.9.
        let (lower, upper) = difference(1.0, 0.1);
        assert!(lower < upper);
        assert_eq!(upper, 1.0 - 0.1);
        assert_eq!(difference(1.5, 0.5), (1.0, 1.0));
    }
}
