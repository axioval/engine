//! `accessible-route` as it judged before its decision became a template
//! over its walk (#286), kept to hold the template to (see `templates.md`).
//! Compiled only with the `parity-reference` feature; never registered. It
//! shares the walk with the template's list.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

use super::{Verdict, declaration, walked};
use crate::selection::select_objects;

/// The capability as it judged before its template.
pub struct AccessibleRoute;

impl RuleCapability for AccessibleRoute {
    fn id(&self) -> &'static str {
        "axioval:capability.accessible-route"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        super::parameters()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("accessible-route: {message}"),
                );
            }
        };
        let (destinations, mut evaluation) = select_objects(context, &rule.selector);
        if destinations.is_empty() {
            return evaluation;
        }
        let walked = match walked(context, &declared, &destinations) {
            Ok(walked) => walked,
            Err((reason, message)) => {
                for destination in &destinations {
                    evaluation.push_object_not_evaluated(
                        destination.id.clone(),
                        reason.clone(),
                        message.clone(),
                    );
                }
                return evaluation;
            }
        };
        let judge = walked.judge(context, &declared);
        for destination in destinations {
            match judge.destination(&destination.id) {
                Verdict::Reachable => {}
                Verdict::Blocked(blocked) => {
                    evaluation.push_finding(judge.finding(rule, &destination.id, blocked));
                }
                Verdict::Crowded(missed) => {
                    evaluation.push_finding(judge.crowded(rule, &destination.id, missed));
                }
                Verdict::Undecided(reason, message) => {
                    evaluation.push_object_not_evaluated(destination.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}
