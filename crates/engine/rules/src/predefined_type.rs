//! Requirements on an object's predefined type.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PropertyRequest, PropertyResolution, PropertyResolutionServiceHandle, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{
    Evidence, Object, PREDEFINED_TYPE, PREDEFINED_TYPE_SET, PREDEFINED_TYPE_USER_DEFINED,
    PropertyValue,
};

use crate::property_value::finding;
use crate::selection::{property_error, select_objects};
use crate::xsd_pattern;

/// An object's predefined type, as its source resolves it in the reserved
/// [`PREDEFINED_TYPE_SET`].
pub(crate) struct Designation {
    /// The designation, or `None` when the object states none.
    pub(crate) value: Option<String>,
    /// The evidence for every answer read.
    pub(crate) evidence: Vec<Evidence>,
}

type Unavailable = (NotEvaluatedReason, String);

/// Reads one property of the reserved set. The names are engine vocabulary,
/// the same in every source, so they are requested as they are and never
/// bound through a package.
fn reserved(
    context: &RuleContext<'_>,
    object: &Object,
    name: &str,
) -> Result<(Option<PropertyValue>, Evidence), Unavailable> {
    let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
        return Err((
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        ));
    };
    let request = PropertyRequest::try_new(
        object.id.clone(),
        Some(PREDEFINED_TYPE_SET.to_owned()),
        name,
    )
    .map_err(|error| (NotEvaluatedReason::InvalidDeclaration, error.to_string()))?;
    match service.resolve(&request) {
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.property();
            let evidence = property.evidence.iter().next().cloned().ok_or_else(|| {
                (
                    NotEvaluatedReason::InvalidEvidence,
                    format!("{PREDEFINED_TYPE_SET}.{name} carries no evidence"),
                )
            })?;
            Ok((Some(property.value.clone()), evidence))
        }
        Ok(PropertyResolution::Absent(proof)) => Ok((None, proof.evidence().clone())),
        Err(error) => Err(property_error(error)),
    }
}

/// The predefined type of `object`.
pub(crate) fn designation(
    context: &RuleContext<'_>,
    object: &Object,
) -> Result<Designation, Unavailable> {
    let (value, evidence) = reserved(context, object, PREDEFINED_TYPE)?;
    let value = match value {
        None => None,
        Some(PropertyValue::String(text)) => Some(text),
        Some(other) => {
            return Err((
                NotEvaluatedReason::InvalidEvidence,
                format!("the predefined type is not text: {other:?}"),
            ));
        }
    };
    Ok(Designation {
        value,
        evidence: vec![evidence],
    })
}

/// Whether the predefined type of `object` is user-defined.
fn user_defined(
    context: &RuleContext<'_>,
    object: &Object,
) -> Result<(bool, Evidence), Unavailable> {
    match reserved(context, object, PREDEFINED_TYPE_USER_DEFINED)? {
        (Some(PropertyValue::Boolean(flag)), evidence) => Ok((flag, evidence)),
        (other, _) => Err((
            NotEvaluatedReason::InvalidEvidence,
            format!(
                "{PREDEFINED_TYPE_SET}.{PREDEFINED_TYPE_USER_DEFINED} is not a boolean: {other:?}"
            ),
        )),
    }
}

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
        for object in selected {
            let judged = if user_defined {
                self::user_defined(context, object).map(|(flag, evidence)| {
                    let failure =
                        (!flag).then(|| "the predefined type is not user-defined".to_owned());
                    (failure, vec![evidence])
                })
            } else {
                designation(context, object).map(|resolved| {
                    let failure = match resolved.value.as_deref() {
                        None => Some("the object has no predefined type".to_owned()),
                        Some(value) => {
                            let listed = values.is_empty() || values.iter().any(|v| v == value);
                            let matched = compiled.is_empty()
                                || compiled.iter().any(|regex| regex.is_match(value));
                            (!(listed && matched)).then(|| {
                                format!(
                                    "the predefined type {value:?} is not one of the required ones"
                                )
                            })
                        }
                    };
                    (failure, resolved.evidence)
                })
            };
            match judged {
                Ok((Some(message), evidence)) => {
                    evaluation.push_finding(finding(rule, object, message, evidence));
                }
                Ok((None, _)) => {}
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}
