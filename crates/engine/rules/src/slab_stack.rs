//! The spacing of slabs stacked above one another, from their surfaces.
//!
//! `level-spacing` reads storey elevations; this reads the measured bottom
//! and top of each slab. Two slabs belong to one stack when their plan
//! footprints overlap by at least a declared share, and each slab is checked
//! against the next one up in its stack.
//!
//! Every measurement is an interval: a tessellated slab's elevations and a
//! tessellated footprint are approximate. A verdict needs the whole interval
//! on one side of a bound, and anything the intervals leave open -- whether
//! two slabs stack, which of two slabs is the next one up, whether a distance
//! meets a bound -- is not evaluated, never passed.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ElevationInterval, NotEvaluatedReason, ParameterDescriptor,
    ParameterType, PlanArea, PlanAreaError, PlanAreaServiceHandle, ProximityServiceHandle,
    RuleCapability, RuleContext, VerticalExtent, VerticalExtentError, VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use crate::level_spacing::{metres, prevailing};
use crate::pairs::{Unevaluated, refuse_all};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Checks the distances between consecutive slabs of each stack.
///
/// Two selected slabs stack when the overlap of their footprints is at least
/// `minimum_overlap_ratio` of the smaller footprint. Stacked slabs are
/// ordered by the elevation of their tops, and each slab is checked against
/// the next slab up that it stacks with:
///
/// - top to top: the rise from this slab's top to the next one's top;
/// - bottom to bottom: the rise from bottom to bottom;
/// - top to bottom: the clear gap from this slab's top to the next one's
///   underside, negative when they overlap vertically.
///
/// Each distance may be bounded by `<measure>_minimum` and
/// `<measure>_maximum`, inclusive. `consistent` names the measures that must
/// be equal, within `tolerance` (1 mm by default), across every pair of one
/// stack: slabs connected through consecutive pairs. The reference is the
/// prevailing distance, as in `level-spacing`.
///
/// A slab with no stack partner above it, including the top of a stack and
/// a slab standing alone, has nothing to check.
pub struct SlabStackSpacing;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Measure {
    TopToTop,
    BottomToBottom,
    TopToBottom,
}

impl Measure {
    const ALL: [Self; 3] = [Self::TopToTop, Self::BottomToBottom, Self::TopToBottom];

    fn name(self) -> &'static str {
        match self {
            Self::TopToTop => "top_to_top",
            Self::BottomToBottom => "bottom_to_bottom",
            Self::TopToBottom => "top_to_bottom",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::TopToTop => "top-to-top distance",
            Self::BottomToBottom => "bottom-to-bottom distance",
            Self::TopToBottom => "clear distance from top to underside",
        }
    }

    /// The distance from `lower` up to `upper`, as an interval.
    fn between(self, lower: &VerticalExtent, upper: &VerticalExtent) -> Interval {
        let (from, to) = match self {
            Self::TopToTop => (lower.top(), upper.top()),
            Self::BottomToBottom => (lower.bottom(), upper.bottom()),
            Self::TopToBottom => (lower.top(), upper.bottom()),
        };
        Interval {
            lower: to.lower_metres() - from.upper_metres(),
            upper: to.upper_metres() - from.lower_metres(),
        }
    }
}

#[derive(Clone, Copy)]
struct Interval {
    lower: f64,
    upper: f64,
}

impl Interval {
    fn midpoint(self) -> f64 {
        f64::midpoint(self.lower, self.upper)
    }

