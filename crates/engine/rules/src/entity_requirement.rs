//! Requirements on an object's own class.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::Evidence;

use crate::classification_requirement::Matcher;
use crate::predefined_type::designation;
use crate::property_value::finding;
use crate::selection::select_objects;

/// Requires each selected object's class (its kind, compared upper case) to
/// be one of `classes` or match `class_patterns`, and, if given, its
/// predefined type to be one of `predefined_types` or match
/// `predefined_patterns`, as the source resolves it in the reserved
/// predefined-type set.
pub struct EntityRequirement;
impl RuleCapability for EntityRequirement {
    fn selectable(&self) -> bool {
        true
    }

    fn id(&self) -> &'static str {
        "axioval:capability.entity"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("classes", ParameterType::StringList),
            ParameterDescriptor::optional("class_patterns", ParameterType::StringList),
            ParameterDescriptor::optional("predefined_types", ParameterType::StringList),
            ParameterDescriptor::optional("predefined_patterns", ParameterType::StringList),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let matchers = Matcher::read(rule, "classes", "class_patterns").and_then(|class| {
            let predefined = Matcher::read(rule, "predefined_types", "predefined_patterns")?;
            if class.is_none() {
                Err("no class is required".to_owned())
            } else {
                Ok((class, predefined))
            }
        });
        let (class, predefined) = match matchers {
            Ok(matchers) => matchers,
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("entity parameters are invalid: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let kind = object.kind().to_ascii_uppercase();
            if !class.matches(&kind) {
                let evidence =
                    Evidence::exact(object.id.source.clone(), format!("kind:{}", object.id));
                evaluation.push_finding(finding(
                    rule,
                    object,
                    format!("the object is a {kind}, not a required class"),
                    vec![evidence],
                ));
                continue;
            }
            if predefined.is_none() {
                continue;
            }
            match designation(context, object) {
                Ok(resolved) => {
                    let met = resolved
                        .value
                        .as_deref()
                        .is_some_and(|value| predefined.matches(value));
                    if !met {
                        evaluation.push_finding(finding(
                            rule,
                            object,
                            format!(
                                "the predefined type {:?} is not a required one",
                                resolved.value.as_deref().unwrap_or("none")
                            ),
                            resolved.evidence,
                        ));
                    }
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}
