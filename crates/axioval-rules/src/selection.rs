//! Deterministic, fail-closed selector evaluation.

use axioval_engine::{
    CapabilityEvaluation, NotEvaluatedReason, PropertyRequest, PropertyResolution,
    PropertyResolutionError, PropertyResolutionServiceHandle, RuleContext,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Object, Property, PropertyValue};
use regex::Regex;

pub(crate) fn select_objects<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, CapabilityEvaluation) {
    let mut selected = Vec::new();
    let mut evaluation = CapabilityEvaluation::default();
    for object in context.project.objects() {
        match selector_matches(context, selector, object) {
            Selection::Match => selected.push(object),
            Selection::NoMatch => {}
            Selection::NotEvaluated(reason, message) => {
                evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
            }
        }
    }
    (selected, evaluation)
}

#[derive(Clone, Debug)]
enum Selection {
    Match,
    NoMatch,
    NotEvaluated(NotEvaluatedReason, String),
}

fn selector_matches(context: &RuleContext<'_>, selector: &Selector, object: &Object) -> Selection {
    match selector {
        Selector::All => Selection::Match,
        Selector::EntityType { object_type, .. } => verdict(object.kind() == object_type),
        Selector::Classification { system, code, .. } => verdict(
            object
                .classifications
                .iter()
                .any(|item| item.system == *system && item.code == *code),
        ),
        Selector::AllOf { operands } => all_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object)),
        ),
        Selector::AnyOf { operands } => any_of(
            operands
                .iter()
                .map(|item| selector_matches(context, item, object)),
        ),
        Selector::Not { operand } => match selector_matches(context, operand, object) {
            Selection::Match => Selection::NoMatch,
            Selection::NoMatch => Selection::Match,
            unavailable @ Selection::NotEvaluated(..) => unavailable,
        },
        Selector::Property {
            property_set,
            property,
            operator,
            value,
        } => property_selector_matches(
            context,
            object,
            property_set.clone(),
            property,
            operator,
            value.as_ref(),
        ),
    }
}

fn verdict(value: bool) -> Selection {
    if value {
        Selection::Match
    } else {
        Selection::NoMatch
    }
}

fn all_of(items: impl Iterator<Item = Selection>) -> Selection {
    let mut unavailable = None;
    for item in items {
        match item {
            Selection::NoMatch => return Selection::NoMatch,
            Selection::NotEvaluated(_, _) if unavailable.is_none() => unavailable = Some(item),
            _ => {}
        }
    }
    unavailable.unwrap_or(Selection::Match)
}

fn any_of(items: impl Iterator<Item = Selection>) -> Selection {
    let mut unavailable = None;
    for item in items {
        match item {
            Selection::Match => return Selection::Match,
            Selection::NotEvaluated(_, _) if unavailable.is_none() => unavailable = Some(item),
            _ => {}
        }
    }
    unavailable.unwrap_or(Selection::NoMatch)
}

fn property_selector_matches(
    context: &RuleContext<'_>,
    object: &Object,
    set: Option<String>,
    name: &str,
    operator: &ComparisonOperator,
    expected: Option<&ParameterValue>,
) -> Selection {
    if let Some(message) = selector_declaration_error(operator, expected) {
        return Selection::NotEvaluated(NotEvaluatedReason::InvalidDeclaration, message);
    }
    let Some(service) = context.services.get::<PropertyResolutionServiceHandle>() else {
        return Selection::NotEvaluated(
            NotEvaluatedReason::MissingService,
            "property-resolution service is not registered".into(),
        );
    };
    let request = match PropertyRequest::try_new(object.id.clone(), set, name) {
        Ok(request) => request,
        Err(error) => return invalid(&error),
    };
    match service.resolve(&request) {
        Ok(PropertyResolution::Absent(_)) => Selection::NoMatch,
        Ok(PropertyResolution::Present(resolved)) => {
            let property = resolved.property();
            if matches!(operator, ComparisonOperator::Exists) {
                Selection::Match
            } else if let Some(expected) = expected {
                match compare_property(property, operator, expected) {
                    Ok(matches) => verdict(matches),
                    Err(message) => {
                        Selection::NotEvaluated(NotEvaluatedReason::InvalidDeclaration, message)
                    }
                }
            } else {
                Selection::NotEvaluated(
                    NotEvaluatedReason::InvalidDeclaration,
                    "property selector comparison has no expected value".into(),
                )
            }
        }
        Err(error) => unavailable(error),
    }
}

