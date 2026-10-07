//! The `horizontal-guard` implementation its template replaced, kept only
//! as the parity reference the template is held to in tests
//! (`axioval_rules::reference::HorizontalGuard`): the decision of which
//! defect an edge has, over the same search and filters the measured
//! `guard_edges` reads. Never register it.

use axioval_engine::{
    CapabilityEvaluation, ClimbableCandidate, CompiledRule, GuardCandidate, GuardEdge, GuardSearch,
    GuardServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Finding, Severity};

use super::{
    EPSILON_M, REQUIRED_COVERAGE, admitted, barrier_height, coverage_gap, guard_search, may_climb,
    nearest_landing, reaching_barriers, tallest_reaching_barrier, unmeasured_reason, wide_enough,
};
use crate::counts::Population;
use crate::guard_diagnosis::GuardDefect;
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable};
use axioval_ir::ObjectId;
use std::collections::{BTreeMap, BTreeSet};

/// A barrier is *present* on an edge only when it runs along more than half.
const BARRIER_PRESENT_COVERAGE: f64 = 0.5;

/// Restricts which objects may count as barriers.
const BARRIER_SELECTOR: &str = "barrier_selector";
/// Restricts which objects may count as landings.
const LANDING_SELECTOR: &str = "landing_selector";
/// Restricts which objects may count as climbing aids.
const CLIMBABLE_SELECTOR: &str = "climbable_selector";

/// Requires exposed edges of walking surfaces to be guarded against falls,
/// as the capability judged it before it became a template.
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

        // The selection is the walking-surface profile: the ruleset, not the
        // host, says which edges are checked for fall protection.
        let surfaces: BTreeSet<ObjectId> =
            selected.iter().map(|object| object.id.clone()).collect();
        let Some(search) = guard_search([
            policy.maximum_barrier_gap_metres,
            policy.maximum_platform_gap_metres,
            policy.maximum_landing_gap_metres,
            policy.climbable_barrier_distance_metres,
        ]) else {
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
        coverage_gap(
            policy.maximum_barrier_gap_metres,
            policy.maximum_platform_gap_metres,
        ),
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
    let reaching = reaching_barriers(edge, policy.maximum_platform_gap_metres);
    let reaching_coverage = GuardEdge::covered_fraction(
        &reaching,
        coverage_gap(
            policy.maximum_barrier_gap_metres,
            policy.maximum_platform_gap_metres,
        ),
    );
    if reaching_coverage > BARRIER_PRESENT_COVERAGE
        && let Some(tallest) = tallest_reaching_barrier(
            edge,
            policy.maximum_platform_gap_metres,
            policy.measure_barrier_from_curb,
        )
    {
        let mut defects = Vec::new();
        if barrier_height(&tallest, policy.measure_barrier_from_curb) + EPSILON_M
            < policy.minimum_barrier_height_metres
        {
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
            barrier_height(barrier, policy.measure_barrier_from_curb) + EPSILON_M
                >= policy.minimum_barrier_height_metres
        })
        .cloned()
        .collect()
}

/// Landings close enough below, and wide enough to stand on.
fn adequate_landings(edge: &GuardEdge, policy: &Policy) -> Vec<GuardCandidate> {
    edge.landings()
        .iter()
        .filter(|landing| {
            // `top_offset` is negative below the walking surface, so a short
            // fall is one whose landing is no further down than the allowance.
            landing.top_offset_metres() + EPSILON_M >= -policy.maximum_fall_height_metres
                && wide_enough(landing, policy.minimum_landing_width_metres)
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
        may_climb(
            climbable,
            policy.climbable_barrier_distance_metres,
            policy.minimum_climbable_side_length_metres,
        ) && climbable.top_offset_metres() <= policy.maximum_climbable_height_metres + EPSILON_M
    })
}

/// One defect together with the elements that explain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardDiagnosis {
    defect: GuardDefect,
    related: Vec<ObjectId>,
}

impl GuardDiagnosis {
    /// A diagnosis naming the elements a reviewer needs to look at.
    pub fn new(defect: GuardDefect, related: impl IntoIterator<Item = ObjectId>) -> Self {
        let mut related: Vec<ObjectId> = related.into_iter().collect();
        related.sort();
        related.dedup();
        Self { defect, related }
    }

    /// Which defect this is.
    pub fn defect(&self) -> GuardDefect {
        self.defect
    }

    /// Elements that explain the defect, sorted and deduplicated.
    pub fn related(&self) -> &[ObjectId] {
        &self.related
    }

    /// The worst diagnosis, or `None` when nothing was found.
    ///
    /// Reporting every defect on an edge would bury the one that matters, so a
    /// surface reports its worst. Ties keep the first, which preserves the
    /// order the edges were measured in and keeps output deterministic.
    pub fn worst(diagnoses: impl IntoIterator<Item = Self>) -> Option<Self> {
        diagnoses
            .into_iter()
            .min_by_key(|diagnosis| diagnosis.defect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard_diagnosis::GuardDefect;

    fn oid(value: &str) -> ObjectId {
        ObjectId::new(axioval_ir::SourceId::new("cad", "model").unwrap(), value).unwrap()
    }

    #[test]
    fn worst_ignores_measurement_order() {
        let hole = GuardDiagnosis::new(GuardDefect::HoleInBarrier, [oid("rail")]);
        let missing = GuardDiagnosis::new(GuardDefect::MissingBarrier, []);
        assert_eq!(
            GuardDiagnosis::worst([hole.clone(), missing.clone()]),
            GuardDiagnosis::worst([missing.clone(), hole]),
            "the worst defect must win regardless of which edge was measured first"
        );
        assert_eq!(
            GuardDiagnosis::worst([missing.clone()]).map(|d| d.defect()),
            Some(GuardDefect::MissingBarrier)
        );
    }

    #[test]
    fn related_elements_are_sorted_and_deduplicated() {
        let diagnosis =
            GuardDiagnosis::new(GuardDefect::MissingBarrier, [oid("b"), oid("a"), oid("b")]);
        assert_eq!(diagnosis.related(), &[oid("a"), oid("b")]);
    }

    #[test]
    fn nothing_found_is_not_a_defect() {
        assert_eq!(GuardDiagnosis::worst([]), None);
    }
}
