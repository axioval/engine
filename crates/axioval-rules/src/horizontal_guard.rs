//! Exact source-neutral horizontal-guard capability.
//!
//! ADR 0004: exposed edges and nearby elements are measured by a
//! [`GuardServiceHandle`]; every height, gap and width that decides whether an
//! edge is adequately guarded lives here.
//!
//! An edge is protected when a barrier is tall enough and close enough, or
//! when the fall beyond it is short enough and lands somewhere wide enough to
//! stand. A barrier that is otherwise adequate can still be defeated by
//! something climbable beside it.

use axioval_engine::{
    CapabilityEvaluation, ClimbableCandidate, CompiledRule, GuardCandidate, GuardEdge, GuardError,
    GuardSearch, GuardServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Finding, Severity};

use crate::selection::select_objects;

/// Tolerance for comparing measured lengths, in metres.
const EPSILON_M: f64 = 1.0e-6;
/// An edge is guarded when this much of it is covered.
const REQUIRED_COVERAGE: f64 = 1.0 - 1.0e-6;

/// Requires exposed edges of walking surfaces to be guarded against falls.
pub struct HorizontalGuard;

impl RuleCapability for HorizontalGuard {
    fn id(&self) -> &'static str {
        "axioval:capability.horizontal-guard"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("minimum_barrier_height_metres", ParameterType::Number),
            ParameterDescriptor::required("maximum_barrier_gap_metres", ParameterType::Number),
            ParameterDescriptor::required("maximum_platform_gap_metres", ParameterType::Number),
            ParameterDescriptor::required("maximum_landing_gap_metres", ParameterType::Number),
            ParameterDescriptor::required("maximum_fall_height_metres", ParameterType::Number),
            ParameterDescriptor::required("minimum_landing_width_metres", ParameterType::Number),
            ParameterDescriptor::required(
                "climbable_barrier_distance_metres",
                ParameterType::Number,
            ),
            ParameterDescriptor::required("maximum_climbable_height_metres", ParameterType::Number),
            ParameterDescriptor::required(
                "minimum_climbable_side_length_metres",
                ParameterType::Number,
            ),
            ParameterDescriptor::required("measure_barrier_from_curb", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        if selected.is_empty() {
            return evaluation;
        }

        let Some(policy) = Policy::from_rule(rule) else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "horizontal-guard declaration is missing or not realisable",
            );
            return evaluation;
        };

        let Some(service) = context.services.get::<GuardServiceHandle>() else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::MissingService,
                "guard service is not registered",
            );
            return evaluation;
        };

        // The search must reach at least as far as any threshold the policy
        // compares against, or a candidate that would have passed is never
        // measured and the edge is wrongly reported unguarded.
        let radius = policy
            .maximum_barrier_gap_metres
            .max(policy.maximum_platform_gap_metres)
            .max(policy.maximum_landing_gap_metres)
            .max(policy.climbable_barrier_distance_metres);
        let Ok(search) = GuardSearch::try_new(radius, sample_spacing(&policy)) else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "horizontal-guard thresholds do not define a usable search",
            );
            return evaluation;
        };

        let measured = match service.measure_guard_edges(search) {
            Ok(measured) => measured,
            Err(error) => {
                evaluation.push_not_evaluated(
                    match error {
                        GuardError::Unavailable => NotEvaluatedReason::IncompleteEvidence,
                        GuardError::InvalidSearch => NotEvaluatedReason::InvalidDeclaration,
                        GuardError::InexactEvidence | GuardError::InvalidQuantity => {
                            NotEvaluatedReason::InvalidEvidence
                        }
                    },
                    error.to_string(),
                );
                return evaluation;
            }
        };

        for edge in measured.edges() {
            let Some(message) = edge_problem(edge, &policy) else {
                continue;
            };
            evaluation.push_finding(Finding {
                rule_id: rule.id.clone(),
                object_id: edge.surface().clone(),
                severity: Severity::Error,
                related: Vec::new(),
                message,
                evidence: vec![measured.evidence().clone()],
            });
        }
        evaluation
    }
}

struct Policy {
    minimum_barrier_height_metres: f64,
    maximum_barrier_gap_metres: f64,
    maximum_platform_gap_metres: f64,
    maximum_landing_gap_metres: f64,
    maximum_fall_height_metres: f64,
    minimum_landing_width_metres: f64,
    climbable_barrier_distance_metres: f64,
    maximum_climbable_height_metres: f64,
    minimum_climbable_side_length_metres: f64,
    measure_barrier_from_curb: bool,
}

