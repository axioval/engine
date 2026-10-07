//! `local-circulation` as it judged before its decision became a template
//! over its circulation search (#286), kept to hold the template to (see
//! `templates.md`). Compiled only with the `parity-reference` feature; never
//! registered. It shares the search with the template's list.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};

use super::{circulate, declaration};
use crate::selection::select_objects;

/// The capability as it judged before its template.
pub struct LocalCirculation;

impl RuleCapability for LocalCirculation {
    fn id(&self) -> &'static str {
        "axioval:capability.local-circulation"
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
                    format!("local-circulation: {message}"),
                );
            }
        };
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        evaluation.absorb(circulate(context, rule, &declared, &spaces));
        evaluation
    }
}
