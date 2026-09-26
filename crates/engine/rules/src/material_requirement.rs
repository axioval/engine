//! Requirements on an object's assigned material.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, MaterialError, MaterialServiceHandle, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;

use crate::classification_requirement::Matcher;
use crate::property_value::finding;
use crate::selection::select_objects;

/// Requires an object to have a material, optionally one known by a name
/// in `values` or matching one of `patterns` (XML Schema, whole value).
///
/// A material is known by every name its source states for it: for a
/// composition, the composition's name and each part's and each part
/// material's name and category. Without a material the object fails,
/// unless `optional`. With `prohibited`, meeting the requirement is the
/// violation.
pub struct MaterialRequirement;
impl RuleCapability for MaterialRequirement {
    fn selectable(&self) -> bool {
        true
    }

    fn id(&self) -> &'static str {
        "axioval:capability.material"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("values", ParameterType::StringList),
            ParameterDescriptor::optional("patterns", ParameterType::StringList),
            ParameterDescriptor::optional("optional", ParameterType::Boolean),
            ParameterDescriptor::optional("prohibited", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let flag = |name: &str| {
            matches!(
                rule.parameters.get(name),
                Some(ParameterValue::Boolean { value: true })
            )
        };
        let (optional, prohibited) = (flag("optional"), flag("prohibited"));
        let name = match Matcher::read(rule, "values", "patterns") {
            Ok(name) if !(optional && prohibited) => name,
            Ok(_) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    "material cannot be both optional and prohibited",
                );
            }
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("material parameters are invalid: {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<MaterialServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "material service is not registered",
                );
            }
            return evaluation;
        };
        for object in selected {
            let material = match service.material(&object.id) {
                Ok(material) => material,
                Err(error) => {
                    let reason = match error {
                        MaterialError::UncoveredSource(_) => NotEvaluatedReason::MissingService,
                        _ => NotEvaluatedReason::InvalidEvidence,
                    };
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason,
                        error.to_string(),
                    );
                    continue;
                }
            };
            let Some(material) = material else {
                if !optional && !prohibited {
                    let evidence = axioval_ir::Evidence::exact(
                        object.id.source.clone(),
                        format!("materials:{}", object.id),
                    );
                    evaluation.push_finding(finding(
                        rule,
                        object,
                        "the object has no material".to_owned(),
                        vec![evidence],
                    ));
                }
                continue;
            };
            let met = name.is_none() || material.names.iter().any(|value| name.matches(value));
            let violation = if prohibited {
                met.then(|| "the object has a prohibited material".to_owned())
            } else {
                (!met).then(|| {
                    format!(
                        "no material name meets the requirement (names: {})",
                        if material.names.is_empty() {
                            "none stated".to_owned()
                        } else {
                            material.names.join(", ")
                        }
                    )
                })
            };
            if let Some(message) = violation {
                evaluation.push_finding(finding(rule, object, message, vec![material.evidence]));
            }
        }
        evaluation
    }
}
