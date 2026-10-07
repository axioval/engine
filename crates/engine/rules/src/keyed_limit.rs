//! Limits looked up in a keyed table: the applicable row is chosen by key
//! values read from the object or from objects related to it.

mod defaults;
mod limits;
mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;
mod threshold;

pub(crate) use limits::LimitMeasures;
pub(crate) use measured::DoorMeasures;

use std::collections::BTreeMap;
use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_ir::contract::ParameterValue;

use axioval_engine::{
    AdjacentSide, CapabilityEvaluation, ColumnKind, CompiledRule, DoorLeaf, DoorLeaves,
    DoorLeavesError, LeafMotion, NotEvaluatedReason, ObjectFrameServiceHandle, ParameterDescriptor,
    ParameterType, RuleCapability, RuleContext, TableColumn, TraversalDirection, VerticalExtent,
    adjacent_side,
};
use std::sync::Arc;

use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use self::defaults::{DoorDefaults, Item};
use crate::counts::{Population, relation_text};
use crate::door_swing;
use crate::level_spacing::{extent, extents};
use crate::light_area::{LightArea, length};
use crate::opening_spaces::is_adjacency;
use crate::plan_area::{Measure, Verdict, footprint, judge, member_areas, shown};
use crate::support::table::{Matched, RowSelection, RowTest, TextPattern, match_rows};
use crate::support::{
    Parameters, PropertyRef, Traversal, Unavailable, display, exact_f64, invalid, resolve,
    traversal_parameters, undefined,
};

/// How many keys a table may be keyed by.
const KEYS: usize = 4;

const KEY_COLUMNS: [&str; KEYS] = ["key_1", "key_2", "key_3", "key_4"];
const KEY_PATHS: [&str; KEYS] = ["key_1_path", "key_2_path", "key_3_path", "key_4_path"];

/// The reserved key of a face that opens to the outside, for the pair key.
const EXTERIOR: &str = "exterior";

/// How far the number of sides a row names of the pair key is shifted
/// above the other keys' specificity, so that it ranks rows first.
const PAIR_RANK: u32 = 16;

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("key_1", ColumnKind::TextPattern),
    TableColumn::optional("key_2", ColumnKind::TextPattern),
    TableColumn::optional("key_3", ColumnKind::TextPattern),
    TableColumn::optional("key_4", ColumnKind::TextPattern),
    TableColumn::optional("other_side", ColumnKind::TextPattern),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
];

/// Where one key's value is read: a property of the object, or of the
/// objects a relationship path reaches from it.
struct KeySource<'a> {
    property: PropertyRef<'a>,
    path: Option<Traversal>,
}

impl KeySource<'_> {
    fn describe(&self) -> String {
        match &self.path {
            None => self.property.to_string(),
            Some(path) => format!("{} (via {})", self.property, path.relationship),
        }
    }
}

/// The declared keys, and which of them, if any, is read as the unordered
/// pair of the spaces on either face of the object.
struct Declared<'a> {
    sources: Vec<Option<KeySource<'a>>>,
    pair: Option<usize>,
}

/// One row of the limit table, its key patterns compiled.
struct Limit {
    keys: [Option<TextPattern>; KEYS],
    /// The pattern of the pair key's other side.
    other: Option<TextPattern>,
    minimum: Option<f64>,
    maximum: Option<f64>,
}

