//! Requirements on an object's predefined type.

use axioval_engine::{
    AttributeError, AttributeServiceHandle, CapabilityEvaluation, CompiledRule, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;

use crate::property_value::finding;
use crate::selection::select_objects;
use crate::xsd_pattern;

/// Requires each selected object's predefined type, as its source resolves
/// it, to be one of `values` or to match one of `patterns` (XML Schema,
/// whole value), or, with `user_defined`, to be user-defined at all.
///
/// An object without a predefined type fails a value or pattern
/// requirement. `user_defined` cannot be combined with values or patterns.
pub struct PredefinedTypeRequirement;
impl RuleCapability for PredefinedTypeRequirement {
    fn selectable(&self) -> bool {
        true
    }

    fn id(&self) -> &'static str {
        "axioval:capability.predefined-type"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("values", ParameterType::StringList),
            ParameterDescriptor::optional("patterns", ParameterType::StringList),
            ParameterDescriptor::optional("user_defined", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let list = |name: &str| match rule.parameters.get(name) {
            Some(ParameterValue::StringList { value }) => value.as_slice(),
            _ => &[],
        };
        let (values, patterns) = (list("values"), list("patterns"));
        let user_defined = matches!(
            rule.parameters.get("user_defined"),
            Some(ParameterValue::Boolean { value: true })
        );
        if user_defined == (!values.is_empty() || !patterns.is_empty()) {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "predefined-type needs either user_defined or values/patterns",
            );
        }
        let compiled = match patterns
            .iter()
            .map(|pattern| {
                xsd_pattern::compile(pattern).map_err(|error| format!("{pattern:?}: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(compiled) => compiled,
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("predefined-type pattern {message}"),
                );
            }
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<AttributeServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "attribute service is not registered",
                );
            }
            return evaluation;
        };
        for object in selected {
            let resolved = match service.predefined_type(&object.id) {
                Ok(resolved) => resolved,
                Err(error) => {
                    let reason = match error {
                        AttributeError::UncoveredSource(_) => NotEvaluatedReason::MissingService,
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
            let failure = if user_defined {
                (!resolved.user_defined)
                    .then(|| "the predefined type is not user-defined".to_owned())
            } else {
                match resolved.value.as_deref() {
                    None => Some("the object has no predefined type".to_owned()),
                    Some(value) => {
                        let listed = values.is_empty() || values.iter().any(|v| v == value);
                        let matched = compiled.is_empty()
                            || compiled.iter().any(|regex| regex.is_match(value));
                        (!(listed && matched)).then(|| {
                            format!("the predefined type {value:?} is not one of the required ones")
                        })
                    }
                }
            };
            if let Some(message) = failure {
                evaluation.push_finding(finding(rule, object, message, vec![resolved.evidence]));
            }
        }
        evaluation
    }
}
