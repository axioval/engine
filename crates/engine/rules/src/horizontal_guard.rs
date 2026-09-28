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
//!
//! Which objects may act as barriers, landings or climbing aids is a semantic
//! choice the ruleset states through the optional `barrier_selector`,
//! `landing_selector` and `climbable_selector`. Each resolved set travels in
//! the [`GuardSearch`], so a cupboard is not taken for a railing. An absent
//! selector leaves its role open to any nearby body.

use axioval_engine::{
    CapabilityEvaluation, ClimbableCandidate, CompiledRule, GuardCandidate, GuardEdge, GuardError,
    GuardSearch, GuardServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Finding, Severity};

use crate::counts::Population;
use crate::guard_diagnosis::{GuardDefect, GuardDiagnosis};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable};
use axioval_ir::ObjectId;
use std::collections::{BTreeMap, BTreeSet};

/// Tolerance for comparing measured lengths, in metres.
const EPSILON_M: f64 = 1.0e-6;
/// An edge is guarded when this much of it is covered.
const REQUIRED_COVERAGE: f64 = 1.0 - 1.0e-6;
/// A barrier is *present* on an edge only when it runs along more than half.
const BARRIER_PRESENT_COVERAGE: f64 = 0.5;

/// Restricts which objects may count as barriers.
const BARRIER_SELECTOR: &str = "barrier_selector";
/// Restricts which objects may count as landings.
const LANDING_SELECTOR: &str = "landing_selector";
/// Restricts which objects may count as climbing aids.
const CLIMBABLE_SELECTOR: &str = "climbable_selector";

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
            ParameterDescriptor::optional(BARRIER_SELECTOR, ParameterType::Selector),
            ParameterDescriptor::optional(LANDING_SELECTOR, ParameterType::Selector),
            ParameterDescriptor::optional(CLIMBABLE_SELECTOR, ParameterType::Selector),
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
        let roles = match RoleSelectors::from_rule(rule) {
            Ok(roles) => roles,
            Err((reason, message)) => {
                evaluation.push_not_evaluated(reason, format!("horizontal-guard: {message}"));
                return evaluation;
            }
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
        // The selection is the walking-surface profile: the ruleset, not the
        // host, says which edges are checked for fall protection.
        let surfaces: BTreeSet<ObjectId> =
            selected.iter().map(|object| object.id.clone()).collect();
        let Ok(search) = GuardSearch::try_new(radius, sample_spacing(&policy)) else {
            evaluation.push_not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "horizontal-guard thresholds do not define a usable search",
            );
            return evaluation;
        };

        let search = match roles.restrict(
            context,
            search.with_surfaces(surfaces.iter().cloned().collect()),
        ) {
            Ok(search) => search,
            Err((reason, message)) => {
                evaluation.push_not_evaluated(reason, message);
                return evaluation;
            }
        };
        let measured = match service.measure_guard_edges(search.clone()) {
            Ok(measured) => measured,
            Err(error) => {
                evaluation.push_not_evaluated(unmeasured_reason(error), error.to_string());
                return evaluation;
            }
        };

        // One finding per distinct defect on a surface, not one per edge and
        // not only the worst. Edges are sample points along the boundary, so a
        // slab with a short rail on one side and no rail on another has two
        // separate problems and a reviewer must see both.
        let measured_surfaces: BTreeSet<&ObjectId> = measured
            .edges()
            .iter()
            .map(axioval_engine::GuardEdge::surface)
            .collect();
        for surface in &surfaces {
            if !measured_surfaces.contains(surface) {
                evaluation.push_object_not_evaluated(
                    surface.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "no edge was measured for this walking surface; it has no measurable body",
                );
            }
        }
        let mut grouped: BTreeMap<(ObjectId, GuardDefect), Vec<ObjectId>> = BTreeMap::new();
        for edge in measured.edges() {
            if !surfaces.contains(edge.surface()) {
                continue;
            }
            let edge = admitted(edge, &search);
            let Some(diagnosis) = edge_diagnosis(&edge, &policy) else {
                continue;
            };
            grouped
                .entry((edge.surface().clone(), diagnosis.defect()))
                .or_default()
                .extend(diagnosis.related().iter().cloned());
        }
        for ((surface, defect), mut related) in grouped {
            // Each diagnosis sorted its own related elements; merging several
            // edges can still interleave them, so normalise once more here.
            related.sort();
            related.dedup();
            evaluation.push_finding(Finding {
                related,
                evidence: vec![measured.evidence().clone()],
                ..Finding::new(
                    rule.id.clone(),
                    surface,
                    Severity::Error,
                    defect.code().to_string(),
                )
            });
        }
        evaluation
    }
}

