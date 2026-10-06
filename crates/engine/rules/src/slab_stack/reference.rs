//! `slab-stack-spacing` as it was implemented before it became a template
//! (#282), kept only as the parity reference the template is held to in the
//! rules crate's tests (`parity-reference` feature). It is no capability of
//! any registry. Its stack search is the one the template's measured values
//! run.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor,
    PlanAreaServiceHandle, ProximityServiceHandle, RuleCapability, RuleContext,
    VerticalExtentServiceHandle,
};
use axioval_ir::{Evidence, QuantityDimension};

use super::{Interval, Measure, Member, Stacks, components, extent_unavailable};
use crate::level_spacing::{metres, prevailing};
use crate::pairs::{Unevaluated, refuse_all};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

/// Checks the distances between consecutive slabs of each stack, as
/// `slab-stack-spacing` checked them before it became a template.
pub struct SlabStackSpacing;

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
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
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

// The measures, distances and pairs as the replaced implementation named,
// showed and walked them.

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
}

impl Interval {
    fn shown(self) -> String {
        let (lower, upper) = (metres(self.lower), metres(self.upper));
        if lower == upper {
            lower
        } else {
            format!("between {lower} and {upper}")
        }
    }
}

impl Stacks<'_> {
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
}
