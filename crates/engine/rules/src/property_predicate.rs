//! A declared predicate over one exactly resolved property value.

use axioval_engine::comparison::{self as shared, Order, Pattern, TextOptions};
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext,
};
use axioval_ir::{PropertyValue, QuantityDimension, TemporalPrecision};
use regex::Regex;

use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Tolerance, Unavailable, display, exact_f64, finding, invalid, resolve,
    undefined,
};

/// Checks one property of each selected object against a declared predicate.
///
/// The target is exactly one of `value` (integer), `number`, `quantity`
/// (a value with a unit such as `mm` or `m2`, compared in SI), `text`, `texts`
/// (a list for `one_of`/`none_of`), `boolean`, `date` or `date_time`;
/// `is_defined` and `is_undefined` take none. Text comparisons are
/// case-sensitive unless `case_sensitive` is `false`; `matches` is a regular
/// expression that must match the whole value.
///
/// A `date` or `date_time` target takes the ordered operators and compares
/// chronologically: dates by day, as XML Schema orders them, date-times as
/// instants whatever their UTC offsets. A date stating a time zone equals no
/// date stating none, and within 14 hours of one it is neither before nor
/// after it, so an order there is not evaluated. `precision` `day` reads
/// every date-time and date as the calendar day it states, so a date-time
/// value compares with a `date` target and the reverse; without it that
/// pair is not evaluated. `precision` on any other
/// target is an invalid declaration.
///
/// A comparison presupposes a value: an exactly absent property fails every
/// operator except `is_undefined`, and a value of another type than the
/// target fails too. A quantity is compared only with a `quantity` target of
/// the same dimension, never with a bare number; otherwise the object is not
/// evaluated.
///
/// A numeric target may declare `tolerance`, `relative_tolerance` or
/// `decimals`: a value within the tolerance of the target, or rounding to the
/// same number, is equal to it, and only a value beyond the tolerance is
/// greater or less. A tolerance on a text, text list or boolean target is an
/// invalid declaration.
pub struct PropertyPredicate;

enum Predicate {
    Integer(Order, i64),
    Number(Order, f64),
    Quantity(Order, f64, QuantityDimension),
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
    /// A date or date-time target, with the declared precision.
    Temporal(Order, PropertyValue, Option<TemporalPrecision>),
    Defined(bool),
}

impl Predicate {
    fn numeric(&self) -> bool {
        matches!(
            self,
            Self::Integer(..) | Self::Number(..) | Self::Quantity(..)
        )
    }