/// Why a failed guard measurement leaves the rule not evaluated.
fn unmeasured_reason(error: GuardError) -> NotEvaluatedReason {
    match error {
        GuardError::Unavailable => NotEvaluatedReason::IncompleteEvidence,
        GuardError::InvalidSearch => NotEvaluatedReason::InvalidDeclaration,
        GuardError::InexactEvidence | GuardError::InvalidQuantity => {
            NotEvaluatedReason::InvalidEvidence
        }
    }
}

/// The optional selectors naming which objects may play each guard role.
struct RoleSelectors<'rule> {
    barriers: Option<&'rule Selector>,
    landings: Option<&'rule Selector>,
    climbables: Option<&'rule Selector>,
}

impl<'rule> RoleSelectors<'rule> {
    fn from_rule(rule: &'rule CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        Ok(Self {
            barriers: parameters.selector(BARRIER_SELECTOR)?,
            landings: parameters.selector(LANDING_SELECTOR)?,
            climbables: parameters.selector(CLIMBABLE_SELECTOR)?,
        })
    }

    /// Adds each declared role's resolved set to `search`.
    ///
    /// An object the selector cannot decide might be the rail that guards an
    /// edge, or the cupboard that must not; excluding it could invent a
    /// finding and including it could hide one, so the rule is not evaluated.
    fn restrict(
        &self,
        context: &RuleContext<'_>,
        mut search: GuardSearch,
    ) -> Result<GuardSearch, Unavailable> {
        let resolve = |name: &str, selector: &Selector| {
            let population = Population::of(context, selector);
            if population.undecided.is_empty() {
                Ok(population.matched.into_iter().collect::<Vec<_>>())
            } else {
                Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "horizontal-guard: `{name}` cannot be decided for {} object(s)",
                        population.undecided.len()
                    ),
                ))
            }
        };
        if let Some(selector) = self.barriers {
            search = search.with_barrier_candidates(resolve(BARRIER_SELECTOR, selector)?);
        }
        if let Some(selector) = self.landings {
            search = search.with_landing_candidates(resolve(LANDING_SELECTOR, selector)?);
        }
        if let Some(selector) = self.climbables {
            search = search.with_climbable_candidates(resolve(CLIMBABLE_SELECTOR, selector)?);
        }
        Ok(search)
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