    fn shown(self) -> String {
        let (lower, upper) = (metres(self.lower), metres(self.upper));
        if lower == upper {
            lower
        } else {
            format!("between {lower} and {upper}")
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Band {
    minimum: Option<f64>,
    maximum: Option<f64>,
}

struct Config {
    ratio: f64,
    bands: BTreeMap<Measure, Band>,
    consistent: BTreeSet<Measure>,
    tolerance: f64,
}

impl RuleCapability for SlabStackSpacing {
    fn id(&self) -> &'static str {
        "axioval:capability.slab-stack-spacing"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![ParameterDescriptor::required(
            "minimum_overlap_ratio",
            ParameterType::Number,
        )];
        for measure in Measure::ALL {
            for bound in ["minimum", "maximum"] {
                parameters.push(ParameterDescriptor::optional(
                    format!("{}_{bound}", measure.name()),
                    ParameterType::Quantity,
                ));
            }
        }
        parameters.push(ParameterDescriptor::optional(
            "consistent",
            ParameterType::StringList,
        ));
        parameters.push(ParameterDescriptor::optional(
            "tolerance",
            ParameterType::Quantity,
        ));
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("slab-stack-spacing: {message}"),
                );
            }
        };
        let (slabs, selection) = select_objects(context, &rule.selector);
        let (Some(extents), Some(areas)) = (
            context.services.get::<VerticalExtentServiceHandle>(),
            context.services.get::<PlanAreaServiceHandle>(),
        ) else {
            return refuse_all(
                &slabs,
                selection,
                &NotEvaluatedReason::MissingService,
                "slab-stack-spacing needs the vertical-extent and plan-area services",
            );
        };
        let mut unevaluated = Unevaluated::default();
        // An object whose selection is undecided may be a slab of a stack.
        let mut undecided = Vec::new();
        for outcome in selection.not_evaluated_outcomes() {
            if let Some(object) = outcome.object_id() {
                undecided.push(object.clone());
                unevaluated.push(
                    object.clone(),
                    outcome.reason().clone(),
                    outcome.message().to_owned(),
                );
            }
        }
        let mut stacks = Stacks {
            members: Vec::new(),
            areas,
            boxes: context.services.get::<ProximityServiceHandle>(),
            ratio: config.ratio,
            footprints: BTreeMap::new(),
            partners: BTreeMap::new(),
        };
        let mut unknown = Vec::new();
        let candidates = slabs
            .iter()
            .map(|slab| (slab.id.clone(), true))
            .chain(undecided.into_iter().map(|object| (object, false)));
        for (object, selected) in candidates {
            match extents.measure_vertical_extent(&object) {
                Ok(extent) => stacks.members.push(Member {
                    id: object,
                    extent,
                    selected,
                }),
                Err(error) => {
                    if selected {
                        let (reason, message) = extent_unavailable(&error);
                        unevaluated.push(object.clone(), reason, message);
                    }
                    unknown.push(object);
                }
            }
        }
        let mut evaluation = CapabilityEvaluation::default();
        if let Some(object) = unknown.first() {
            // Its elevation is unknown, so it could sit between any two slabs.
            for member in stacks.members.iter().filter(|member| member.selected) {
                unevaluated.push(
                    member.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the vertical extent of {object} could not be measured, so the stacks \
                         it may belong to are unknown"
                    ),
                );
            }
        } else {
            let pairs = stacks.consecutive(&mut unevaluated);
            check(
                rule,
                &config,
                &stacks.members,
                &pairs,
                &mut evaluation,
                &mut unevaluated,
            );
        }
        unevaluated.drain_into(&mut evaluation);
        evaluation
    }
}

fn parse(parameters: &Parameters<'_>) -> Result<Config, Unavailable> {
    let length = |name: &str| match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some(_) => Err(invalid(format!("{name} must be a non-negative length"))),
    };
    let ratio = match parameters.number("minimum_overlap_ratio")? {
        Some(ratio) if ratio > 0.0 && ratio <= 1.0 => ratio,
        _ => return Err(invalid("minimum_overlap_ratio must lie in (0, 1]")),
    };
    let mut bands = BTreeMap::new();
    for measure in Measure::ALL {
        let band = Band {
            minimum: length(&format!("{}_minimum", measure.name()))?,
            maximum: length(&format!("{}_maximum", measure.name()))?,
        };
        if matches!((band.minimum, band.maximum), (Some(low), Some(high)) if low > high) {
            return Err(invalid(format!(
                "{}_minimum exceeds {0}_maximum",
                measure.name()
            )));
        }
        if band.minimum.is_some() || band.maximum.is_some() {
            bands.insert(measure, band);
        }
    }
    let mut consistent = BTreeSet::new();
    for name in parameters.strings("consistent")?.unwrap_or_default() {
        let measure = Measure::ALL
            .into_iter()
            .find(|measure| measure.name() == name.trim())
            .ok_or_else(|| {
                invalid(format!(
                    "consistent names `{name}`; expected top_to_top, bottom_to_bottom or \
                     top_to_bottom"
                ))
            })?;
        consistent.insert(measure);
    }
    if bands.is_empty() && consistent.is_empty() {
        return Err(invalid(
            "declare a minimum, a maximum or a consistent measure",
        ));
    }
    Ok(Config {
        ratio,
        bands,
        consistent,
        tolerance: length("tolerance")?.unwrap_or(1e-3),
    })
}

