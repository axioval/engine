//! Built-in trusted source-neutral property capabilities.
#![forbid(unsafe_code)]

use axioval_engine::{
    CapabilityRegistry, CompiledRule, EngineError, ParameterDescriptor, ParameterType,
    RuleCapability, RuleContext,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Finding, Object, Property, PropertyValue, RuleId, Severity};
use regex::Regex;

/// Registers all maintained built-in capabilities into a host registry.
///
/// # Errors
///
/// Returns an error if the registry already contains a built-in capability ID.
pub fn register_builtins(registry: CapabilityRegistry) -> Result<CapabilityRegistry, EngineError> {
    registry
        .register(PropertyExists)
        .and_then(|registry| registry.register(PropertyPredicate))
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
fn selector_matches(selector: &Selector, object: &Object) -> bool {
    match selector {
        Selector::All => true,
        Selector::EntityType { object_type, .. } => object.kind() == object_type,
        Selector::Classification { system, code, .. } => object
            .classifications
            .iter()
            .any(|item| item.system == *system && item.code == *code),
        Selector::AllOf { operands } => operands.iter().all(|item| selector_matches(item, object)),
        Selector::AnyOf { operands } => operands.iter().any(|item| selector_matches(item, object)),
        Selector::Not { operand } => !selector_matches(operand, object),
        Selector::Property {
            property_set,
            property,
            operator,
            value,
        } => property_selector_matches(
            object,
            property_set.as_deref(),
            property,
            operator,
            value.as_ref(),
        ),
    }
}

fn property_selector_matches(
    object: &Object,
    set: Option<&str>,
    name: &str,
    operator: &ComparisonOperator,
    expected: Option<&ParameterValue>,
) -> bool {
    let property = match set {
        Some(set) => object.property(set, name),
        None => object.properties.iter().find(|item| item.name == name),
    };
    let exact = property
        .and_then(|item| item.evidence.as_ref())
        .is_some_and(|evidence| evidence.exact);
    if matches!(operator, ComparisonOperator::Exists) {
        return exact;
    }
    let (Some(property), Some(expected)) = (property.filter(|_| exact), expected) else {
        return false;
    };
    compare_property(property, operator, expected)
}

fn compare_property(
    property: &Property,
    operator: &ComparisonOperator,
    expected: &ParameterValue,
) -> bool {
    match operator {
        ComparisonOperator::Equals => {
            values_equal(&property.value, expected).is_some_and(|equal| equal)
        }
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
            (PropertyValue::String(actual), ParameterValue::String { value }) => {
                Regex::new(value).is_ok_and(|pattern| pattern.is_match(actual))
            }
            _ => false,
        },
        ComparisonOperator::Exists => true,
    }
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

fn finding(rule: &CompiledRule, object: &Object, message: String) -> Finding {
    Finding {
        rule_id: RuleId::new(rule.id.clone()).expect("compiled rule IDs are validated"),
        object_id: object.id.clone(),
        severity: match rule.severity {
            axioval_ir::contract::Severity::Error => Severity::Error,
            axioval_ir::contract::Severity::Warning => Severity::Warning,
            axioval_ir::contract::Severity::Info => Severity::Info,
        },
        message,
        evidence: vec![],
    }
}

/// Requires an exact-evidence property to be present.
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
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> Vec<Finding> {
        let Some((set, name)) = property_reference(rule, "property") else {
            return vec![];
        };
        context
            .project
            .objects()
            .filter(|object| selector_matches(&rule.selector, object))
            .filter_map(|object| {
                let property = match set {
                    Some(set) => object.property(set, name),
                    None => object.properties.iter().find(|item| item.name == name),
                };
                (property.is_none()
                    || property
                        .and_then(|item| item.evidence.as_ref())
                        .is_none_or(|evidence| !evidence.exact))
                .then(|| finding(rule, object, format!("missing exact property {name}")))
            })
            .collect()
    }
}

/// Compares an exact-evidence integer property with a declarative integer literal.
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
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> Vec<Finding> {
        let (Some(set), Some(name), Some(operator), Some(expected)) = (
            string(rule, "property_set"),
            string(rule, "property"),
            string(rule, "operator"),
            integer(rule, "value"),
        ) else {
            return vec![];
        };
        context
            .project
            .objects()
            .filter(|object| selector_matches(&rule.selector, object))
            .filter_map(|object| {
                let actual = object
                    .property(set, name)
                    .filter(|p| p.evidence.as_ref().is_some_and(|e| e.exact))
                    .and_then(|p| match p.value {
                        PropertyValue::Integer(value) => Some(value),
                        _ => None,
                    });
                let passes = actual.is_some_and(|actual| match operator {
                    "equal" => actual == expected,
                    "greater_than" => actual > expected,
                    "greater_or_equal" => actual >= expected,
                    "less_than" => actual < expected,
                    "less_or_equal" => actual <= expected,
                    _ => false,
                });
                (!passes).then(|| {
                    finding(
                        rule,
                        object,
                        format!("property {set}.{name} does not satisfy {operator} {expected}"),
                    )
                })
            })
            .collect()
    }
}
