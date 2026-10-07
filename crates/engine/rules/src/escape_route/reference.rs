//! `escape-route` as it judged before its decision became a template over
//! its search (#286), kept to hold the template to (see `templates.md`).
//! Compiled only with the `parity-reference` feature; never registered. It
//! shares the search with the template's list.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, RuleCapability, RuleContext,
};
use axioval_ir::ObjectId;

use super::{Bindings, Candidates, Selected, declaration, search};
use crate::selection::select_objects;

/// The capability as it judged before its template.
pub struct EscapeRoute;

impl RuleCapability for EscapeRoute {
    fn id(&self) -> &'static str {
        "axioval:capability.escape-route"
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
                    format!("escape-route: {message}"),
                );
            }
        };
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        let candidates = Candidates::select(context, &rule.selector);
        let undecided: Vec<ObjectId> = candidates.undecided.keys().cloned().collect();
        evaluation.absorb(search(
            context,
            rule,
            &declared,
            &Bindings::default(),
            &Selected {
                spaces: &spaces,
                undecided: &undecided,
                checked: &candidates.universe,
            },
        ));
        evaluation
    }
}