    #[allow(clippy::too_many_lines)]
    fn parse(parameters: &Parameters<'_>) -> Result<Self, Unavailable> {
        let operator = parameters.required_string("operator")?;
        let fold = !parameters.boolean("case_sensitive")?.unwrap_or(true);
        let targets = [
            parameters.integer("value")?.is_some(),
            parameters.number("number")?.is_some(),
            parameters.quantity("quantity")?.is_some(),
            parameters.string("text")?.is_some(),
            parameters.strings("texts")?.is_some(),
            parameters.boolean("boolean")?.is_some(),
            parameters.date("date")?.is_some(),
            parameters.date_time("date_time")?.is_some(),
        ]
        .into_iter()
        .filter(|given| *given)
        .count();
        let order = match operator {
            "equal" => Some(Order::Equal),
            "not_equal" => Some(Order::NotEqual),
            "greater_than" => Some(Order::Greater),
            "greater_or_equal" => Some(Order::GreaterOrEqual),
            "less_than" => Some(Order::Less),
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
        if let Some((value, dimension)) = parameters.quantity("quantity")? {
            return order
                .map(|order| Self::Quantity(order, value, dimension))
                .ok_or_else(|| {
                    invalid(format!(
                        "operator `{operator}` does not apply to a quantity"
                    ))
                });
        }
        let precision = parameters.precision()?;
        let temporal = parameters
            .date("date")?
            .map(PropertyValue::Date)
            .or(parameters
                .date_time("date_time")?
                .map(PropertyValue::DateTime));
        if let Some(value) = temporal {
            return order
                .map(|order| Self::Temporal(order, value, precision))
                .ok_or_else(|| invalid(format!("operator `{operator}` does not apply to a date")));
        }
        if precision.is_some() {
            return Err(invalid(
                "`precision` applies to a date or date_time target only",
            ));
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
                "matches" => shared::pattern(Pattern::Matches, text, !fold)
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
    fn holds(&self, actual: Option<&PropertyValue>, tolerance: &Tolerance) -> Result<bool, String> {
        if let Self::Defined(defined) = self {
            return Ok(undefined(actual) != *defined);
        }
        let Some(actual) = actual else {
            return Ok(false);
        };
        if let Self::Temporal(order, expected, precision) = self {
            // A value of another type fails, as for every other target.
            return shared::temporal(*order, actual, expected, *precision).unwrap_or(Ok(false));
        }
        match (self, actual) {
            (
                Self::Quantity(order, expected, dimension),
                PropertyValue::Quantity {
                    value,
                    dimension: held,
                },
            ) => {
                return if held == dimension {
                    Ok(compare(*order, *value, *expected, tolerance))
                } else {
                    Err(format!(
                        "a quantity in {} cannot be compared with one in {}",
                        held.unit_symbol(),
                        dimension.unit_symbol()
                    ))
                };
            }
            (Self::Quantity(..), PropertyValue::Integer(_) | PropertyValue::Decimal(_)) => {
                return Err("a unit-less number cannot be compared with a quantity".into());
            }
            (_, PropertyValue::Quantity { .. }) => {
                return Err("a quantity cannot be compared with a unit-less target".into());
            }
            _ => {}
        }
        let equality = |equal: bool| if equal { Order::Equal } else { Order::NotEqual };
        Ok(match (self, actual) {
            (Self::Integer(order, expected), PropertyValue::Integer(value))
                if tolerance.is_exact() =>
            {
                shared::integers(*order, *value, *expected)
            }
            (Self::Integer(order, expected), PropertyValue::Integer(value)) => {
                match (exact_f64(*value), exact_f64(*expected)) {
                    (Some(value), Some(expected)) => compare(*order, value, expected, tolerance),
                    _ => {
                        return Err(
                            "an integer beyond 2^53 cannot be compared under a tolerance".into(),
                        );
                    }
                }
            }
            (Self::Integer(order, expected), PropertyValue::Decimal(value)) => exact_f64(*expected)
                .is_some_and(|expected| compare(*order, *value, expected, tolerance)),
            (Self::Number(order, expected), PropertyValue::Decimal(value)) => {
                compare(*order, *value, *expected, tolerance)
            }
            (Self::Number(order, expected), PropertyValue::Integer(value)) => {
                exact_f64(*value).is_some_and(|value| compare(*order, value, *expected, tolerance))
            }
            (
                Self::Text {
                    equal,
                    text,
                    fold: f,
                },
                PropertyValue::String(value),
            ) => shared::texts(equality(*equal), value, text, TextOptions::case(!*f)),
            (Self::Contains { text, fold: f }, PropertyValue::String(value)) => {
                shared::contains(value, text, TextOptions::case(!*f))
            }
            (Self::Matches(pattern), PropertyValue::String(value)) => pattern.is_match(value),
            (
                Self::OneOf {
                    none,
                    texts,
                    fold: f,
                },
                PropertyValue::String(value),
            ) => shared::member(value, texts, TextOptions::case(!*f)) != *none,
            (Self::Boolean { equal, value }, PropertyValue::Boolean(actual)) => {
                shared::booleans(equality(*equal), *actual, *value) == Some(true)
            }
            _ => false,
        })
    }
}

/// `left order right` through the shared comparison; a number that is no
/// number fails.
fn compare(order: Order, left: f64, right: f64, tolerance: &Tolerance) -> bool {
    shared::numbers(order, (left, left), (right, right), tolerance) == Ok(true)
}

fn target(parameters: &Parameters<'_>) -> String {
    let rule = parameters.0;
    [
        "value",
        "number",
        "quantity",
        "text",
        "texts",
        "boolean",
        "date",
        "date_time",
    ]
    .iter()
    .find_map(|name| rule.parameters.get(*name))
    .map_or_else(String::new, |value| match value {
        axioval_ir::contract::ParameterValue::Integer { value } => format!(" {value}"),
        axioval_ir::contract::ParameterValue::Number { value } => format!(" {value}"),
        axioval_ir::contract::ParameterValue::Quantity { value, unit } => {
            format!(" {value} {unit}")
        }
        axioval_ir::contract::ParameterValue::String { value } => format!(" `{value}`"),
        axioval_ir::contract::ParameterValue::StringList { value } => {
            format!(" [{}]", value.join(", "))
        }
        axioval_ir::contract::ParameterValue::Boolean { value } => format!(" {value}"),
        axioval_ir::contract::ParameterValue::Date { value } => format!(" {value}"),
        axioval_ir::contract::ParameterValue::DateTime { value } => format!(" {value}"),
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
            ParameterDescriptor::optional("value", ParameterType::Integer).per_object(),
            ParameterDescriptor::optional("number", ParameterType::Number).per_object(),
            ParameterDescriptor::optional("quantity", ParameterType::Quantity).per_object(),
            ParameterDescriptor::optional("text", ParameterType::String).per_object(),
            ParameterDescriptor::optional("texts", ParameterType::StringList),
            ParameterDescriptor::optional("boolean", ParameterType::Boolean).per_object(),
            ParameterDescriptor::optional("date", ParameterType::Date).per_object(),
            ParameterDescriptor::optional("date_time", ParameterType::DateTime).per_object(),
            ParameterDescriptor::optional("precision", ParameterType::String),
            ParameterDescriptor::optional("case_sensitive", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(crate::support::tolerance_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        let parameters = Parameters(rule);
        let parsed = (|| {
            let property = PropertyRef {
                set: Some(parameters.required_string("property_set")?),
                name: parameters.required_string("property")?,
            };
            let predicate = Predicate::parse(&parameters)?;
            let tolerance = parameters.tolerance()?;
            if !tolerance.is_exact() && !predicate.numeric() {
                return Err(invalid("a tolerance applies to a numeric target only"));
            }
            Ok::<_, Unavailable>((property, predicate, tolerance))
        })();
        let (property, predicate, tolerance) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("property-predicate: {message}"),
                );
            }
        };
        let operator = parameters.required_string("operator").unwrap_or("invalid");
        let target = format!("{}{}", target(&parameters), tolerance.suffix());
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        for object in selected {
            let resolved = match resolve(context, object, property) {
                Ok(resolved) => resolved,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            match predicate.holds(resolved.value(), &tolerance) {
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
