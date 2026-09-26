//! Requirements on an object's own attributes.

use axioval_engine::{
    AttributeError, AttributeServiceHandle, AttributeValue, CapabilityEvaluation, CompiledRule,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;

use crate::property_value::{self, Constraints, Verdict, finding, forbid_if, judge};
use crate::selection::{bound_name, select_objects};

/// Requires an attribute to be set, and optionally to meet value constraints.
///
/// `attribute` is a property reference without a set, bound like any
/// property concept and read through the source's attribute service. The
/// other parameters are those of `property-value`, all optional: without
/// any, the attribute must merely hold a value. Unset and blank attributes
/// are violations unless `optional`, which lets an unset one pass. A
/// reference or a non-empty list is a value, but has nothing to compare, so a
/// rule that constrains it is not evaluated.
pub struct AttributeValueConstraint;
impl RuleCapability for AttributeValueConstraint {
    fn id(&self) -> &'static str {
        "axioval:capability.attribute-value"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = property_value::PropertyValueConstraint.parameters();
        parameters[0] =
            ParameterDescriptor::required("attribute", ParameterType::PropertyReference);
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some(ParameterValue::PropertyReference {
            property: concept,
            property_set: None,
        }) = rule.parameters.get("attribute")
        else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "attribute-value needs an attribute reference without a property set",
            );
        };
        let constraints = match Constraints::read(rule, true) {
            Ok(constraints) => constraints,
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("attribute-value parameters are invalid: {message}"),
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
            let name = match bound_name(context, object, concept) {
                Ok(name) => name,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let resolved = match service.attribute(&object.id, &name) {
                Ok(resolved) => resolved,
                Err(error) => {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason(&error),
                        error.to_string(),
                    );
                    continue;
                }
            };
            let verdict = match &resolved.value {
                AttributeValue::Unset if constraints.optional || constraints.prohibited => {
                    Verdict::Meets
                }
                AttributeValue::Unset => {
                    Verdict::Fails(format!("missing required attribute {name}"))
                }
                AttributeValue::Structured
                    if constraints.constrains_value() || constraints.data_type.is_some() =>
                {
                    Verdict::Inapplicable(
                        NotEvaluatedReason::InvalidDeclaration,
                        format!(
                            "attribute {name} holds a reference or list, which has no value to compare"
                        ),
                    )
                }
                AttributeValue::Structured => {
                    forbid_if(&constraints, Verdict::Meets, "attribute", &name)
                }
                AttributeValue::Scalar { value, data_type } => forbid_if(
                    &constraints,
                    judge(
                        value,
                        data_type.as_deref(),
                        "attribute",
                        &name,
                        &constraints,
                    ),
                    "attribute",
                    &name,
                ),
            };
            match verdict {
                Verdict::Meets => {}
                Verdict::Fails(message) => evaluation.push_finding(finding(
                    rule,
                    object,
                    message,
                    vec![resolved.evidence.clone()],
                )),
                Verdict::Inapplicable(reason, message) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

fn reason(error: &AttributeError) -> NotEvaluatedReason {
    match error {
        // The declaration names an attribute this class does not have.
        AttributeError::UnknownAttribute { .. } => NotEvaluatedReason::InvalidDeclaration,
        AttributeError::UncoveredSource(_) => NotEvaluatedReason::MissingService,
        AttributeError::UnknownObject(_)
        | AttributeError::Unsupported(_)
        | AttributeError::Unreadable(_) => NotEvaluatedReason::InvalidEvidence,
    }
}