/// The quantity each row limits.
enum Quantity<'a> {
    PlanArea,
    /// The summed footprints of the members the object reaches, as
    /// `plan-area` sums them with `member_selector`.
    MemberPlanArea {
        members: &'a Selector,
        traversal: Option<Traversal>,
    },
    Property(PropertyRef<'a>),
    /// A registered measured value, read from the measured set: the same
    /// value an expression reads by that name.
    Measured(PropertyRef<'a>),
    /// The object's bottom above the bottom of each object `floor_path`
    /// reaches from it.
    SillHeight(Traversal),
    /// A door's clear width: stated, or its overall width less a deduction
    /// the rule states.
    ClearWidth(ClearWidth<'a>),
    /// A door's clear height: stated, or its overall height less its head
    /// lining and threshold.
    ClearHeight(ClearHeight<'a>),
    /// A door's glazed share of its leaf: stated, or its type's default.
    GlazingRatio(GlazingRatio<'a>),
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
    /// The overall width and the rule's deduction from it, in metres;
    /// without one, the door type's deduction alone.
    derived: Option<(PropertyRef<'a>, Option<f64>)>,
    /// The defaults per door type, whose width deduction comes before the
    /// rule's.
    defaults: Option<Arc<DoorDefaults>>,
}

impl<'a> ClearWidth<'a> {
    /// The declared steps: at least one, the overall width only with a
    /// deduction (the rule's, the door type's, or both) and the rule's
    /// deduction only with the overall width.
    fn declared(
        stated: Option<PropertyRef<'a>>,
        leaves: Option<LeafMode>,
        overall: Option<PropertyRef<'a>>,
        deduction: Option<f64>,
        defaults: Option<Arc<DoorDefaults>>,
    ) -> Result<Self, Unavailable> {
        let derived = match (overall, deduction) {
            (Some(overall), deduction) if deduction.is_some() || defaults.is_some() => {
                Some((overall, deduction))
            }
            (None, None) => None,
            _ => {
                return Err(invalid(
                    "`overall_width` is declared with `width_deduction` or                      `door_type_defaults`, and `width_deduction` with `overall_width`",
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
            defaults,
        })
    }

    /// The clear width of `object`: the stated one when present, else the
    /// overall width less the deduction. Only an exact absence moves on to
    /// the next step; a stated value that is not a positive length stops the
    /// chain rather than being replaced by an approximation.
    fn measure(&self, context: &RuleContext<'_>, object: &Object) -> Result<Measured, Unavailable> {
        let mut evidence = Vec::new();
        if let Some(stated) = self.stated {
            if let Some((lower, upper)) = stated_width(context, object, stated, &mut evidence)? {
                evidence.push(Self::record(&object.id, "stated", upper <= lower));
                return Ok(Measured {
                    lower,
                    upper,
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
        let typed = match &self.defaults {
            Some(defaults) => defaults.lookup(context, object, Item::WidthDeduction)?,
            None => None,
        };
        let (deduction, words, step) = match (typed, deduction) {
            (Some(used), _) => {
                evidence.extend(used.evidence);
                (
                    used.value,
                    used.words,
                    format!("overall-width-less-type-deduction;deduction={}", used.value),
                )
            }
            (None, Some(deduction)) => (
                deduction,
                format!("the rule's deduction {} m", shown(deduction, deduction)),
                format!("overall-width-less-deduction;deduction={deduction}"),
            ),
            (None, None) => {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the clear width is not stated, the door's type gives no width                          deduction and the rule states none, so {overall} cannot be reduced                          to a clear width"
                    ),
                ));
            }
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
        evidence.push(Self::record(&object.id, &step, false));
        Ok(Measured {
            lower,
            upper,
            unit: " m".into(),
            what: format!(
                "clear width ({overall} {} m less {words}, an approximation)",
                shown(width, width),
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

/// A clear width `object` states: `None` when absent, a positive length,
/// or a measured interval of positive lengths (known only that closely);
/// anything else, null included, leaves the object not evaluated.
fn stated_width(
    context: &RuleContext<'_>,
    object: &Object,
    property: PropertyRef<'_>,
    evidence: &mut Vec<Evidence>,
) -> Result<Option<(f64, f64)>, Unavailable> {
    let resolved = resolve(context, object, property)?;
    if let Some(PropertyValue::Measured {
        lower,
        upper,
        dimension: Some(QuantityDimension::Length),
    }) = resolved.value()
        && lower.is_finite()
        && upper.is_finite()
        && *lower > 0.0
        && lower <= upper
    {
        evidence.extend(resolved.evidence());
        return Ok(Some((*lower, *upper)));
    }
    Ok(LightArea::length(context, object, property, evidence)?.map(|width| (width, width)))
}

/// A clear width as an interval, how a message words it, and its
/// evidence.
pub(crate) type MeasuredWidth = ((f64, f64), String, Vec<Evidence>);

/// A door's clear width read as the `clear-width` quantity reads it, from
/// the same steps under the same parameter names, the stated width from a
/// parameter the caller names. Shared with `local-circulation`, which
/// judges entrance widths against its path width; never a second reading.
pub(crate) struct DoorClearWidth<'a>(ClearWidth<'a>);

/// The parameters a [`DoorClearWidth`] reads besides the stated width.
pub(crate) const CLEAR_WIDTH_SOURCES: [&str; 3] = [
    "clear_width_from_leaves",
    "overall_width",
    "width_deduction",
];

impl<'a> DoorClearWidth<'a> {
    /// The declared steps, `None` when none is declared.
    pub(crate) fn parse(
        parameters: &Parameters<'a>,
        stated: &str,
    ) -> Result<Option<Self>, Unavailable> {
        let stated = parameters.property(stated)?;
        let leaves = LeafMode::parse(parameters.string("clear_width_from_leaves")?)?;
        let overall = parameters.property("overall_width")?;
        let deduction = length(parameters, "width_deduction")?;
        if stated.is_none() && leaves.is_none() && overall.is_none() && deduction.is_none() {
            return Ok(None);
        }
        ClearWidth::declared(stated, leaves, overall, deduction, None)
            .map(|steps| Some(Self(steps)))
    }

    /// The clear width of `door` as an interval sure to hold it, how a
    /// message words it (`clear width (…)`) and the evidence.
    pub(crate) fn measure(
        &self,
        context: &RuleContext<'_>,
        door: &Object,
    ) -> Result<MeasuredWidth, Unavailable> {
        let measured = self.0.measure(context, door)?;
        Ok((
            (measured.lower, measured.upper),
            measured.what,
            measured.evidence,
        ))
    }

    /// Judges the clear width of `door` against `minimum`, read as the
    /// decimals it displays: the verdict, the width as a finding words it
    /// (`clear width (…) is 0.8 m`), and the evidence.
    pub(crate) fn judge(
        &self,
        context: &RuleContext<'_>,
        door: &Object,
        minimum: f64,
    ) -> Result<(Verdict, String, Vec<Evidence>), Unavailable> {
        let measured = self.0.measure(context, door)?;
        let verdict = judge_as_displayed(measured.lower, measured.upper, Some(minimum), None);
        Ok((
            verdict,
            format!(
                "{} is {} m",
                measured.what,
                shown(measured.lower, measured.upper)
            ),
            measured.evidence,
        ))
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
/// the door states. A declared thickness the door does not state is its
/// type's default (`height_deduction` for the lining, `threshold_height`)
/// where `door_type_defaults` gives one, else unknown, never zero: the
/// clear height is then bounded only from above.
struct ClearHeight<'a> {
    stated: Option<PropertyRef<'a>>,
    overall: Option<PropertyRef<'a>>,
    lining: Option<PropertyRef<'a>>,
    threshold: Option<PropertyRef<'a>>,
    defaults: Option<Arc<DoorDefaults>>,
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
        for (declared, noun, item) in [
            (self.lining, "lining", Item::HeightDeduction),
            (self.threshold, "threshold", Item::ThresholdHeight),
        ] {
            let Some(property) = declared else { continue };
            let typed = match thickness(context, object, property, &mut evidence)? {
                Some(value) => {
                    words.push(format!("less the {noun} {} m", shown(value, value)));
                    Some(value)
                }
                None => match &self.defaults {
                    Some(defaults) => defaults.lookup(context, object, item)?.map(|used| {
                        evidence.extend(used.evidence);
                        words.push(format!("less {}", used.words));
                        used.value
                    }),
                    None => None,
                },
            };
            if let Some(value) = typed {
                lower = difference(lower, value).0;
                upper = difference(upper, value).1;
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

/// A door's glazed share of its leaf: the fraction `quantity_property`
/// states, else its type's `glazing_ratio` default. Only an exact absence
/// (absent or null) takes the default.
struct GlazingRatio<'a> {
    stated: Option<PropertyRef<'a>>,
    defaults: Option<Arc<DoorDefaults>>,
}

impl GlazingRatio<'_> {
    fn measure(&self, context: &RuleContext<'_>, object: &Object) -> Result<Measured, Unavailable> {
        let mut evidence = Vec::new();
        if let Some(stated) = self.stated {
            let resolved = resolve(context, object, stated)?;
            evidence.extend(resolved.evidence());
            let ratio = match resolved.value() {
                None | Some(PropertyValue::Null) => None,
                Some(PropertyValue::Decimal(value)) if (0.0..=1.0).contains(value) => Some(*value),
                Some(PropertyValue::Integer(value @ (0 | 1))) => exact_f64(*value),
                other => {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "{} {stated} is {}, not a ratio between 0 and 1",
                            object.id,
                            display(other)
                        ),
                    ));
                }
            };
            if let Some(ratio) = ratio {
                return Ok(Measured {
                    lower: ratio,
                    upper: ratio,
                    unit: String::new(),
                    what: format!("glazing ratio ({stated})"),
                    evidence,
                });
            }
        }
        let typed = match &self.defaults {
            Some(defaults) => defaults.lookup(context, object, Item::GlazingRatio)?,
            None => None,
        };
        let Some(used) = typed else {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                match self.stated {
                    Some(stated) => format!(
                        "{stated} is absent and the door's type gives no default glazing ratio"
                    ),
                    None => "the door's type gives no default glazing ratio".into(),
                },
            ));
        };
        evidence.extend(used.evidence);
        Ok(Measured {
            lower: used.value,
            upper: used.value,
            unit: String::new(),
            what: format!("glazing ratio ({})", used.words),
            evidence,
        })
    }
}

/// A door's clear height as the `clear-height` quantity measures it: its
/// bounds, how it was read, and the evidence. Shared with `escape-route`.
pub(crate) fn door_clear_height<'a>(
    context: &RuleContext<'_>,
    object: &Object,
    stated: Option<PropertyRef<'a>>,
    overall: Option<PropertyRef<'a>>,
    lining: Option<PropertyRef<'a>>,
    threshold: Option<PropertyRef<'a>>,
) -> Result<(f64, f64, String, Vec<Evidence>), Unavailable> {
    let measured = ClearHeight {
        stated,
        overall,
        lining,
        threshold,
        defaults: None,
    }
    .measure(context, object)?;
    Ok((
        measured.lower,
        measured.upper,
        measured.what,
        measured.evidence,
    ))
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
    /// The pair key: the key of each face of the object, each known or
    /// unknown, in no particular order.
    Pair(Box<[Self; 2]>),
}

impl Key {
    fn shown(&self) -> String {
        match self {
            Self::Known(text) => format!("`{text}`"),
            Self::Unknown(_) => "unknown".into(),
            Self::Pair(sides) => format!("{} and {}", sides[0].shown(), sides[1].shown()),
        }
    }
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
/// footprint in square metres, `member-plan-area`, the summed footprints of
/// the members `member_selector` picks among the objects the object reaches
/// through the traversal parameters (everywhere in its source without
/// them), as `plan-area` sums a storey's spaces, `property`, the number or
/// quantity stated
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
/// floor, at its top. `glazing-ratio` is the fraction `quantity_property`
/// states, between 0 and 1.
///
/// `door_type_defaults` gives defaults per door type, used only where the
/// door states no value: a row applies by the operation type the door's
/// leaves state (`operation`) and a selector (`applies_to`), the first
/// matching row being the door's type, and gives a `width_deduction` (in
/// place of the rule's), a `height_deduction` (for an unstated head
/// lining), a `threshold_height` (for an unstated threshold) and a
/// `glazing_ratio`. Each default used is named in the message and cited by
/// an inexact `axioval:default.door-type` evidence entry; an undecided type
/// row leaves the door not evaluated.
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
///
/// `pair_key` names a key read on each face of the object along the
/// derived adjacency, giving the unordered pair of space types it connects;
/// a face to the outside is the reserved key `exterior`. A row names one
/// side in that key's column and the other in `other_side`, either blank or
/// a wildcard, and applies in either order. Rows rank first by how many
/// sides they name; equally specific rows apply only when their bounds
/// agree.
pub struct KeyedLimit;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for KeyedLimit {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![
        ParameterDescriptor::required("limits", ParameterType::Table(COLUMNS)).per_object(),
        ParameterDescriptor::required("quantity", ParameterType::String),
        ParameterDescriptor::optional("quantity_property", ParameterType::PropertyReference),
        ParameterDescriptor::optional("measured_value", ParameterType::String),
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
        ParameterDescriptor::optional("member_selector", ParameterType::Selector),
        ParameterDescriptor::optional("pair_key", ParameterType::String),
        ParameterDescriptor::optional(
            "door_type_defaults",
            ParameterType::Table(defaults::COLUMNS),
        ),
    ];
    parameters.extend(traversal_parameters());
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

/// The declaration the capability refused, in its order and words: the
/// keys, the rows, then the quantity. `stated` holds the rule's
/// parameters the list names, by the list's keys (the parameters' own
/// names).
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(stated.clone());
    parse(&Parameters(&rule)).map(|_| ())
}

type Parsed<'a> = (Declared<'a>, Vec<Limit>, Quantity<'a>);

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Parsed<'a>, Unavailable> {
    let declared = declared(parameters)?;
    let limits = limits(parameters, &declared)?;
    Ok((
        declared,
        limits,
        quantity(parameters, || door_defaults(parameters))?,
    ))
}

/// The declared keys, checked in the capability's order.
fn declared<'a>(parameters: &Parameters<'a>) -> Result<Declared<'a>, Unavailable> {
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
    let pair = pair_key(parameters, &keys)?;
    Ok(Declared {
        sources: keys,
        pair,
    })
}

/// The rows of `limits`, their patterns compiled, checked against the
/// declared keys.
fn limits(parameters: &Parameters<'_>, declared: &Declared<'_>) -> Result<Vec<Limit>, Unavailable> {
    let (keys, pair) = (&declared.sources, declared.pair);
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
        let other = row.pattern("other_side", case_sensitive)?;
        if other.is_some() && pair.is_none() {
            return Err(invalid(format!(
                "limit row {index} keys `other_side`, which needs `pair_key`"
            )));
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
            other,
            minimum,
            maximum,
        });
    }
    Ok(limits)
}

/// The slot of the key `pair_key` names, read as the unordered pair of the
/// spaces on either face of the object. Its path must be the derived
/// adjacency alone, forward, since only that records the faces.
fn pair_key(
    parameters: &Parameters<'_>,
    keys: &[Option<KeySource<'_>>],
) -> Result<Option<usize>, Unavailable> {
    let Some(name) = parameters.string("pair_key")? else {
        return Ok(None);
    };
    let slot = KEY_COLUMNS
        .iter()
        .position(|column| *column == name)
        .ok_or_else(|| invalid(format!("`pair_key` `{name}` is none of `key_1` … `key_4`")))?;
    let source = keys[slot]
        .as_ref()
        .ok_or_else(|| invalid(format!("`pair_key` names `{name}`, which is not declared")))?;
    let sided = source.path.as_ref().is_some_and(|path| {
        matches!(path.steps(), [step]
            if step.direction() == TraversalDirection::Forward
                && matches!(step.relationships(), [relationship]
                    if is_adjacency(relationship.as_str())))
    });
    if !sided {
        return Err(invalid(format!(
            "`pair_key` `{name}` reads the spaces on each face of the object, so `{name}_path` \
             must be `axioval:derived.adjacent-space` alone, forward"
        )));
    }
    Ok(Some(slot))
}

/// Which quantities each quantity-specific parameter applies to.
const APPLIES: &[(&str, &[&str])] = &[
    (
        "quantity_property",
        &["property", "clear-width", "clear-height", "glazing-ratio"],
    ),
    (
        "door_type_defaults",
        &[
            "clear-width",
            "clear-height",
            "threshold-step",
            "glazing-ratio",
        ],
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
    ("measured_value", &["measured"]),
    ("member_selector", &["member-plan-area"]),
    ("relationship", &["member-plan-area"]),
    ("direction", &["member-plan-area"]),
    ("follow_chain", &["member-plan-area"]),
    ("path", &["member-plan-area"]),
    ("skip_absent_relationship_ends", &["member-plan-area"]),
];

/// The registered measured value `measured_value` names, as a quantity.
fn measured_quantity<'a>(parameters: &Parameters<'a>) -> Result<Quantity<'a>, Unavailable> {
    let name = parameters
        .string("measured_value")?
        .ok_or_else(|| invalid("`quantity` `measured` needs `measured_value`"))?;
    axioval_ir::measured::parse(name)
        .map_err(|error| invalid(format!("`measured_value`: {error}")))?;
    Ok(Quantity::Measured(PropertyRef {
        set: Some(axioval_ir::MEASURED_SET),
        name,
    }))
}

/// The rule's `door_type_defaults` table, if it states one.
fn door_defaults(parameters: &Parameters<'_>) -> Result<Option<Arc<DoorDefaults>>, Unavailable> {
    let case_sensitive = parameters.boolean("case_sensitive")?.unwrap_or(true);
    Ok(DoorDefaults::parse(parameters, case_sensitive)?.map(Arc::new))
}

/// The declared quantity's name, each parameter that applies only to
/// other quantities refused.
fn quantity_name<'a>(parameters: &Parameters<'a>) -> Result<&'a str, Unavailable> {
    let named = parameters.required_string("quantity")?;
    if !matches!(
        named,
        "plan-area"
            | "member-plan-area"
            | "property"
            | "sill-height"
            | "clear-width"
            | "clear-height"
            | "threshold-step"
            | "glazing-ratio"
            | "measured"
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
    Ok(named)
}

/// The declared quantity, its own parameters checked against it, with the
/// rule's `door_type_defaults` table (`door_defaults`, or the table already
/// read), read where the capability read it.
fn quantity<'a>(
    parameters: &Parameters<'a>,
    defaults: impl FnOnce() -> Result<Option<Arc<DoorDefaults>>, Unavailable>,
) -> Result<Quantity<'a>, Unavailable> {
    let named = quantity_name(parameters)?;
    let property = parameters.property("quantity_property")?;
    let floor = parameters.strings("floor_path")?;
    let defaults = defaults()?;
    Ok(match named {
        "plan-area" => Quantity::PlanArea,
        "member-plan-area" => Quantity::MemberPlanArea {
            members: parameters
                .selector("member_selector")?
                .ok_or_else(|| invalid("`quantity` `member-plan-area` needs `member_selector`"))?,
            traversal: parameters.traversal()?,
        },
        "property" => Quantity::Property(
            property.ok_or_else(|| invalid("`quantity` `property` needs `quantity_property`"))?,
        ),
        "measured" => measured_quantity(parameters)?,
        "sill-height" => Quantity::SillHeight(Traversal::path(
            floor.ok_or_else(|| invalid("`quantity` `sill-height` needs `floor_path`"))?,
        )?),
        "clear-width" => Quantity::ClearWidth(ClearWidth::declared(
            property,
            LeafMode::parse(parameters.string("clear_width_from_leaves")?)?,
            parameters.property("overall_width")?,
            length(parameters, "width_deduction")?,
            defaults,
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
                defaults,
            })
        }
        "glazing-ratio" => {
            if property.is_none() && defaults.is_none() {
                return Err(invalid(
                    "`quantity` `glazing-ratio` needs `quantity_property`, `door_type_defaults` \
                     or both",
                ));
            }
            Quantity::GlazingRatio(GlazingRatio {
                stated: property,
                defaults,
            })
        }
        _ => Quantity::ThresholdStep(threshold::ThresholdStep::parse(
            parameters,
            Traversal::path(
                floor.ok_or_else(|| invalid("`quantity` `threshold-step` needs `floor_path`"))?,
            )?,
            defaults,
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
        declared: &Declared<'_>,
        object: &Object,
    ) -> Result<Self, Unavailable> {
        let mut keys = Self {
            values: Vec::with_capacity(declared.sources.len()),
            evidence: Vec::new(),
            sources: Vec::new(),
        };
        for (slot, source) in declared.sources.iter().enumerate() {
            let value = match source {
                Some(source) if declared.pair == Some(slot) => {
                    Some(keys.pair(context, source, object)?)
                }
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
        self.agreed(context, source.property, holders, &object.id)
    }

    /// The pair key of `object`: the key of the spaces on each of its faces,
    /// as the derived adjacency records them, or the reserved `exterior` for
    /// a face that opens to the outside. A face's spaces must agree; a face
    /// recorded both ways or neither, a space recorded on no face or both,
    /// and a space stating the reserved key itself leave that side unknown.
    fn pair(
        &mut self,
        context: &RuleContext<'_>,
        source: &KeySource<'_>,
        object: &Object,
    ) -> Result<Key, Unavailable> {
        let Some(path) = &source.path else {
            return Err(invalid("the pair key has no path"));
        };
        let everything: Vec<&Object> = context.project.objects().collect();
        let (reached, cited) = path.related(context, &object.id, &everything)?;
        let faces = [AdjacentSide::Positive, AdjacentSide::Negative];
        let mut spaces: [Vec<ObjectId>; 2] = Default::default();
        let mut misplaced = None;
        for space in reached {
            let sides: Vec<AdjacentSide> = cited
                .iter()
                .filter_map(|item| adjacent_side(&item.locator, &object.id, Some(&space)))
                .collect();
            match sides.as_slice() {
                [side] => spaces[usize::from(*side == AdjacentSide::Negative)].push(space),
                [] => {
                    misplaced.get_or_insert_with(|| {
                        format!(
                            "the adjacency evidence of {} records no face for {space}",
                            object.id
                        )
                    });
                }
                _ => {
                    misplaced.get_or_insert_with(|| {
                        format!("{space} lies on both faces of {}", object.id)
                    });
                }
            }
        }
        let outside: Vec<AdjacentSide> = cited
            .iter()
            .filter_map(|item| adjacent_side(&item.locator, &object.id, None))
            .collect();
        self.evidence.extend(cited);
        if let Some(why) = misplaced {
            return Ok(Key::Unknown(why));
        }
        let mut sides = Vec::with_capacity(2);
        for (face, held) in faces.into_iter().zip(spaces) {
            let open = outside.contains(&face);
            sides.push(match (held.is_empty(), open) {
                (true, true) => Key::Known(EXTERIOR.into()),
                (true, false) => Key::Unknown(format!(
                    "the adjacency evidence of {} records neither a space nor the outside on \
                     its {face} face",
                    object.id
                )),
                (false, true) => Key::Unknown(format!(
                    "the adjacency evidence of {} records both a space and the outside on its \
                     {face} face",
                    object.id
                )),
                (false, false) => match self.agreed(context, source.property, held, &object.id)? {
                    Key::Known(text) if text.eq_ignore_ascii_case(EXTERIOR) => {
                        Key::Unknown(format!(
                            "{} on the {face} face of {} states the reserved key `{EXTERIOR}`",
                            source.property, object.id
                        ))
                    }
                    other => other,
                },
            });
        }
        let negative = sides.pop().unwrap_or_else(|| Key::Unknown(String::new()));
        let positive = sides.pop().unwrap_or_else(|| Key::Unknown(String::new()));
        Ok(Key::Pair(Box::new([positive, negative])))
    }

    /// The one value `property` states on every holder, or why it is
    /// unknown: absent, of another type, or differing between holders.
    fn agreed(
        &mut self,
        context: &RuleContext<'_>,
        property: PropertyRef<'_>,
        holders: Vec<ObjectId>,
        object: &ObjectId,
    ) -> Result<Key, Unavailable> {
        let mut found: Option<(String, ObjectId)> = None;
        for holder in holders {
            let target = crate::selection::object_by_id(context, &holder)
                .ok_or_else(|| invalid(format!("{holder} is not in the project")))?;
            let resolved = resolve(context, target, property)?;
            self.evidence.extend(resolved.evidence());
            if holder != *object {
                self.sources.push(holder.clone());
            }
            let text = match resolved.value() {
                value if undefined(value) => {
                    return Ok(Key::Unknown(format!(
                        "{} of {holder} is {}",
                        property,
                        display(value)
                    )));
                }
                Some(PropertyValue::String(text)) => text.clone(),
                Some(PropertyValue::Boolean(value)) => value.to_string(),
                Some(PropertyValue::Integer(value)) => value.to_string(),
                other => {
                    return Ok(Key::Unknown(format!(
                        "{} of {holder} is {}, not text, a boolean or an integer",
                        property,
                        display(other)
                    )));
                }
            };
            match &found {
                Some((held, first)) if *held != text => {
                    return Ok(Key::Unknown(format!(
                        "{property} differs between {first} (`{held}`) and {holder} (`{text}`)"
                    )));
                }
                Some(_) => {}
                None => found = Some((text, holder)),
            }
        }
        Ok(found.map_or_else(
            || Key::Unknown(format!("{property} has no value")),
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
                    (_, Some(Key::Pair(sides))) => {
                        pair_test(pattern.as_ref(), limit.other.as_ref(), sides)
                    }
                    (None, _) => RowTest::Match(0),
                    (Some(pattern), Some(Key::Known(text))) => pattern.test(text),
                    (Some(_), Some(Key::Unknown(_)) | None) => RowTest::Undecided,
                })
            })
    }

    /// The keys as a reviewer reads them: each property and its value.
    fn describe(&self, declared: &Declared<'_>) -> String {
        declared
            .sources
            .iter()
            .zip(&self.values)
            .filter_map(|(source, value)| {
                let source = source.as_ref()?;
                Some(match value {
                    Some(value) => format!("{} {}", source.describe(), value.shown()),
                    None => format!("{} unknown", source.describe()),
                })
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn unknown(&self) -> Vec<&str> {
        self.values
            .iter()
            .flat_map(|value| match value {
                Some(Key::Unknown(why)) => vec![why.as_str()],
                Some(Key::Pair(sides)) => sides
                    .iter()
                    .filter_map(|side| match side {
                        Key::Unknown(why) => Some(why.as_str()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .collect()
    }
}

/// Whether a row applies to the pair key's two sides, in either order.
/// Its specificity is the number of sides the row names (a pattern holding
/// a literal character), ranked above every other key's.
fn pair_test(
    first: Option<&TextPattern>,
    second: Option<&TextPattern>,
    sides: &[Key; 2],
) -> RowTest {
    let side = |pattern: Option<&TextPattern>, key: &Key| match (pattern, key) {
        (None, _) => RowTest::Match(0),
        (Some(pattern), Key::Known(text)) => pattern.test(text),
        (Some(_), _) => RowTest::Undecided,
    };
    let straight = side(first, &sides[0]).and(side(second, &sides[1]));
    let crossed = side(first, &sides[1]).and(side(second, &sides[0]));
    let named: u32 = [first, second]
        .into_iter()
        .map(|pattern| u32::from(pattern.is_some_and(|pattern| pattern.literals() > 0)))
        .sum();
    match (straight, crossed) {
        (RowTest::Match(_), _) | (_, RowTest::Match(_)) => RowTest::Match(named << PAIR_RANK),
        (RowTest::Undecided, _) | (_, RowTest::Undecided) => RowTest::Undecided,
        _ => RowTest::NoMatch,
    }
}

/// Whether the rows tied for most specific all set the same bounds.
fn agree(limits: &[Limit], rows: &[usize]) -> bool {
    let bounds = |limit: &Limit| {
        (
            limit.minimum.map(f64::to_bits),
            limit.maximum.map(f64::to_bits),
        )
    };
    rows.windows(2)
        .all(|pair| bounds(&limits[pair[0]]) == bounds(&limits[pair[1]]))
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
        Quantity::MemberPlanArea { .. } => Err(invalid("member areas are summed per anchor")),
        Quantity::ClearWidth(clear) => clear.measure(context, object),
        Quantity::ClearHeight(clear) => clear.measure(context, object),
        Quantity::GlazingRatio(glazing) => glazing.measure(context, object),
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
        Quantity::Measured(property) => {
            let resolved = resolve(context, object, *property)?;
            let (lower, upper, unit) = match resolved.value() {
                Some(PropertyValue::Quantity { value, dimension }) => {
                    (*value, *value, format!(" {}", dimension.unit_symbol()))
                }
                Some(PropertyValue::Measured {
                    lower,
                    upper,
                    dimension,
                }) => (
                    *lower,
                    *upper,
                    dimension.map_or_else(String::new, |dimension| {
                        format!(" {}", dimension.unit_symbol())
                    }),
                ),
                Some(PropertyValue::Decimal(value)) => (*value, *value, String::new()),
                Some(PropertyValue::Integer(value)) => {
                    let value = exact_f64(*value).ok_or_else(|| {
                        (
                            NotEvaluatedReason::InvalidEvidence,
                            format!("{property} {value} cannot be compared exactly"),
                        )
                    })?;
                    (value, value, String::new())
                }
                other => {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{property} is no number ({})", display(other)),
                    ));
                }
            };
            Ok(Measured {
                lower,
                upper,
                unit,
                what: property.name.to_owned(),
                evidence: resolved.evidence(),
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

/// The quantity a rule limits, with the members it sums, if any.
struct Measuring<'q, 'a> {
    quantity: &'q Quantity<'a>,
    members: Option<&'q Population>,
}

impl Measuring<'_, '_> {
    /// The quantity of `object`, the members it summed and how many
    /// reached objects may be members undecided.
    fn measure(
        &self,
        context: &RuleContext<'_>,
        object: &Object,
    ) -> Result<(Measured, Vec<ObjectId>, usize), Unavailable> {
        let (Quantity::MemberPlanArea { traversal, .. }, Some(population)) =
            (self.quantity, self.members)
        else {
            return Ok((measure(context, self.quantity, object)?, Vec::new(), 0));
        };
        let (sum, reached) = member_areas(
            context,
            traversal.as_ref(),
            object,
            population,
            Measure::Footprint,
        )?;
        let (related, undecided) = reached.unwrap_or_default();
        Ok((
            Measured {
                lower: sum.lower,
                upper: sum.upper,
                unit: " m²".into(),
                what: format!(
                    "summed plan area of the members {}",
                    relation_text(traversal.as_ref())
                ),
                evidence: sum.evidence,
            },
            related,
            undecided,
        ))
    }
}

/// The single row that applies to `keys`, `None` when no row matches.
/// Rows tied for most specific are refused, unless the pair key is
/// declared and their bounds agree.
fn select<'l>(
    limits: &'l [Limit],
    declared: &Declared<'_>,
    keys: &Keys,
) -> Result<Option<(usize, &'l Limit)>, Unavailable> {
    match match_rows(limits, RowSelection::MostSpecific, |limit| keys.test(limit)) {
        Matched::Rows(rows) => Ok(rows.first().copied()),
        Matched::Undecided => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "the applicable limit row cannot be decided: {}",
                keys.unknown().join("; ")
            ),
        )),
        Matched::Ambiguous(rows) if declared.pair.is_some() && agree(limits, &rows) => {
            Ok(Some((rows[0], &limits[rows[0]])))
        }
        Matched::Ambiguous(rows) => Err(invalid(format!(
            "limit rows {} apply equally to {}",
            rows.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            keys.describe(declared)
        ))),
    }
}

/// The row an object's keys select, as the measured values read it: its
/// index (none where no row matches), the keys as a message describes them,
/// and what they were read from.
pub(crate) struct Selected {
    pub(crate) index: Option<usize>,
    pub(crate) described: String,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) sources: Vec<ObjectId>,
}

/// The row `subject`'s keys select among `limits`; refused where the keys
/// cannot decide it or rows tie, as the capability refused it.
fn selected(
    context: &RuleContext<'_>,
    declared: &Declared<'_>,
    limits: &[Limit],
    subject: &Object,
) -> Result<Selected, Unavailable> {
    let keys = Keys::read(context, declared, subject)?;
    let index = select(limits, declared, &keys)?.map(|(index, _)| index);
    Ok(Selected {
        index,
        described: keys.describe(declared),
        evidence: keys.evidence,
        sources: keys.sources,
    })
}

/// `minuend - subtrahend` as an interval sure to hold the exact difference: the
/// rounded difference, widened by one step where rounding moved it.
pub(crate) fn difference(minuend: f64, subtrahend: f64) -> (f64, f64) {
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
    let (lower, upper) = snapped(lower, upper, minimum, maximum);
    judge(lower, upper, minimum, maximum)
}

/// `(lower, upper)` read as the decimals they display: an end within a few
/// units in the last place of a bound is the bound.
pub(crate) fn snapped(
    lower: f64,
    upper: f64,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> (f64, f64) {
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
    (snap(lower), snap(upper))
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

/// The sill height above one floor with the floor's evidence, or why the
/// floor cannot be measured.
pub(crate) type Sill = Result<((f64, f64), Evidence), String>;

/// A window's sill height above each floor a path reaches from it.
pub(crate) struct Sills {
    /// Each floor, and the sill height above it with the floor's evidence,
    /// or why the floor cannot be measured.
    pub(crate) floors: Vec<(ObjectId, Sill)>,
    /// The window's extent's evidence.
    pub(crate) window: Evidence,
    /// What the path was followed by.
    pub(crate) cited: Vec<Evidence>,
}

/// The sill height of `subject` above each floor `path` reaches from it.
pub(crate) fn sills(
    context: &RuleContext<'_>,
    path: &Traversal,
    subject: &Object,
) -> Result<Sills, Unavailable> {
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
    let floors = floors
        .into_iter()
        .map(|floor| {
            let measured = extent(service, &floor)
                .map(|measured| {
                    (
                        sill_interval(&window, &measured),
                        measured.evidence().clone(),
                    )
                })
                .map_err(|(_, why)| format!("the floor of {floor} cannot be measured: {why}"));
            (floor, measured)
        })
        .collect();
    Ok(Sills {
        floors,
        window: window.evidence().clone(),
        cited,
    })
}

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