fn extent_unavailable(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

fn area_unavailable(error: &PlanAreaError) -> Unavailable {
    let reason = match error {
        PlanAreaError::UnknownObject(_) | PlanAreaError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        PlanAreaError::InvalidMeasurement | PlanAreaError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

struct Member {
    id: ObjectId,
    extent: VerticalExtent,
    /// `false` for an object whose selection is undecided: it may be a slab,
    /// so it may be the next one up, but it is never checked itself.
    selected: bool,
}

/// Whether two objects stack, as far as their footprints tell.
#[derive(Clone)]
enum Partnership {
    Stacked(Vec<Evidence>),
    Apart,
    Unknown(Unavailable),
}

/// Whether `upper` surely comes after `lower` in stack order: its top is
/// above `lower`'s top whatever the intervals hold, or both tops are the
/// same exact elevation and identity breaks the tie.
fn above(lower: &Member, upper: &Member) -> bool {
    let (from, to) = (lower.extent.top(), upper.extent.top());
    #[allow(clippy::float_cmp)]
    let tied = from.is_exact() && to.is_exact() && from.lower_metres() == to.lower_metres();
    to.lower_metres() > from.upper_metres() || (tied && upper.id > lower.id)
}

fn midpoint(elevation: ElevationInterval) -> f64 {
    f64::midpoint(elevation.lower_metres(), elevation.upper_metres())
}

struct Stacks<'a> {
    members: Vec<Member>,
    areas: &'a PlanAreaServiceHandle,
    /// Optional broad phase: enclosing boxes prove disjoint footprints apart
    /// without measuring their overlap.
    boxes: Option<&'a ProximityServiceHandle>,
    ratio: f64,
    footprints: BTreeMap<usize, Result<PlanArea, Unavailable>>,
    partners: BTreeMap<(usize, usize), Partnership>,
}

impl Stacks<'_> {
    fn footprint(&mut self, index: usize) -> Result<PlanArea, Unavailable> {
        let areas = self.areas;
        let members = &self.members;
        self.footprints
            .entry(index)
            .or_insert_with(|| {
                areas
                    .measure_footprint(&members[index].id)
                    .map_err(|error| area_unavailable(&error))
            })
            .clone()
    }

    /// Whether the enclosing plan boxes of two objects share no area.
    fn plan_disjoint(&self, first: usize, second: usize) -> bool {
        let Some(boxes) = self.boxes else {
            return false;
        };
        let plan = |index: usize| {
            let bounds = boxes.bounds(&self.members[index].id).ok()?;
            (bounds.object() == &self.members[index].id).then(|| bounds.enclosing())
        };
        let (Some(a), Some(b)) = (plan(first), plan(second)) else {
            return false;
        };
        (0..2).any(|axis| a.max()[axis].min(b.max()[axis]) <= a.min()[axis].max(b.min()[axis]))
    }

    fn partnership(&mut self, first: usize, second: usize) -> Partnership {
        let key = (first.min(second), first.max(second));
        if let Some(known) = self.partners.get(&key) {
            return known.clone();
        }
        let found = self.measure(key.0, key.1);
        self.partners.insert(key, found.clone());
        found
    }

    fn measure(&mut self, first: usize, second: usize) -> Partnership {
        if self.plan_disjoint(first, second) {
            return Partnership::Apart;
        }
        let measured = (|| {
            let a = self.footprint(first)?;
            let b = self.footprint(second)?;
            let overlap = self
                .areas
                .measure_plan_overlap(&self.members[first].id, &self.members[second].id)
                .map_err(|error| area_unavailable(&error))?;
            let smaller_upper = a.upper_square_metres().min(b.upper_square_metres());
            if smaller_upper <= 0.0 {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} or {} has no plan footprint to stack by",
                        self.members[first].id, self.members[second].id
                    ),
                ));
            }
            let smaller_lower = a.lower_square_metres().min(b.lower_square_metres());
            let lower = overlap.lower_square_metres() / smaller_upper;
            let upper = if smaller_lower > 0.0 {
                overlap.upper_square_metres() / smaller_lower
            } else {
                f64::INFINITY
            };
            let evidence = vec![
                a.evidence().clone(),
                b.evidence().clone(),
                overlap.evidence().clone(),
            ];
            Ok((lower, upper, evidence))
        })();
        let (lower, upper, evidence) = match measured {
            Ok(measured) => measured,
            Err(unavailable) => return Partnership::Unknown(unavailable),
        };
        let (a, b) = (&self.members[first], &self.members[second]);
        if upper < self.ratio {
            Partnership::Apart
        } else if lower < self.ratio {
            Partnership::Unknown((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether {} and {} stack cannot be decided: their plan overlap ratio \
                     straddles {}",
                    a.id, b.id, self.ratio
                ),
            ))
        } else if !(a.selected && b.selected) {
            let undecided = if a.selected { &b.id } else { &a.id };
            Partnership::Unknown((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{undecided} stacks here but whether it is selected is undecided"),
            ))
        } else {
            Partnership::Stacked(evidence)
        }
    }

    /// Each selected slab paired with the next slab up in its stack, with
    /// the evidence that they stack. Slabs whose next one is uncertain are
    /// reported not evaluated instead.
    fn consecutive(&mut self, unevaluated: &mut Unevaluated) -> Vec<(usize, usize, Vec<Evidence>)> {
        let mut pairs = Vec::new();
        for slab in 0..self.members.len() {
            if !self.members[slab].selected {
                continue;
            }
            match self.next_up(slab) {
                Ok(Some((next, evidence))) => pairs.push((slab, next, evidence)),
                Ok(None) => {}
                Err((reason, message)) => {
                    unevaluated.push(self.members[slab].id.clone(), reason, message);
                }
            }
        }
        pairs
    }

    fn next_up(&mut self, slab: usize) -> Result<Option<(usize, Vec<Evidence>)>, Unavailable> {
        // Candidates that may stack with `slab` and are not surely below it.
        let mut higher = Vec::new();
        for other in 0..self.members.len() {
            if other == slab || above(&self.members[other], &self.members[slab]) {
                continue;
            }
            let partnership = self.partnership(slab, other);
            if matches!(partnership, Partnership::Apart) {
                continue;
            }
            if !above(&self.members[slab], &self.members[other]) {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} may stack with {} but which of their tops is higher cannot be \
                         decided",
                        self.members[slab].id, self.members[other].id
                    ),
                ));
            }
            higher.push((other, partnership));
        }
        let members = &self.members;
        let Some((position, _)) = higher.iter().enumerate().min_by(|(_, left), (_, right)| {
            let (left, right) = (&members[left.0], &members[right.0]);
            midpoint(left.extent.top())
                .total_cmp(&midpoint(right.extent.top()))
                .then_with(|| left.id.cmp(&right.id))
        }) else {
            return Ok(None);
        };
        let (next, partnership) = higher.swap_remove(position);
        if let Some((other, _)) = higher
            .iter()
            .find(|(other, _)| !above(&members[next], &members[*other]))
        {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "the next slab up may be {} or {}: their tops cannot be ordered",
                    members[next].id, members[*other].id
                ),
            ));
        }
        match partnership {
            Partnership::Stacked(evidence) => Ok(Some((next, evidence))),
            Partnership::Unknown((reason, message)) => Err((
                reason,
                format!("the next slab up cannot be identified: {message}"),
            )),
            Partnership::Apart => unreachable!("slabs apart were skipped"),
        }
    }
}

