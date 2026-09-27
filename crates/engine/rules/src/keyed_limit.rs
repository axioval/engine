//! Limits looked up in a keyed table: the applicable row is chosen by key
//! values read from the object or from objects related to it.

mod threshold;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, Deviation, DoorLeaf, DoorLeaves,
    DoorLeavesError, LeafMotion, NotEvaluatedReason, ObjectFrameServiceHandle, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn, VerticalExtent,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

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
    /// A door's clear height: stated, or its overall height less its head
    /// lining and threshold.
    ClearHeight(ClearHeight<'a>),
    /// The step from each floor `floor_path` reaches (or a ramp's top near
    /// the door) to the door's bottom and threshold.
    ThresholdStep(threshold::ThresholdStep<'a>),
}

/// Which clear width the door's leaves give.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LeafMode {
    /// The whole passage, every hinged leaf standing open in it.
    Passage,
    /// The widest hinged leaf's own passage.
    WidestLeaf,
}

impl LeafMode {
    fn parse(value: Option<&str>) -> Result<Option<Self>, Unavailable> {
        match value {
            None => Ok(None),
            Some("passage") => Ok(Some(Self::Passage)),
            Some("widest-leaf") => Ok(Some(Self::WidestLeaf)),
            Some(other) => Err(invalid(format!(
                "`clear_width_from_leaves` `{other}` is unsupported; use `passage` or \
                 `widest-leaf`"
            ))),
        }
    }
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
    /// Whether, and how, to derive it next from the door's leaves and
    /// lining.
    leaves: Option<LeafMode>,
    /// The overall width and the deduction from it, in metres.
    derived: Option<(PropertyRef<'a>, f64)>,
}

impl<'a> ClearWidth<'a> {
    /// The declared steps: at least one, the overall width and its
    /// deduction only together.
    fn declared(
        stated: Option<PropertyRef<'a>>,
        leaves: Option<LeafMode>,
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
        if stated.is_none() && leaves.is_none() && derived.is_none() {
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
        if let Some(mode) = self.leaves
            && let Some(measured) = Self::from_leaves(context, object, mode, &mut evidence)?
        {
            return Ok(measured);
        }
        let Some((overall, deduction)) = self.derived else {
            let mut absent: Vec<String> = self.stated.iter().map(ToString::to_string).collect();
            if self.leaves.is_some() {
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
    /// thickness of every hinged leaf standing open in the opening, or with
    /// [`LeafMode::WidestLeaf`] the widest hinged leaf's width less the
    /// lining at each jamb it meets and its own thickness. `None` (move on)
    /// when the source states no leaves, no lining thickness or a leaf
    /// thickness, or when a leaf slides or rolls (for the passage, also when
    /// one is fixed), so that this derivation does not apply.
    fn from_leaves(
        context: &RuleContext<'_>,
        object: &Object,
        mode: LeafMode,
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
        let derived = match mode {
            LeafMode::Passage => passage(&leaves),
            LeafMode::WidestLeaf => widest_leaf(&leaves),
        };
        let Some(derived) = derived else {
            return Ok(None);
        };
        let (lower, upper) = difference(derived.width, derived.deduction);
        if upper <= 0.0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the {} {} m less its lining and leaves {} m leaves no clear width",
                    derived.from,
                    shown(derived.width, derived.width),
                    shown(derived.deduction, derived.deduction)
                ),
            ));
        }
        evidence.push(leaves.evidence().clone());
        evidence.push(Self::record(&object.id, derived.step, false));
        Ok(Some(Measured {
            lower,
            upper,
            unit: " m".into(),
            what: derived.what,
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

/// A clear width derived from a door's leaves: what it is taken from, the
/// deduction, and how a finding words it.
struct FromLeaves {
    from: &'static str,
    width: f64,
    deduction: f64,
    step: &'static str,
    what: String,
}

/// The whole passage: the overall width less the lining on both jambs and
/// every hinged leaf standing open in it; `None` when a leaf does not swing
/// or a thickness is not stated.
fn passage(leaves: &DoorLeaves) -> Option<FromLeaves> {
    if leaves
        .leaves()
        .iter()
        .any(|leaf| !leaf.motion().is_hinged())
    {
        return None;
    }
    let lining = leaves.lining_thickness_metres()?;
    let mut depth = 0.0;
    for leaf in leaves.hinged() {
        depth += leaf.depth_metres()?;
    }
    let overall = leaves.overall_width_metres();
    Some(FromLeaves {
        from: "overall width",
        width: overall,
        deduction: 2.0 * lining + depth,
        step: "lining-and-leaves",
        what: format!(
            "clear width (overall width {} m less 2 × {} m lining and {} m of open leaf, as the \
             door states them)",
            shown(overall, overall),
            shown(lining, lining),
            shown(depth, depth)
        ),
    })
}

/// The widest hinged leaf's passage: its width less the lining at each
/// jamb its closed edge meets (where the door's outermost leaves end) and
/// its own thickness standing open. A fixed leaf narrows nothing but may
/// stand between a leaf and a jamb. `None` when a leaf slides or rolls, no
/// leaf swings, or a thickness it needs is not stated.
fn widest_leaf(leaves: &DoorLeaves) -> Option<FromLeaves> {
    let all = leaves.leaves();
    if all
        .iter()
        .any(|leaf| matches!(leaf.motion(), LeafMotion::Slide(_) | LeafMotion::RollUp))
    {
        return None;
    }
    let axis = all.first()?.along().components();
    let span = |leaf: &DoorLeaf| {
        let (from, to) = leaf.closed_edge();
        let project =
            |point: [f64; 3]| point[0] * axis[0] + point[1] * axis[1] + point[2] * axis[2];
        let (a, b) = (project(from), project(to));
        (a.min(b), a.max(b))
    };
    let (low, high) = all
        .iter()
        .map(span)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), (a, b)| {
            (low.min(a), high.max(b))
        });
    let tolerance = 1e-9 * low.abs().max(high.abs()).max(1.0);
    let mut best: Option<FromLeaves> = None;
    for leaf in leaves.hinged() {
        let (a, b) = span(leaf);
        let jambs =
            u8::from((a - low).abs() <= tolerance) + u8::from((high - b).abs() <= tolerance);
        let lining = if jambs == 0 {
            0.0
        } else {
            leaves.lining_thickness_metres()?
        };
        let depth = leaf.depth_metres()?;
        let width = leaf.width_metres();
        let deduction = f64::from(jambs).mul_add(lining, depth);
        if best
            .as_ref()
            .is_some_and(|best| best.width - best.deduction >= width - deduction)
        {
            continue;
        }
        let jamb_words = match jambs {
            0 => String::new(),
            1 => format!(" {} m lining at one jamb and", shown(lining, lining)),
            _ => format!(" 2 × {} m lining and", shown(lining, lining)),
        };
        best = Some(FromLeaves {
            from: "widest leaf",
            width,
            deduction,
            step: "widest-leaf",
            what: format!(
                "clear width of the widest leaf (leaf {} m less{jamb_words} {} m of open leaf, as \
                 the door states them)",
                shown(width, width),
                shown(depth, depth)
            ),
        });
    }
    best
}

/// A door's clear height: the length `quantity_property` states, else the
/// length `overall_height` states less the head lining and the threshold
/// the door states. A declared thickness the door does not state is
/// unknown, never zero: the clear height is then bounded only from above.
struct ClearHeight<'a> {
    stated: Option<PropertyRef<'a>>,
    overall: Option<PropertyRef<'a>>,
    lining: Option<PropertyRef<'a>>,
    threshold: Option<PropertyRef<'a>>,
}

impl ClearHeight<'_> {
    fn measure(&self, context: &RuleContext<'_>, object: &Object) -> Result<Measured, Unavailable> {
        let mut evidence = Vec::new();
        if let Some(stated) = self.stated
            && let Some(height) = LightArea::length(context, object, stated, &mut evidence)?
        {
            evidence.push(clear_height_record(&object.id, "stated", true));
            return Ok(Measured {
                lower: height,
                upper: height,
                unit: " m".into(),
                what: format!("clear height ({stated})"),
                evidence,
            });
        }
        let stated = self
            .stated
            .map_or_else(String::new, |stated| format!("{stated} and "));
        let Some(overall) = self.overall else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{}is absent and the rule states no overall height to derive it from",
                    stated.trim_end_matches("and ")
                ),
            ));
        };
        let Some(height) = LightArea::length(context, object, overall, &mut evidence)? else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{stated}{overall} are absent, so no clear height can be derived"),
            ));
        };
        let (mut lower, mut upper) = (height, height);
        let mut words = vec![format!("{overall} {} m", shown(height, height))];
        let mut unknown = false;
        for (declared, noun) in [(self.lining, "lining"), (self.threshold, "threshold")] {
            let Some(property) = declared else { continue };
            if let Some(value) = thickness(context, object, property, &mut evidence)? {
                lower = difference(lower, value).0;
                upper = difference(upper, value).1;
                words.push(format!("less the {noun} {} m", shown(value, value)));
            } else {
                unknown = true;
                words.push(format!("less a {noun} {property} does not state"));
            }
        }
        if upper <= 0.0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} leaves no clear height", words.join(" ")),
            ));
        }
        evidence.push(clear_height_record(
            &object.id,
            "overall-height-less-lining-and-threshold",
            false,
        ));
        Ok(Measured {
            lower: if unknown { 0.0 } else { lower.max(0.0) },
            upper,
            unit: " m".into(),
            what: format!("clear height ({})", words.join(" ")),
            evidence,
        })
    }
}

