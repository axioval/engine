//! A declared predicate over one exactly resolved property value.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::PropertyValue;
use regex::{Regex, RegexBuilder};

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Unavailable, display, finding, invalid, resolve, undefined,
};

/// Checks one property of each selected object against a declared predicate.
///
/// The target is exactly one of `value` (integer), `number`, `text`, `texts`
/// (a list for `one_of`/`none_of`) or `boolean`; `is_defined` and
/// `is_undefined` take none. Text comparisons are case-sensitive unless
/// `case_sensitive` is `false`; `matches` is a regular expression that must
/// match the whole value.
///
/// A comparison presupposes a value: an exactly absent property fails every
/// operator except `is_undefined`, and a value of another type than the
/// target fails too. A quantity is never compared with a bare number, since
/// the declaration states no unit; that object is not evaluated.
pub struct PropertyPredicate;

#[derive(Clone, Copy, Debug)]
enum Order {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
}

impl Order {
    fn holds(self, ordering: std::cmp::Ordering) -> bool {
        match self {
            Self::Equal => ordering.is_eq(),
            Self::NotEqual => !ordering.is_eq(),
            Self::GreaterThan => ordering.is_gt(),
            Self::GreaterOrEqual => ordering.is_ge(),
            Self::LessThan => ordering.is_lt(),
            Self::LessOrEqual => ordering.is_le(),
        }
    }
}

enum Predicate {
    Integer(Order, i64),
    Number(Order, f64),
    Text {
        equal: bool,
        text: String,
        fold: bool,
    },
    Contains {
        text: String,
        fold: bool,
    },
    Matches(Regex),
    OneOf {
        none: bool,
        texts: Vec<String>,
        fold: bool,
    },
    Boolean {
        equal: bool,
        value: bool,
    },
    Defined(bool),
}

impl Predicate {
    #[allow(clippy::too_many_lines)]
    fn parse(parameters: &Parameters<'_>) -> Result<Self, Unavailable> {
        let operator = parameters.required_string("operator")?;
        let fold = !parameters.boolean("case_sensitive")?.unwrap_or(true);
        let targets = [
            parameters.integer("value")?.is_some(),
            parameters.number("number")?.is_some(),
            parameters.string("text")?.is_some(),
            parameters.strings("texts")?.is_some(),
            parameters.boolean("boolean")?.is_some(),
        ]
        .into_iter()
        .filter(|given| *given)
        .count();
        let order = match operator {
            "equal" => Some(Order::Equal),
            "not_equal" => Some(Order::NotEqual),
            "greater_than" => Some(Order::GreaterThan),
            "greater_or_equal" => Some(Order::GreaterOrEqual),
            "less_than" => Some(Order::LessThan),
            "less_or_equal" => Some(Order::LessOrEqual),
            _ => None,
        };
        let expected = match operator {
            "is_defined" | "is_undefined" => 0,
            _ => 1,
        };
        if targets != expected {
            return Err(invalid(format!(
                "operator `{operator}` takes {expected} target value(s); {targets} given"
            )));
        }
        let fold_text = |text: &str| {
            if fold {
                text.to_lowercase()
            } else {
                text.to_owned()
            }
        };
        if let Some(value) = parameters.integer("value")? {
            return order
                .map(|order| Self::Integer(order, value))
                .ok_or_else(|| {
                    invalid(format!(
                        "operator `{operator}` does not apply to an integer"
                    ))
                });
        }
        if let Some(value) = parameters.number("number")? {
            return order
                .map(|order| Self::Number(order, value))
                .ok_or_else(|| {
                    invalid(format!("operator `{operator}` does not apply to a number"))
                });
        }
        if let Some(value) = parameters.boolean("boolean")? {
            return match operator {
                "equal" | "not_equal" => Ok(Self::Boolean {
                    equal: operator == "equal",
                    value,
                }),
                _ => Err(invalid(format!(
                    "operator `{operator}` does not apply to a boolean"
                ))),
            };
        }
        if let Some(texts) = parameters.strings("texts")? {
            return match operator {
                "one_of" | "none_of" => Ok(Self::OneOf {
                    none: operator == "none_of",
                    texts: texts.iter().map(|text| fold_text(text)).collect(),
                    fold,
                }),
                _ => Err(invalid(format!(
                    "operator `{operator}` does not apply to a text list"
                ))),
            };
        }
        if let Some(text) = parameters.string("text")? {
            return match operator {
                "equal" | "not_equal" => Ok(Self::Text {
                    equal: operator == "equal",
                    text: fold_text(text),
                    fold,
                }),
                "contains" => Ok(Self::Contains {
                    text: fold_text(text),
                    fold,
                }),
                "matches" => RegexBuilder::new(&format!("^(?:{text})$"))
                    .case_insensitive(fold)
                    .build()
                    .map(Self::Matches)
                    .map_err(|error| invalid(format!("invalid regular expression: {error}"))),
                _ => Err(invalid(format!(
                    "operator `{operator}` does not apply to text"
                ))),
            };
        }
        match operator {
            "is_defined" => Ok(Self::Defined(true)),
            "is_undefined" => Ok(Self::Defined(false)),
            _ => Err(invalid(format!("operator `{operator}` is unsupported"))),
        }
    }