/// A consecutive pair and what was measured between it.
struct Measured<'a> {
    lower: &'a Member,
    upper: &'a Member,
    evidence: Vec<Evidence>,
}

fn check(
    rule: &CompiledRule,
    config: &Config,
    members: &[Member],
    pairs: &[(usize, usize, Vec<Evidence>)],
    evaluation: &mut CapabilityEvaluation,
    unevaluated: &mut Unevaluated,
) {
    let measured: Vec<Measured<'_>> = pairs
        .iter()
        .map(|(lower, upper, stacked)| {
            let mut evidence = stacked.clone();
            evidence.push(members[*lower].extent.evidence().clone());
            evidence.push(members[*upper].extent.evidence().clone());
            Measured {
                lower: &members[*lower],
                upper: &members[*upper],
                evidence,
            }
        })
        .collect();
    for pair in &measured {
        for (measure, band) in &config.bands {
            judge(rule, *measure, *band, pair, evaluation, unevaluated);
        }
    }
    if config.consistent.is_empty() {
        return;
    }
    for stack in components(members.len(), pairs) {
        if stack.len() < 2 {
            continue;
        }
        for measure in &config.consistent {
            consistency(
                rule,
                *measure,
                config.tolerance,
                &stack,
                &measured,
                evaluation,
                unevaluated,
            );
        }
    }
}