/// The evidence entry recording which step produced a clear height; the
/// derivation takes the lining to run across the head, so it is never
/// exact.
fn clear_height_record(object: &ObjectId, step: &str, exact: bool) -> Evidence {
    let mut evidence = Evidence::exact(
        object.source.clone(),
        format!("axioval:derived.clear-height:{object}:step={step}"),
    );
    evidence.exact = exact;
    evidence
}

/// A thickness `object` states: `None` when absent or null, a non-negative
/// length otherwise; anything else leaves the object not evaluated.
fn thickness(
    context: &RuleContext<'_>,
    object: &Object,
    property: PropertyRef<'_>,
    evidence: &mut Vec<Evidence>,
) -> Result<Option<f64>, Unavailable> {
    let resolved = resolve(context, object, property)?;
    evidence.extend(resolved.evidence());
    match resolved.value() {
        None | Some(PropertyValue::Null) => Ok(None),
        Some(PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }) if value.is_finite() && *value >= 0.0 => Ok(Some(*value)),
        other => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} {property} is {}, not a non-negative length",
                object.id,
                display(other)
            ),
        )),
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
/// else, with `clear_width_from_leaves` `passage`, the overall width less
/// the lining on both jambs and every open hinged leaf's thickness as the
/// door's leaves state them (`widest-leaf`: the widest hinged leaf less the
/// lining at its jambs and its thickness), else the length `overall_width`
/// states less the rule's `width_deduction`; each step only after an exact
/// absence. The deduction is the rule author's declared approximation of
/// frame and lining; a width derived with it says so and cites an inexact
/// `axioval:derived.clear-width` evidence entry.
///
/// `clear-height` is a door's stated clear height, else `overall_height`
/// less the `lining_thickness` and `threshold_thickness` it states, a
/// declared thickness it does not state leaving only an upper bound.
/// `threshold-step` is the step from each floor `floor_path` reaches to the
/// door's bottom and stated threshold, measured from geometry; with
/// `ramp_selector` a ramp within `ramp_reach` over a space is that side's
/// floor, at its top.
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
            ParameterDescriptor::optional("clear_width_from_leaves", ParameterType::String),
            ParameterDescriptor::optional("overall_height", ParameterType::PropertyReference),
            ParameterDescriptor::optional("lining_thickness", ParameterType::PropertyReference),
            ParameterDescriptor::optional("threshold_thickness", ParameterType::PropertyReference),
            ParameterDescriptor::optional("ramp_selector", ParameterType::Selector),
            ParameterDescriptor::optional("ramp_reach", ParameterType::Quantity),
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
    Ok((keys, limits, quantity(parameters)?))
}

