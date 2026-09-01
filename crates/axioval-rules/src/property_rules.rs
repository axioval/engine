//! Trusted property capabilities over exact host-provided resolutions.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PropertyRequest, PropertyResolution, PropertyResolutionServiceHandle, RuleCapability,
    RuleContext,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, Finding, Object, PropertyValue, Severity};

use crate::selection::{property_error, select_objects};

fn property_reference<'a>(
    rule: &'a CompiledRule,
    name: &str,
) -> Option<(Option<&'a str>, &'a str)> {
    match rule.parameters.get(name)? {
        ParameterValue::PropertyReference {
            property,
            property_set,
        } => Some((property_set.as_deref(), property)),
        _ => None,
    }
}

fn string<'a>(rule: &'a CompiledRule, name: &str) -> Option<&'a str> {
    match rule.parameters.get(name)? {
        ParameterValue::String { value }
        | ParameterValue::Enum { value }
        | ParameterValue::Reference { value } => Some(value),
        _ => None,
    }
}

fn integer(rule: &CompiledRule, name: &str) -> Option<i64> {
    match rule.parameters.get(name)? {
        ParameterValue::Integer { value } => Some(*value),
        _ => None,
    }
}

fn finding(
    rule: &CompiledRule,
    object: &Object,
    message: String,
    evidence: Vec<Evidence>,
) -> Finding {
    Finding {
        rule_id: rule.id.clone(),
        object_id: object.id.clone(),
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence,
    }
}

fn unavailable_selected(
    selected: &[&Object],
    reason: &NotEvaluatedReason,
    message: &str,
    mut evaluation: CapabilityEvaluation,
) -> CapabilityEvaluation {
    for object in selected {
        evaluation.push_object_not_evaluated(object.id.clone(), reason.clone(), message);
    }
    evaluation
}

fn resolve_error(
    evaluation: &mut CapabilityEvaluation,
    object: &Object,
    error: axioval_engine::PropertyResolutionError,
) {
    let (reason, message) = property_error(error);
    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
}

/// Requires an exactly resolved property to be present.
pub struct PropertyExists;
impl RuleCapability for PropertyExists {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some((set, name)) = property_reference(rule, "property") else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-exists has no valid property reference",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
            return unavailable_selected(
                &selected,
                &NotEvaluatedReason::MissingService,
                "property-resolution service is not registered",
                evaluation,
            );
        };
        for object in selected {
            let request =
                match PropertyRequest::try_new(object.id.clone(), set.map(ToOwned::to_owned), name)
                {
                    Ok(request) => request,
                    Err(error) => {
                        evaluation.push_not_evaluated(
                            NotEvaluatedReason::InvalidDeclaration,
                            error.to_string(),
                        );
                        return evaluation;
                    }
                };
            match service.resolve(&request) {
                Ok(PropertyResolution::Present(_)) => {}
                Ok(PropertyResolution::Absent(proof)) => evaluation.push_finding(finding(
                    rule,
                    object,
                    format!("missing exact property {name}"),
                    vec![proof.evidence().clone()],
                )),
                Err(error) => resolve_error(&mut evaluation, object, error),
            }
        }
        evaluation
    }
}

/// Requires an exactly resolved property to contain a non-empty semantic value.
///
/// Exact absence, `null`, and blank text are violations. Other exact typed
/// values satisfy the requirement; adapter failures remain not-evaluated.
pub struct PropertyRequired;
impl RuleCapability for PropertyRequired {
    fn id(&self) -> &'static str {
        "axioval:capability.property-required"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let Some((set, name)) = property_reference(rule, "property") else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-required has no valid property reference",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
            return unavailable_selected(
                &selected,
                &NotEvaluatedReason::MissingService,
                "property-resolution service is not registered",
                evaluation,
            );
        };
        for object in selected {
            let request =
                match PropertyRequest::try_new(object.id.clone(), set.map(ToOwned::to_owned), name)
                {
                    Ok(request) => request,
                    Err(error) => {
                        evaluation.push_not_evaluated(
                            NotEvaluatedReason::InvalidDeclaration,
                            error.to_string(),
                        );
                        return evaluation;
                    }
                };
            match service.resolve(&request) {
                Ok(PropertyResolution::Present(resolved)) => {
                    let value = &resolved.property().value;
                    if matches!(value, PropertyValue::Null)
                        || matches!(value, PropertyValue::String(text) if text.trim().is_empty())
                    {
                        evaluation.push_finding(finding(
                            rule,
                            object,
                            format!("missing required property {name}"),
                            resolved.property().evidence.clone().into_iter().collect(),
                        ));
                    }
                }
                Ok(PropertyResolution::Absent(proof)) => evaluation.push_finding(finding(
                    rule,
                    object,
                    format!("missing required property {name}"),
                    vec![proof.evidence().clone()],
                )),
                Err(error) => resolve_error(&mut evaluation, object, error),
            }
        }
        evaluation
    }
}

fn boolean(rule: &CompiledRule, name: &str) -> Option<bool> {
    match rule.parameters.get(name)? {
        ParameterValue::Boolean { value } => Some(*value),
        _ => None,
    }
}