impl Policy {
    fn from_rule(rule: &CompiledRule) -> Option<Self> {
        let number = |key: &str| match rule.parameters.get(key)? {
            ParameterValue::Number { value } if value.is_finite() && *value >= 0.0 => Some(*value),
            _ => None,
        };
        let boolean = |key: &str| match rule.parameters.get(key)? {
            ParameterValue::Boolean { value } => Some(*value),
            _ => None,
        };
        Some(Self {
            minimum_barrier_height_metres: number("minimum_barrier_height_metres")?,
            maximum_barrier_gap_metres: number("maximum_barrier_gap_metres")?,
            maximum_platform_gap_metres: number("maximum_platform_gap_metres")?,
            maximum_landing_gap_metres: number("maximum_landing_gap_metres")?,
            maximum_fall_height_metres: number("maximum_fall_height_metres")?,
            minimum_landing_width_metres: number("minimum_landing_width_metres")?,
            climbable_barrier_distance_metres: number("climbable_barrier_distance_metres")?,
            maximum_climbable_height_metres: number("maximum_climbable_height_metres")?,
            minimum_climbable_side_length_metres: number("minimum_climbable_side_length_metres")?,
            measure_barrier_from_curb: boolean("measure_barrier_from_curb")?,
        })
    }
}

/// Samples an edge at half the tightest gap the policy cares about, so a
/// candidate cannot slip between samples.
fn sample_spacing(policy: &Policy) -> f64 {
    let tightest = policy
        .maximum_barrier_gap_metres
        .min(policy.maximum_platform_gap_metres);
    (0.5 * tightest).max(0.1)
}

/// Describes why an edge is unguarded, or `None` when it is protected.
fn edge_problem(edge: &GuardEdge, policy: &Policy) -> Option<String> {
    let barrier_coverage = GuardEdge::covered_fraction(
        &adequate_barriers(edge, policy),
        policy
            .maximum_barrier_gap_metres
            .max(policy.maximum_platform_gap_metres),
    );
    if barrier_coverage >= REQUIRED_COVERAGE {
        return defeating_climbable(edge, policy).map(|climbable| {
            format!(
                "barrier can be climbed from an object {:.3} m away",
                climbable.distance_to_barrier_metres()
            )
        });
    }

    let landing_coverage = GuardEdge::covered_fraction(
        &adequate_landings(edge, policy),
        policy.maximum_landing_gap_metres,
    );
    if landing_coverage >= REQUIRED_COVERAGE {
        return None;
    }

    Some(format!(
        "edge is {:.1}% guarded by barriers and {:.1}% by landings",
        barrier_coverage * 100.0,
        landing_coverage * 100.0
    ))
}

/// Barriers tall enough to stop a fall.
fn adequate_barriers(edge: &GuardEdge, policy: &Policy) -> Vec<GuardCandidate> {
    edge.barriers()
        .iter()
        .filter(|barrier| {
            barrier_height(barrier, policy) + EPSILON_M >= policy.minimum_barrier_height_metres
        })
        .cloned()
        .collect()
}

/// A barrier standing on a curb is only as tall as its exposed part when the
/// declaration says to measure from the curb.
fn barrier_height(barrier: &GuardCandidate, policy: &Policy) -> f64 {
    match (
        policy.measure_barrier_from_curb,
        barrier.curb_top_offset_metres(),
    ) {
        (true, Some(curb)) => barrier.top_offset_metres() - curb,
        _ => barrier.top_offset_metres(),
    }
}

/// Landings close enough below, and wide enough to stand on.
fn adequate_landings(edge: &GuardEdge, policy: &Policy) -> Vec<GuardCandidate> {
    edge.landings()
        .iter()
        .filter(|landing| {
            // `top_offset` is negative below the walking surface, so a short
            // fall is one whose landing is no further down than the allowance.
            landing.top_offset_metres() + EPSILON_M >= -policy.maximum_fall_height_metres
                && landing.landing_width_metres() + EPSILON_M >= policy.minimum_landing_width_metres
        })
        .cloned()
        .collect()
}

/// An object close enough, tall enough and broad enough to climb.
fn defeating_climbable<'edge>(
    edge: &'edge GuardEdge,
    policy: &Policy,
) -> Option<&'edge ClimbableCandidate> {
    edge.climbables().iter().find(|climbable| {
        climbable.distance_to_barrier_metres()
            <= policy.climbable_barrier_distance_metres + EPSILON_M
            && climbable.top_offset_metres() <= policy.maximum_climbable_height_metres + EPSILON_M
            && climbable.minimum_side_length_metres() + EPSILON_M
                >= policy.minimum_climbable_side_length_metres
    })
}