fn judge(
    rule: &CompiledRule,
    measure: Measure,
    band: Band,
    pair: &Measured<'_>,
    evaluation: &mut CapabilityEvaluation,
    unevaluated: &mut Unevaluated,
) {
    let distance = measure.between(&pair.lower.extent, &pair.upper.extent);
    let mut verdict = None;
    if let Some(minimum) = band.minimum {
        if distance.upper < minimum {
            verdict = Some((true, format!("at least {}", metres(minimum))));
        } else if distance.lower < minimum {
            verdict = Some((false, format!("at least {}", metres(minimum))));
        }
    }
    if let (None, Some(maximum)) = (&verdict, band.maximum) {
        if distance.lower > maximum {
            verdict = Some((true, format!("at most {}", metres(maximum))));
        } else if distance.upper > maximum {
            verdict = Some((false, format!("at most {}", metres(maximum))));
        }
    }
    match verdict {
        None => {}
        Some((true, bound)) => evaluation.push_finding(finding(
            rule,
            &pair.lower.id,
            format!(
                "{} to {} is {}; required {bound}",
                measure.label(),
                pair.upper.id,
                distance.shown()
            ),
            pair.evidence.clone(),
            vec![pair.upper.id.clone()],
        )),
        Some((false, bound)) => unevaluated.push(
            pair.lower.id.clone(),
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{} to {} is {}, which straddles the bound {bound}",
                measure.label(),
                pair.upper.id,
                distance.shown()
            ),
        ),
    }
}

fn root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Pair indices grouped by stack: slabs connected through consecutive pairs.
fn components(count: usize, pairs: &[(usize, usize, Vec<Evidence>)]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..count).collect();
    for (lower, upper, _) in pairs {
        let (a, b) = (root(&mut parent, *lower), root(&mut parent, *upper));
        parent[a.max(b)] = a.min(b);
    }
    let mut stacks: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (lower, _, _)) in pairs.iter().enumerate() {
        stacks
            .entry(root(&mut parent, *lower))
            .or_default()
            .push(index);
    }
    stacks.into_values().collect()
}

fn consistency(
    rule: &CompiledRule,
    measure: Measure,
    tolerance: f64,
    stack: &[usize],
    measured: &[Measured<'_>],
    evaluation: &mut CapabilityEvaluation,
    unevaluated: &mut Unevaluated,
) {
    let distances: Vec<Interval> = stack
        .iter()
        .map(|index| {
            let pair = &measured[*index];
            measure.between(&pair.lower.extent, &pair.upper.extent)
        })
        .collect();
    let midpoints: Vec<f64> = distances
        .iter()
        .map(|distance| distance.midpoint())
        .collect();
    let Some(reference) = prevailing(&midpoints, tolerance).map(|index| distances[index]) else {
        return;
    };
    for (index, distance) in stack.iter().zip(&distances) {
        let pair = &measured[*index];
        // Nearest and farthest the true distances can be from each other.
        let gap = (distance.lower - reference.upper).max(reference.lower - distance.upper);
        let spread = (distance.upper - reference.lower).max(reference.upper - distance.lower);
        if gap > tolerance {
            evaluation.push_finding(finding(
                rule,
                &pair.lower.id,
                format!(
                    "{} to {} is {}, which differs from the prevailing {} in this stack",
                    measure.label(),
                    pair.upper.id,
                    distance.shown(),
                    reference.shown()
                ),
                pair.evidence.clone(),
                vec![pair.upper.id.clone()],
            ));
        } else if spread > tolerance {
            unevaluated.push(
                pair.lower.id.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} to {} is {}; whether it equals the prevailing {} within {} cannot \
                     be decided",
                    measure.label(),
                    pair.upper.id,
                    distance.shown(),
                    reference.shown(),
                    metres(tolerance)
                ),
            );
        }
    }
}
