//! Requirements that an object be part of a whole of some class.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Decomposition, DecompositionError,
    DecompositionServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::ParameterValue;

use crate::classification_requirement::Matcher;
use crate::property_value::finding;
use crate::selection::select_objects;

/// Requires an object to be part of a whole whose class is one of `classes`
/// or matches `class_patterns`, and, if given, whose predefined type is one
/// of `predefined_types` or matches `predefined_patterns`.
///
/// `relation` is `aggregation`, `grouping`, `containment`, `nesting`,
/// `voiding` or `any` (the default). The source lists the object's wholes
/// nearest first; the first whole of a required class decides, and its
/// predefined type must then match. With `prohibited`, meeting the
/// requirement is the violation.
pub struct PartOfRequirement;
impl RuleCapability for PartOfRequirement {
    fn selectable(&self) -> bool {
        true
    }

    fn id(&self) -> &'static str {
        "axioval:capability.part-of"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("relation", ParameterType::String),
            ParameterDescriptor::optional("classes", ParameterType::StringList),
            ParameterDescriptor::optional("class_patterns", ParameterType::StringList),
            ParameterDescriptor::optional("predefined_types", ParameterType::StringList),
            ParameterDescriptor::optional("predefined_patterns", ParameterType::StringList),
            ParameterDescriptor::optional("prohibited", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let relation = match rule.parameters.get("relation") {
            None => Ok(Decomposition::Any),
            Some(ParameterValue::String { value }) => match value.as_str() {
                "aggregation" => Ok(Decomposition::Aggregation),
                "grouping" => Ok(Decomposition::Grouping),
                "containment" => Ok(Decomposition::Containment),
                "nesting" => Ok(Decomposition::Nesting),
                "voiding" => Ok(Decomposition::Voiding),
                "any" => Ok(Decomposition::Any),
                other => Err(format!("unknown relation {other:?}")),
            },
            Some(_) => Err("relation is not a string".to_owned()),
        };
        let matchers = relation.and_then(|relation| {
            let class = Matcher::read(rule, "classes", "class_patterns")?;
            let predefined = Matcher::read(rule, "predefined_types", "predefined_patterns")?;
            if class.is_none() {
                return Err("no class is required".to_owned());
            }
            Ok((relation, class, predefined))
        });
        let (relation, class, predefined) = match matchers {
            Ok(matchers) => matchers,
            Err(message) => {
                return CapabilityEvaluation::not_evaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    format!("part-of parameters are invalid: {message}"),
                );
            }
        };
        let prohibited = matches!(
            rule.parameters.get("prohibited"),
            Some(ParameterValue::Boolean { value: true })
        );
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<DecompositionServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "decomposition service is not registered",
                );
            }
            return evaluation;
        };
        for object in selected {
            let resolved = match service.wholes(&object.id, relation) {
                Ok(resolved) => resolved,
                Err(error) => {
                    let reason = match error {
                        DecompositionError::UncoveredSource(_) => {
                            NotEvaluatedReason::MissingService
                        }
                        DecompositionError::Ambiguous(_) => NotEvaluatedReason::IncompleteEvidence,
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
            let whole = resolved
                .wholes
                .iter()
                .find(|whole| class.matches(&whole.class));
            let met = whole.is_some_and(|whole| {
                predefined.is_none()
                    || whole
                        .predefined_type
                        .as_deref()
                        .is_some_and(|value| predefined.matches(value))
            });
            let violation = if prohibited {
                met.then(|| "the object is part of a prohibited whole".to_owned())
            } else if met {
                None
            } else if resolved.wholes.is_empty() {
                Some(format!(
                    "the object is part of nothing through {relation:?}"
                ))
            } else {
                let classes: Vec<&str> = resolved
                    .wholes
                    .iter()
                    .map(|whole| whole.class.as_str())
                    .collect();
                Some(format!(
                    "no whole meets the requirement (wholes: {})",
                    classes.join(", ")
                ))
            };
            if let Some(message) = violation {
                evaluation.push_finding(finding(rule, object, message, vec![resolved.evidence]));
            }
        }
        evaluation
    }
}
