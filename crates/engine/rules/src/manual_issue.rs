//! A reviewer's instruction, raised once for all selected objects.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};

use axioval_ir::{Finding, Scope};

use crate::pairs::severity;
use crate::selection::select_objects;
use crate::support::{Parameters, finding, invalid};

/// Raises one finding per rule, carrying a declared text.
///
/// For requirements no capability can decide, such as "check the escape
/// signage by hand": the rule records once that the check is owed and names
/// every object it concerns. The first selected object is the subject and
/// the others are related objects. Nothing is judged, so no evidence is
/// attached. A selection that decidedly picks nothing still owes the check:
/// it is raised once against the project, saying that no object matched.
/// A selection with undecided objects raises nothing of its own; those
/// objects are reported not evaluated.
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
        let mut objects = selected.into_iter().map(|object| object.id.clone());
        if let Some(first) = objects.next() {
            evaluation.push_finding(finding(rule, &first, text, vec![], objects.collect()));
        } else if evaluation.not_evaluated_outcomes().is_empty() {
            evaluation.push_finding(Finding::new(
                rule.id.clone(),
                Scope::Project,
                severity(rule),
                format!("{text} (no object matches the selection)"),
            ));
        }
        evaluation
    }
}
