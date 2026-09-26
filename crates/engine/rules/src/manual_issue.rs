//! A reviewer's instruction, raised against every selected object.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};

use crate::selection::select_objects;
use crate::support::{Parameters, finding, invalid};

/// Raises one finding per selected object, carrying a declared text.
///
/// For requirements no capability can decide, such as "check the escape
/// signage by hand": the rule records that the check is owed and names every
/// object it concerns. Nothing is judged, so no evidence is attached. A
/// selection that picks nothing raises nothing.
pub struct ManualIssue;

impl RuleCapability for ManualIssue {
    fn id(&self) -> &'static str {
        "axioval:capability.manual-issue"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("title", ParameterType::String),
            ParameterDescriptor::optional("description", ParameterType::String),
            ParameterDescriptor::optional("category", ParameterType::String),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let title = parameters.required_string("title")?;
            if title.trim().is_empty() {
                return Err(invalid("title is blank"));
            }
            Ok((
                title.trim(),
                parameters.string("description")?.map(str::trim),
                parameters.string("category")?.map(str::trim),
            ))
        })();
        let (title, description, category) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("manual-issue: {message}"),
                );
            }
        };
        let text = [category, Some(title), description]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(": ");
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            evaluation.push_finding(finding(rule, &object.id, text.clone(), vec![], vec![]));
        }
        evaluation
    }
}
