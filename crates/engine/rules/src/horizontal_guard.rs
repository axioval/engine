//! Exact source-neutral horizontal-guard capability.
//!
//! ADR 0004: exposed edges and nearby elements are measured by a
//! [`GuardServiceHandle`](axioval_engine::GuardServiceHandle), as the
//! measured member list `guard_edges` and the measured value
//! `guard_surfaces` ([`GuardMeasures`]); every height, gap and width that
//! decides whether an edge is adequately guarded, and which defect it has,
//! is the template's policy.
//!
//! An edge is protected when a barrier is tall enough and close enough, or
//! when the fall beyond it is short enough and lands somewhere wide enough to
//! stand. A barrier that is otherwise adequate can still be defeated by
//! something climbable beside it.
//!
//! Which objects may act as barriers, landings or climbing aids is a semantic
//! choice the ruleset states through the optional `barrier_selector`,
//! `landing_selector` and `climbable_selector`. Each resolved set travels in
//! the [`GuardSearch`], so a cupboard is not taken for a railing. An absent
//! selector leaves its role open to any nearby body.
//!
//! This module keeps what the measurement and the replaced implementation
//! share: the search the thresholds define, and the filters by which a
//! candidate reaches, stands or climbs.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, ClimbableCandidate, CompiledRule, GuardCandidate, GuardEdge, GuardError,
    GuardSearch, NotEvaluatedReason, ParameterDescriptor, RuleCapability, RuleContext,
};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::GuardMeasures;

/// Tolerance for comparing measured lengths, in metres.
pub(crate) const EPSILON_M: f64 = 1.0e-6;
/// An edge is guarded when this much of it is covered.
pub(crate) const REQUIRED_COVERAGE: f64 = 1.0 - 1.0e-6;

/// Requires exposed edges of walking surfaces to be guarded against falls.
///
/// It runs as a template ([`axioval_engine::template`]): the search measured
/// once for the rule's whole selection (`guard_surfaces`), then each
/// surface's exposed edges (`guard_edges`) judged one by one, one finding
/// per distinct defect on a surface.
pub struct HorizontalGuard;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for HorizontalGuard {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// Why a failed guard measurement leaves the rule not evaluated.
pub(crate) fn unmeasured_reason(error: GuardError) -> NotEvaluatedReason {
    match error {
        GuardError::Unavailable => NotEvaluatedReason::IncompleteEvidence,
        GuardError::InvalidSearch => NotEvaluatedReason::InvalidDeclaration,
        GuardError::InexactEvidence | GuardError::InvalidQuantity => {
            NotEvaluatedReason::InvalidEvidence
        }
    }
}

/// The edge with every candidate the search did not admit for its role
/// removed.
///
/// The adapter is asked to filter already; repeating it here keeps a service
/// that ignores the sets from letting a cupboard count as a railing.
pub(crate) fn admitted(edge: &GuardEdge, search: &GuardSearch) -> GuardEdge {
    GuardEdge::new(
        edge.surface().clone(),
        edge.barriers()
            .iter()
            .filter(|barrier| search.admits_barrier(barrier.element()))
            .cloned()
            .collect(),
        edge.landings()
            .iter()
            .filter(|landing| search.admits_landing(landing.element()))
            .cloned()
            .collect(),
        edge.climbables()
            .iter()
            .filter(|climbable| {
                search.admits_climbable(climbable.element())
                    && search.admits_barrier(climbable.barrier())
            })
            .cloned()
            .collect(),
    )
}

/// The search for the barrier, platform and landing gaps and the climbing
/// distance, or `None` when they define no usable search.
///
/// The search must reach at least as far as any threshold compared
/// against, or a candidate that would have passed is never measured and
/// the edge is wrongly reported unguarded. It samples an edge at half the
/// tightest gap that matters, so a candidate cannot slip between samples.
pub(crate) fn guard_search(
    [barrier_gap, platform_gap, landing_gap, climb_distance]: [f64; 4],
) -> Option<GuardSearch> {
    let radius = barrier_gap
        .max(platform_gap)
        .max(landing_gap)
        .max(climb_distance);
    let spacing = (0.5 * barrier_gap.min(platform_gap)).max(0.1);
    GuardSearch::try_new(radius, spacing).ok()
}

/// The gap a barrier may leave along the edge and still cover it.
pub(crate) fn coverage_gap(barrier_gap: f64, platform_gap: f64) -> f64 {
    barrier_gap.max(platform_gap)
}

/// Every barrier close enough to the edge to count, whatever its height.
pub(crate) fn reaching_barriers(edge: &GuardEdge, platform_gap: f64) -> Vec<GuardCandidate> {
    edge.barriers()
        .iter()
        .filter(|barrier| reaches(barrier, platform_gap))
        .cloned()
        .collect()
}

/// Whether `barrier` stands within `platform_gap` of the edge.
fn reaches(barrier: &GuardCandidate, platform_gap: f64) -> bool {
    barrier.horizontal_gap_metres() <= platform_gap + EPSILON_M
}

/// The tallest barrier close enough to the edge to matter, regardless of
/// whether it is tall enough (the last of equally tall ones): what
/// distinguishes an inadequate barrier from none.
pub(crate) fn tallest_reaching_barrier(
    edge: &GuardEdge,
    platform_gap: f64,
    from_curb: bool,
) -> Option<GuardCandidate> {
    edge.barriers()
        .iter()
        .filter(|barrier| reaches(barrier, platform_gap))
        .max_by(|left, right| {
            barrier_height(left, from_curb).total_cmp(&barrier_height(right, from_curb))
        })
        .cloned()
}

/// The landing nearest the edge, whatever its quality.
pub(crate) fn nearest_landing(edge: &GuardEdge) -> Option<GuardCandidate> {
    edge.landings()
        .iter()
        .min_by(|left, right| {
            left.horizontal_gap_metres()
                .total_cmp(&right.horizontal_gap_metres())
        })
        .cloned()
}

/// A barrier standing on a curb is only as tall as its exposed part when the
/// declaration says to measure from the curb.
pub(crate) fn barrier_height(barrier: &GuardCandidate, from_curb: bool) -> f64 {
    match (from_curb, barrier.curb_top_offset_metres()) {
        (true, Some(curb)) => barrier.top_offset_metres() - curb,
        _ => barrier.top_offset_metres(),
    }
}

/// Whether `landing` is at least `minimum_width` wide to stand on.
pub(crate) fn wide_enough(landing: &GuardCandidate, minimum_width: f64) -> bool {
    landing.landing_width_metres() + EPSILON_M >= minimum_width
}

/// Whether `climbable` stands within `distance` of its barrier and is at
/// least `side` broad: close and broad enough to climb, whatever its
/// height.
pub(crate) fn may_climb(climbable: &ClimbableCandidate, distance: f64, side: f64) -> bool {
    climbable.distance_to_barrier_metres() <= distance + EPSILON_M
        && climbable.minimum_side_length_metres() + EPSILON_M >= side
}