    /// Whether `actual` satisfies the predicate; `Err` when it cannot be judged.
    fn holds(&self, actual: Option<&PropertyValue>) -> Result<bool, String> {
        if let Self::Defined(defined) = self {
            return Ok(undefined(actual) != *defined);
        }
        let Some(actual) = actual else {
            return Ok(false);
        };
        if matches!(actual, PropertyValue::Quantity { .. }) {
            return Err("a quantity cannot be compared with a unit-less target".into());
        }
        let fold = |text: &str, fold: bool| {
            if fold {
                text.to_lowercase()
            } else {
                text.to_owned()
            }
        };
        Ok(match (self, actual) {
            (Self::Integer(order, expected), PropertyValue::Integer(value)) => {
                order.holds(value.cmp(expected))
            }
            (Self::Integer(order, expected), PropertyValue::Decimal(value)) => {
                exact_f64(*expected).is_some_and(|expected| compare(*order, *value, expected))
            }
            (Self::Number(order, expected), PropertyValue::Decimal(value)) => {
                compare(*order, *value, *expected)
            }
            (Self::Number(order, expected), PropertyValue::Integer(value)) => {
                exact_f64(*value).is_some_and(|value| compare(*order, value, *expected))
            }
            (
                Self::Text {
                    equal,
                    text,
                    fold: f,
                },
                PropertyValue::String(value),
            ) => (fold(value, *f) == *text) == *equal,
            (Self::Contains { text, fold: f }, PropertyValue::String(value)) => {
                fold(value, *f).contains(text.as_str())
            }
            (Self::Matches(pattern), PropertyValue::String(value)) => pattern.is_match(value),
            (
                Self::OneOf {
                    none,
                    texts,
                    fold: f,
                },
                PropertyValue::String(value),
            ) => texts.contains(&fold(value, *f)) != *none,
            (Self::Boolean { equal, value }, PropertyValue::Boolean(actual)) => {
                (actual == value) == *equal
            }
            _ => false,
        })
    }
}

fn compare(order: Order, left: f64, right: f64) -> bool {
    left.partial_cmp(&right)
        .is_some_and(|ordering| order.holds(ordering))
}

fn exact_f64(value: i64) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    (value.unsigned_abs() <= 1 << 53).then_some(value as f64)
}

fn target(parameters: &Parameters<'_>) -> String {
    let rule = parameters.0;
    ["value", "number", "text", "texts", "boolean"]
        .iter()
        .find_map(|name| rule.parameters.get(*name))
        .map_or_else(String::new, |value| match value {
            axioval_ir::contract::ParameterValue::Integer { value } => format!(" {value}"),
            axioval_ir::contract::ParameterValue::Number { value } => format!(" {value}"),
            axioval_ir::contract::ParameterValue::String { value } => format!(" `{value}`"),
            axioval_ir::contract::ParameterValue::StringList { value } => {
                format!(" [{}]", value.join(", "))
            }
            axioval_ir::contract::ParameterValue::Boolean { value } => format!(" {value}"),
            _ => String::new(),
        })
}

impl RuleCapability for PropertyPredicate {
    fn id(&self) -> &'static str {
        "axioval:capability.property-predicate"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("property_set", ParameterType::String),
            ParameterDescriptor::required("property", ParameterType::String),
            ParameterDescriptor::required("operator", ParameterType::String),
            ParameterDescriptor::optional("value", ParameterType::Integer),
            ParameterDescriptor::optional("number", ParameterType::Number),
            ParameterDescriptor::optional("text", ParameterType::String),
            ParameterDescriptor::optional("texts", ParameterType::StringList),
            ParameterDescriptor::optional("boolean", ParameterType::Boolean),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let property = PropertyRef {
                set: Some(parameters.required_string("property_set")?),
                name: parameters.required_string("property")?,
            };
            Ok::<_, Unavailable>((property, Predicate::parse(&parameters)?))
        })();
        let (property, predicate) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("property-predicate: {message}"),
                );
            }
        };
        let operator = parameters.required_string("operator").unwrap_or("invalid");
        let target = target(&parameters);
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let resolved = match resolve(context, object, property) {
                Ok(resolved) => resolved,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            match predicate.holds(resolved.value()) {
                Ok(true) => {}
                Ok(false) => evaluation.push_finding(finding(
                    rule,
                    &object.id,
                    format!(
                        "property {property} does not satisfy {operator}{target}; actual value is {}",
                        display(resolved.value())
                    ),
                    resolved.evidence(),
                    vec![],
                )),
                Err(message) => evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    axioval_ir::NotEvaluatedReason::InvalidEvidence,
                    message,
                ),
            }
        }
        evaluation
    }
}