/// The edge with every candidate the search did not admit for its role
/// removed.
///
/// The adapter is asked to filter already; repeating it here keeps a service
/// that ignores the sets from letting a cupboard count as a railing.
fn admitted(edge: &GuardEdge, search: &GuardSearch) -> GuardEdge {
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

/// Samples an edge at half the tightest gap the policy cares about, so a
/// candidate cannot slip between samples.
fn sample_spacing(policy: &Policy) -> f64 {
    let tightest = policy
        .maximum_barrier_gap_metres
        .min(policy.maximum_platform_gap_metres);
    (0.5 * tightest).max(0.1)
}

/// Describes why an edge is unguarded, or `None` when it is protected.
/// Which defect an edge has, or `None` when it is adequately guarded.
///
/// Barrier protection is decided first: an edge covered by barriers that are
/// tall enough is safe unless something climbable defeats them. Only an edge
/// without adequate barrier coverage falls through to landings, where a short
/// fall onto something wide enough to stand on is also safe.
fn edge_diagnosis(edge: &GuardEdge, policy: &Policy) -> Option<GuardDiagnosis> {
    let barriers = adequate_barriers(edge, policy);
    let barrier_coverage = GuardEdge::covered_fraction(
        &barriers,
        policy
            .maximum_barrier_gap_metres
            .max(policy.maximum_platform_gap_metres),
    );

    if barrier_coverage >= REQUIRED_COVERAGE {
        // The barrier covers the edge; only a climbable object can defeat it.
        return defeating_climbable(edge, policy).map(|climbable| {
            GuardDiagnosis::new(
                GuardDefect::BarrierTooLowDueToClimbableObject,
                [climbable.element().clone()],
            )
        });
    }

    // A barrier only counts as *present* on this edge when it runs along more
    // than half of it. Below that the edge is unprotected regardless of how
    // tall the stub is, and the fall itself is what matters -- so the landing
    // branch decides. Without this gate a single short railing beside a long
    // open edge reports `hole_in_barrier` instead of `missing_barrier`.
    let reaching = reaching_barriers(edge, policy);
    let reaching_coverage = GuardEdge::covered_fraction(
        &reaching,
        policy
            .maximum_barrier_gap_metres
            .max(policy.maximum_platform_gap_metres),
    );
    if reaching_coverage > BARRIER_PRESENT_COVERAGE
        && let Some(tallest) = tallest_reaching_barrier(edge, policy)
    {
        let mut defects = Vec::new();
        if barrier_height(&tallest, policy) + EPSILON_M < policy.minimum_barrier_height_metres {
            // The curb is the *reason* the barrier is short, so it is the more
            // specific and more actionable diagnosis of the two.
            let defect = if curb_lowers_barrier(&tallest, policy) {
                GuardDefect::BarrierTooLowDueToCurb
            } else {
                GuardDefect::BarrierTooLow
            };
            defects.push(GuardDiagnosis::new(defect, [tallest.element().clone()]));
        } else if barrier_coverage > 0.0 {
            // Tall enough somewhere, but not along the whole edge.
            defects.push(GuardDiagnosis::new(
                GuardDefect::HoleInBarrier,
                [tallest.element().clone()],
            ));
        }
        if let Some(worst) = GuardDiagnosis::worst(defects) {
            return Some(worst);
        }
    }

    let landings = adequate_landings(edge, policy);
    let landing_coverage =
        GuardEdge::covered_fraction(&landings, policy.maximum_landing_gap_metres);
    if landing_coverage >= REQUIRED_COVERAGE {
        return None;
    }

    // No adequate protection. If a landing was measured at all, the way it
    // falls short is more useful than "missing barrier".
    if let Some(nearest) = nearest_landing(edge) {
        let defect = if nearest.horizontal_gap_metres() > policy.maximum_landing_gap_metres {
            GuardDefect::LandingTooFarAway
        } else if -nearest.top_offset_metres() > policy.maximum_fall_height_metres + EPSILON_M {
            GuardDefect::LandingTooLow
        } else if nearest.landing_width_metres() + EPSILON_M < policy.minimum_landing_width_metres {
            GuardDefect::LandingsTooSmall
        } else {
            GuardDefect::InsufficientLandings
        };
        return Some(GuardDiagnosis::new(defect, [nearest.element().clone()]));
    }

    Some(GuardDiagnosis::new(GuardDefect::MissingBarrier, []))
}

/// The tallest barrier close enough to the edge to matter, regardless of
/// whether it is tall enough. Distinguishes an inadequate barrier from none.
/// Every barrier close enough to the edge to count, whatever its height.
fn reaching_barriers(edge: &GuardEdge, policy: &Policy) -> Vec<GuardCandidate> {
    edge.barriers()
        .iter()
        .filter(|barrier| {
            barrier.horizontal_gap_metres() <= policy.maximum_platform_gap_metres + EPSILON_M
        })
        .cloned()
        .collect()
}

fn tallest_reaching_barrier(edge: &GuardEdge, policy: &Policy) -> Option<GuardCandidate> {
    edge.barriers()
        .iter()
        .filter(|barrier| {
            barrier.horizontal_gap_metres() <= policy.maximum_platform_gap_metres + EPSILON_M
        })
        .max_by(|left, right| {
            barrier_height(left, policy).total_cmp(&barrier_height(right, policy))
        })
        .cloned()
}

/// The landing nearest the edge, whatever its quality.
fn nearest_landing(edge: &GuardEdge) -> Option<GuardCandidate> {
    edge.landings()
        .iter()
        .min_by(|left, right| {
            left.horizontal_gap_metres()
                .total_cmp(&right.horizontal_gap_metres())
        })
        .cloned()
}

/// Whether the barrier only reaches the required height when measured from the
/// floor rather than the curb it stands on.
fn curb_lowers_barrier(barrier: &GuardCandidate, policy: &Policy) -> bool {
    policy.measure_barrier_from_curb
        && barrier.curb_top_offset_metres().is_some()
        && barrier.top_offset_metres() + EPSILON_M >= policy.minimum_barrier_height_metres
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
