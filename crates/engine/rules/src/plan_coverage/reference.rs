//! `plan-coverage` as it was implemented before it became a template
//! (#282), kept only as the parity reference the template is held to in the
//! rules crate's tests (`parity-reference` feature). It is no capability of
//! any registry. Its search is the one the template's measured value runs.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, RuleCapability,
    RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId};

use super::search;
use crate::counts::Population;
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Requires each subject's footprint to lie mostly within one candidate, as
/// `plan-coverage` judged it before it became a template.
pub struct PlanCoverage;

impl RuleCapability for PlanCoverage {
    fn id(&self) -> &'static str {
        super::template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::template::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.number("minimum_ratio")?;
            match minimum {
                Some(minimum) if minimum > 0.0 && minimum <= 1.0 => {}
                _ => return Err(invalid("minimum_ratio must lie in (0, 1]")),
            }
            Ok::<_, Unavailable>((
                parameters.required_selector("candidate_selector")?,
                minimum.unwrap_or(1.0),
                parameters.traversal()?,
            ))
        })();
        let (candidates, minimum, traversal) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("plan-coverage: {message}"),
                );
            }
        };
        let candidates = Population::of(context, candidates);
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        for subject in subjects {
            match coverage(context, traversal.as_ref(), subject, &candidates, minimum) {
                Ok(None) => {}
                Ok(Some((message, evidence, best))) => evaluation.push_finding(finding(
                    rule,
                    &subject.id,
                    message,
                    evidence,
                    best.into_iter().collect(),
                )),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

type Failure = (String, Vec<Evidence>, Option<ObjectId>);

/// `None` when covered; the finding when conclusively not.
fn coverage(
    context: &RuleContext<'_>,
    traversal: Option<&Traversal>,
    subject: &Object,
    candidates: &Population,
    minimum: f64,
) -> Result<Option<Failure>, Unavailable> {
    let found = search(context, traversal, subject, candidates, minimum)?;
    if found.covered.is_some() {
        return Ok(None);
    }
    if found.undecided {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("coverage of {minimum} cannot be decided from the measured areas"),
        ));
    }
    Ok(Some((
        format!(
            "at most {} of the footprint lies within any {}; required {minimum}",
            (found.upper.min(1.0) * 1e4).round() / 1e4,
            if found.best.is_none() {
                "candidate (there are none)".to_owned()
            } else {
                "candidate".to_owned()
            }
        ),
        found.evidence,
        found.best,
    )))
}