/// Which quantities each quantity-specific parameter applies to.
const APPLIES: &[(&str, &[&str])] = &[
    (
        "quantity_property",
        &["property", "clear-width", "clear-height"],
    ),
    ("floor_path", &["sill-height", "threshold-step"]),
    ("overall_width", &["clear-width"]),
    ("width_deduction", &["clear-width"]),
    ("clear_width_from_leaves", &["clear-width"]),
    ("overall_height", &["clear-height"]),
    ("lining_thickness", &["clear-height"]),
    ("threshold_thickness", &["clear-height", "threshold-step"]),
    ("ramp_selector", &["threshold-step"]),
    ("ramp_reach", &["threshold-step"]),
];

/// The declared quantity, its own parameters checked against it.
fn quantity<'a>(parameters: &Parameters<'a>) -> Result<Quantity<'a>, Unavailable> {
    let named = parameters.required_string("quantity")?;
    if !matches!(
        named,
        "plan-area"
            | "property"
            | "sill-height"
            | "clear-width"
            | "clear-height"
            | "threshold-step"
    ) {
        return Err(invalid(format!("quantity `{named}` is unsupported")));
    }
    for (parameter, quantities) in APPLIES {
        if parameters.0.parameters.contains_key(*parameter) && !quantities.contains(&named) {
            let quantities = quantities
                .iter()
                .map(|quantity| format!("`{quantity}`"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(invalid(format!(
                "`{parameter}` applies only to {quantities}, not `{named}`"
            )));
        }
    }
    let property = parameters.property("quantity_property")?;
    let floor = parameters.strings("floor_path")?;
    Ok(match named {
        "plan-area" => Quantity::PlanArea,
        "property" => Quantity::Property(
            property.ok_or_else(|| invalid("`quantity` `property` needs `quantity_property`"))?,
        ),
        "sill-height" => Quantity::SillHeight(Traversal::path(
            floor.ok_or_else(|| invalid("`quantity` `sill-height` needs `floor_path`"))?,
        )?),
        "clear-width" => Quantity::ClearWidth(ClearWidth::declared(
            property,
            LeafMode::parse(parameters.string("clear_width_from_leaves")?)?,
            parameters.property("overall_width")?,
            length(parameters, "width_deduction")?,
        )?),
        "clear-height" => {
            let overall = parameters.property("overall_height")?;
            let lining = parameters.property("lining_thickness")?;
            let threshold = parameters.property("threshold_thickness")?;
            if property.is_none() && overall.is_none() {
                return Err(invalid(
                    "`quantity` `clear-height` needs `quantity_property` or `overall_height`",
                ));
            }
            if overall.is_none() && (lining.is_some() || threshold.is_some()) {
                return Err(invalid(
                    "`lining_thickness` and `threshold_thickness` are deducted from \
                     `overall_height`, which is not declared",
                ));
            }
            Quantity::ClearHeight(ClearHeight {
                stated: property,
                overall,
                lining,
                threshold,
            })
        }
        _ => Quantity::ThresholdStep(threshold::ThresholdStep::parse(
            parameters,
            Traversal::path(
                floor.ok_or_else(|| invalid("`quantity` `threshold-step` needs `floor_path`"))?,
            )?,
        )?),
    })
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
        Quantity::SillHeight(_) | Quantity::ThresholdStep(_) => Err(invalid(
            "a sill height or threshold step is judged per floor",
        )),
        Quantity::ClearWidth(clear) => clear.measure(context, object),
        Quantity::ClearHeight(clear) => clear.measure(context, object),
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
    if let Quantity::ThresholdStep(step) = quantity {
        let described = format!("limit row {index}: {}", keys.describe(declared));
        let limit = (limit.minimum, limit.maximum);
        return step.judge(
            context,
            rule,
            subject,
            limit,
            &described,
            keys.evidence,
            keys.sources,
        );
    }
    let measured = measure(context, quantity, subject)?;
    let unit = &measured.unit;
    let verdict = if matches!(quantity, Quantity::ClearWidth(_) | Quantity::ClearHeight(_)) {
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
                value.is_finite()
                    && (value - bound).abs() <= 4.0 * f64::EPSILON * value.abs().max(bound.abs())
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
