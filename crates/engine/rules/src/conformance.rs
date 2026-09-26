//! Every selected object must satisfy a declared requirement selector.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};

use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::{Parameters, finding};

/// Checks each selected object against a `requirement` selector.
///
/// This is the shape of every "agreed list" check. Each allowed combination
/// of values is one `allOf` of property conditions, and the agreed list is
/// their `anyOf`: a space passes when its type, name and number together
/// match at least one agreed row, a door when its construction type is one
/// of the types agreed for doors. An object the requirement cannot be
/// decided for (a property the source cannot resolve) is not evaluated; it
/// is never read as conforming or as violating.
///
/// The finding cites every property fact the requirement consulted, so the
/// reviewer sees the values that failed. `message` replaces the default
/// finding text.
pub struct SelectorConformance;

impl RuleCapability for SelectorConformance {
    fn id(&self) -> &'static str {
        "axioval:capability.selector-conformance"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("requirement", ParameterType::Selector),
            ParameterDescriptor::optional("message", ParameterType::String),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = parameters
            .required_selector("requirement")
            .and_then(|requirement| Ok((requirement, parameters.string("message")?)));
        let (requirement, message) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("selector-conformance: {message}"),
                );
            }
        };
        let message = message.unwrap_or("does not match any agreed combination of values");
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let mut evidence = Vec::new();
            match selector_matches(context, requirement, object, &mut evidence) {
                Selection::Match => {}
                Selection::NoMatch => evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    message.to_owned(),
                    evidence,
                    vec![],
                )),
                Selection::NotEvaluated(reason, why) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, why);
                }
            }
        }
        evaluation
    }
}
