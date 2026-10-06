//! `plan-coverage`: whether each subject's footprint lies mostly within one
//! candidate, and the search for that candidate the template reads.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::CoverageSearch;

use crate::counts::{Population, tally};
use crate::plan_area::{service, unavailable};
use crate::support::{Traversal, Unavailable};

/// Requires each subject's footprint to lie mostly within one candidate.
///
/// A space must lie within a fire compartment: for each subject, the share
/// of its footprint that overlaps a candidate (an object `candidate_selector`
/// picks, reached through the declared relationship or anywhere in the
/// subject's source) must reach `minimum_ratio` for at least one candidate.
/// The subject fails when no candidate can reach it, and is not evaluated
/// when one might, given the areas' intervals.
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `plan_coverage`, the share of the footprint within the candidate covering
/// most of it, judged against `minimum_ratio` by the generic range judge.
pub struct PlanCoverage;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for PlanCoverage {
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

/// What the search for a candidate covering a subject's footprint found.
#[derive(Clone)]
pub(crate) struct Search {
    /// The lower share of the first candidate surely covering at least the
    /// minimum, where one does; the search stops there.
    pub(crate) covered: Option<f64>,
    /// The largest lower share among the candidates measured.
    pub(crate) lower: f64,
    /// The largest upper share, the best candidate's; zero without one.
    pub(crate) upper: f64,
    /// The candidate of the largest upper share, the first among equals.
    pub(crate) best: Option<ObjectId>,
    /// Whether a candidate may reach the minimum, or one may be picked.
    pub(crate) undecided: bool,
    pub(crate) evidence: Vec<Evidence>,
}

/// Searches the candidates `subject` reaches for one covering at least
/// `minimum` of its footprint, in the order they are reached, stopping at
/// the first that surely does.
pub(crate) fn search(
    context: &RuleContext<'_>,
    traversal: Option<&Traversal>,
    subject: &Object,
    candidates: &Population,
    minimum: f64,
) -> Result<Search, Unavailable> {
    let service = service(context)?;
    let reached = tally(context, traversal, subject, candidates)?;
    let footprint = service
        .measure_footprint(&subject.id)
        .map_err(unavailable)?;
    if footprint.upper_square_metres() <= 0.0 {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            "the subject has no plan footprint".into(),
        ));
    }
    let mut found = Search {
        covered: None,
        lower: 0.0,
        upper: 0.0,
        best: None,
        undecided: reached.undecided > 0,
        evidence: reached.evidence,
    };
    found.evidence.push(footprint.evidence().clone());
    let mut best: Option<(f64, ObjectId)> = None;
    for candidate in &reached.decided {
        let overlap = service
            .measure_plan_overlap(&subject.id, candidate)
            .map_err(unavailable)?;
        found.evidence.push(overlap.evidence().clone());
        let lower = overlap.lower_square_metres() / footprint.upper_square_metres();
        let upper = if footprint.lower_square_metres() > 0.0 {
            overlap.upper_square_metres() / footprint.lower_square_metres()
        } else {
            f64::INFINITY
        };
        found.lower = found.lower.max(lower);
        if lower >= minimum {
            found.covered = Some(lower);
            return Ok(found);
        }
        if upper >= minimum {
            found.undecided = true;
        }
        if best.as_ref().is_none_or(|(held, _)| upper > *held) {
            best = Some((upper, candidate.clone()));
        }
    }
    if let Some((upper, candidate)) = best {
        found.upper = upper;
        found.best = Some(candidate);
    }
    Ok(found)
}