fn selector_declaration_error(
    operator: &ComparisonOperator,
    expected: Option<&ParameterValue>,
) -> Option<String> {
    match (operator, expected) {
        (ComparisonOperator::Exists, Some(_)) => {
            Some("property selector exists operator must not have a value".into())
        }
        (ComparisonOperator::Exists, None)
        | (ComparisonOperator::Matches, Some(ParameterValue::String { .. })) => None,
        (_, None) => Some("property selector comparison has no expected value".into()),
        (ComparisonOperator::Matches, Some(_)) => {
            Some("property selector regex must be a string".into())
        }
        (_, Some(_)) => None,
    }
}

fn invalid(error: &PropertyResolutionError) -> Selection {
    Selection::NotEvaluated(NotEvaluatedReason::InvalidDeclaration, error.to_string())
}

pub(crate) fn property_error(error: PropertyResolutionError) -> (NotEvaluatedReason, String) {
    match error {
        PropertyResolutionError::Unavailable(message) => {
            (NotEvaluatedReason::BackendUnavailable, message)
        }
        error => (NotEvaluatedReason::InvalidEvidence, error.to_string()),
    }
}

fn unavailable(error: PropertyResolutionError) -> Selection {
    let (reason, message) = property_error(error);
    Selection::NotEvaluated(reason, message)
}

fn compare_property(
    property: &Property,
    operator: &ComparisonOperator,
    expected: &ParameterValue,
) -> Result<bool, String> {
    Ok(match operator {
        ComparisonOperator::Equals => values_equal(&property.value, expected).unwrap_or(false),
        ComparisonOperator::NotEquals => {
            values_equal(&property.value, expected).is_some_and(|equal| !equal)
        }
        ComparisonOperator::LessThan => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_lt)
        }
        ComparisonOperator::LessThanOrEquals => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_le)
        }
        ComparisonOperator::GreaterThan => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_gt)
        }
        ComparisonOperator::GreaterThanOrEquals => {
            ordered(&property.value, expected).is_some_and(std::cmp::Ordering::is_ge)
        }
        ComparisonOperator::Matches => match (&property.value, expected) {
            (PropertyValue::String(actual), ParameterValue::String { value }) => Regex::new(value)
                .map_err(|error| format!("invalid property selector regex: {error}"))?
                .is_match(actual),
            _ => false,
        },
        ComparisonOperator::Exists => true,
    })
}

fn values_equal(actual: &PropertyValue, expected: &ParameterValue) -> Option<bool> {
    match (actual, expected) {
        (PropertyValue::Boolean(actual), ParameterValue::Boolean { value }) => {
            Some(actual == value)
        }
        (PropertyValue::Integer(actual), ParameterValue::Integer { value }) => {
            Some(actual == value)
        }
        (PropertyValue::Decimal(actual), ParameterValue::Number { value }) => {
            Some(actual.total_cmp(value).is_eq())
        }
        (
            PropertyValue::String(actual),
            ParameterValue::String { value }
            | ParameterValue::Enum { value }
            | ParameterValue::Reference { value },
        ) => Some(actual == value),
        _ => None,
    }
}

fn ordered(actual: &PropertyValue, expected: &ParameterValue) -> Option<std::cmp::Ordering> {
    match (actual, expected) {
        (PropertyValue::Integer(actual), ParameterValue::Integer { value }) => {
            Some(actual.cmp(value))
        }
        (PropertyValue::Decimal(actual), ParameterValue::Number { value }) => {
            actual.partial_cmp(value)
        }
        (PropertyValue::String(actual), ParameterValue::String { value }) => {
            Some(actual.cmp(value))
        }
        _ => None,
    }
}
