//! The share of a subject's footprint within the candidate covering most of
//! it, as a value: `plan-coverage`'s search ([`super::search`]), over the
//! candidates a rule's selector picks along the rule's traversal.

use axioval_engine::{
    Citation, MeasuredProvider, Measurement, NotEvaluatedReason, PropertyResolutionError,
    RuleContext,
};
use axioval_ir::ObjectId;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};

use super::{Search, search};
use crate::counts::Population;
use crate::measured_kinds::{interval, refused, selection, traversal};
use crate::support::{Unavailable, invalid};

/// Measures `plan_coverage`.
pub(crate) struct CoverageSearch;

const PLAN_COVERAGE: &str = "plan_coverage";

/// The search for `object` the call states.
fn searched(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Search, Unavailable> {
    let selection = selection(context, call, "candidates", None)
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?
        .ok_or_else(|| invalid("`plan_coverage` needs `candidates`"))?;
    let Some(MeasuredArgument::Number(minimum)) = call.argument("minimum") else {
        return Err(invalid("`plan_coverage` needs `minimum`"));
    };
    let candidates = Population {
        matched: selection.matched,
        undecided: selection.undecided,
        first: None,
    };
    let subject = crate::selection::object_by_id(context, object)
        .ok_or_else(|| invalid(format!("{object} is not in the project")))?;
    search(
        context,
        traversal(call)?.as_ref(),
        subject,
        &candidates,
        *minimum,
    )
}

impl MeasuredProvider for CoverageSearch {
    fn names(&self) -> &'static [&'static str] {
        &[PLAN_COVERAGE]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_cited(call, object, context)
            .map(|(measurement, _)| measurement)
    }

    /// A share cites the candidate covering most of the footprint.
    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        let found = searched(call, object, context).map_err(refused(call.name(), object))?;
        let exact = found.evidence.iter().all(|evidence| evidence.exact);
        let locator = format!("{PLAN_COVERAGE}:{object}");
        let minimum = match call.argument("minimum") {
            Some(MeasuredArgument::Number(minimum)) => *minimum,
            _ => 0.0,
        };
        // As far as the search for `minimum` needs it: a covering share
        // stops it; a share that may reach the minimum, or a candidate that
        // may be picked, leaves it reaching the minimum at least.
        let (lower, upper) = match found.covered {
            Some(covered) => (covered, covered),
            None if found.undecided => (
                found.lower,
                if found.upper.is_finite() && found.upper >= minimum {
                    found.upper
                } else {
                    minimum.max(found.lower)
                },
            ),
            None => (found.lower, found.upper),
        };
        Ok((
            interval((lower, upper), None, exact, locator),
            Citation {
                related: found.best.into_iter().collect(),
                evidence: Vec::new(),
                ..Citation::default()
            },
        ))
    }
}