/// Compares an exactly resolved boolean property with a declarative value.
pub struct BooleanPropertyEquals;
impl RuleCapability for BooleanPropertyEquals {
    fn id(&self) -> &'static str {
        "axioval:capability.property-value-equals"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("property", ParameterType::PropertyReference),
            ParameterDescriptor::required("expected", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (Some((set, name)), Some(expected)) = (
            property_reference(rule, "property"),
            boolean(rule, "expected"),
        ) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-value-equals parameters are invalid",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
            return unavailable_selected(
                &selected,
                &NotEvaluatedReason::MissingService,
                "property-resolution service is not registered",
                evaluation,
            );
        };
        for object in selected {
            let request =
                match PropertyRequest::try_new(object.id.clone(), set.map(ToOwned::to_owned), name)
                {
                    Ok(request) => request,
                    Err(error) => {
                        evaluation.push_not_evaluated(
                            NotEvaluatedReason::InvalidDeclaration,
                            error.to_string(),
                        );
                        return evaluation;
                    }
                };
            match service.resolve(&request) {
                Ok(PropertyResolution::Present(resolved)) => {
                    let property = resolved.property();
                    match &property.value {
                        PropertyValue::Boolean(actual) if *actual == expected => {}
                        PropertyValue::Boolean(_) => evaluation.push_finding(finding(
                            rule,
                            object,
                            format!("property {name} does not equal {expected}"),
                            property.evidence.clone().into_iter().collect(),
                        )),
                        _ => evaluation.push_object_not_evaluated(
                            object.id.clone(),
                            NotEvaluatedReason::InvalidEvidence,
                            format!("property {name} is not boolean"),
                        ),
                    }
                }
                Ok(PropertyResolution::Absent(proof)) => evaluation.push_finding(finding(
                    rule,
                    object,
                    format!("property {name} does not equal {expected}"),
                    vec![proof.evidence().clone()],
                )),
                Err(error) => resolve_error(&mut evaluation, object, error),
            }
        }
        evaluation
    }
}

#[derive(Clone, Copy)]
enum IntegerOperator {
    Equal,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
}
impl IntegerOperator {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "equal" => Some(Self::Equal),
            "greater_than" => Some(Self::GreaterThan),
            "greater_or_equal" => Some(Self::GreaterOrEqual),
            "less_than" => Some(Self::LessThan),
            "less_or_equal" => Some(Self::LessOrEqual),
            _ => None,
        }
    }
    fn passes(self, actual: i64, expected: i64) -> bool {
        match self {
            Self::Equal => actual == expected,
            Self::GreaterThan => actual > expected,
            Self::GreaterOrEqual => actual >= expected,
            Self::LessThan => actual < expected,
            Self::LessOrEqual => actual <= expected,
        }
    }
}

/// Compares an exact integer property with a declarative integer literal.
pub struct PropertyPredicate;
impl RuleCapability for PropertyPredicate {
    fn id(&self) -> &'static str {
        "axioval:capability.property-predicate"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("property_set", ParameterType::String),
            ParameterDescriptor::required("property", ParameterType::String),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::required("value", ParameterType::Integer),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (Some(set), Some(name), Some(operator), Some(expected)) = (
            string(rule, "property_set"),
            string(rule, "property"),
            string(rule, "operator"),
            integer(rule, "value"),
        ) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-predicate parameters are invalid",
            );
        };
        let Some(operator) = IntegerOperator::parse(operator) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "property-predicate operator is unsupported",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
            return unavailable_selected(
                &selected,
                &NotEvaluatedReason::MissingService,
                "property-resolution service is not registered",
                evaluation,
            );
        };
        for object in selected {
            let request =
                match PropertyRequest::try_new(object.id.clone(), Some(set.to_owned()), name) {
                    Ok(request) => request,
                    Err(error) => {
                        evaluation.push_not_evaluated(
                            NotEvaluatedReason::InvalidDeclaration,
                            error.to_string(),
                        );
                        return evaluation;
                    }
                };
            match service.resolve(&request) {
                Ok(PropertyResolution::Present(resolved)) => {
                    let property = resolved.property();
                    let actual = match &property.value {
                        PropertyValue::Integer(value) => Some(*value),
                        _ => None,
                    };
                    if !actual.is_some_and(|actual| operator.passes(actual, expected)) {
                        evaluation.push_finding(finding(
                            rule,
                            object,
                            format!(
                                "property {set}.{name} does not satisfy {} {expected}",
                                string(rule, "operator").unwrap_or("invalid")
                            ),
                            property.evidence.clone().into_iter().collect(),
                        ));
                    }
                }
                Ok(PropertyResolution::Absent(proof)) => evaluation.push_finding(finding(
                    rule,
                    object,
                    format!(
                        "property {set}.{name} does not satisfy {} {expected}",
                        string(rule, "operator").unwrap_or("invalid")
                    ),
                    vec![proof.evidence().clone()],
                )),
                Err(error) => resolve_error(&mut evaluation, object, error),
            }
        }
        evaluation
    }
}
