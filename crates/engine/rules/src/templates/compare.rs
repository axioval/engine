//! The generic comparison judge ([`Decision::Compare`]): a rule's operator
//! word and target bound into one test, judged on what the source states
//! through the one comparison every rule uses
//! ([`axioval_engine::comparison`]).
//!
//! [`Decision::Compare`]: axioval_engine::template::Decision::Compare

use axioval_engine::CompiledRule;
use axioval_engine::comparison::{self as shared, Order, Pattern, TextOptions, Tolerance};
use axioval_engine::template::{Comparison, ComparisonTarget, TargetKind, Test};
use axioval_ir::contract::{Expression, ParameterValue, ScalarValue};
use axioval_ir::{PropertyValue, QuantityDimension, TemporalPrecision};
use regex::Regex;

use crate::support::{Parameters, Unavailable, exact_f64, invalid, undefined};

/// A rule's comparison, bound: the test, the declared tolerance and how
/// messages show the target.
pub(super) struct Bound {
    judge: Judge,
    tolerance: Tolerance,
    precision: Option<TemporalPrecision>,
    /// The stated target as the rule declares it, after a space; nothing
    /// for a presence test.
    pub(super) target: String,
}

/// One bound test.
enum Judge {
    /// Whether the value is defined (`true`) or undefined.
    Presence(bool),
    Integer(Order, i64),
    Number(Order, f64),
    Quantity(Order, f64, QuantityDimension),
    Temporal(Order, PropertyValue),
    Boolean(Order, bool),
    /// An equality, or `contains` (`None`), with the text folded as
    /// declared.
    Text(Option<Order>, String, TextOptions),
    /// A whole-value regular expression, with its source.
    Pattern(Regex, String, bool),
    /// One of (or, `none`, none of) texts folded as declared.
    Member {
        texts: Vec<String>,
        none: bool,
        options: TextOptions,
    },
}

/// A stated target's value.
enum Declared {
    Integer(i64),
    Number(f64),
    Quantity(f64, QuantityDimension),
    Date(axioval_ir::Date),
    DateTime(axioval_ir::DateTime),
    Boolean(bool),
    Texts(Vec<String>),
    Text(String),
}

/// The target `target` states in `rule`, read as its kind.
fn declared(
    parameters: &Parameters<'_>,
    target: &ComparisonTarget,
) -> Result<Option<Declared>, Unavailable> {
    let name = target.parameter;
    Ok(match target.kind {
        TargetKind::Integer => parameters.integer(name)?.map(Declared::Integer),
        TargetKind::Number => parameters.number(name)?.map(Declared::Number),
        TargetKind::Quantity => parameters
            .quantity(name)?
            .map(|(value, dimension)| Declared::Quantity(value, dimension)),
        TargetKind::Date => parameters.date(name)?.map(Declared::Date),
        TargetKind::DateTime => parameters.date_time(name)?.map(Declared::DateTime),
        TargetKind::Boolean => parameters.boolean(name)?.map(Declared::Boolean),
        TargetKind::Texts => parameters
            .strings(name)?
            .map(|texts| Declared::Texts(texts.to_vec())),
        TargetKind::Text => parameters
            .string(name)?
            .map(|text| Declared::Text(text.to_owned())),
    })
}

/// The `precision` a rule states: `day`, or none for exact.
fn precision(
    parameters: &Parameters<'_>,
    name: &str,
) -> Result<Option<TemporalPrecision>, Unavailable> {
    match parameters.string(name)? {
        None => Ok(None),
        Some("day") => Ok(Some(TemporalPrecision::Day)),
        Some(other) => Err(invalid(format!(
            "{name} `{other}` is unsupported; the only {name} is `day`"
        ))),
    }
}

/// A parameter as the rule declares it, after a space.
fn shown(value: &ParameterValue) -> String {
    match value {
        ParameterValue::Integer { value } => format!(" {value}"),
        ParameterValue::Number { value } => format!(" {value}"),
        ParameterValue::Quantity { value, unit } => format!(" {value} {unit}"),
        ParameterValue::String { value } => format!(" `{value}`"),
        ParameterValue::StringList { value } => format!(" [{}]", value.join(", ")),
        ParameterValue::Boolean { value } => format!(" {value}"),
        ParameterValue::Date { value } => format!(" {value}"),
        ParameterValue::DateTime { value } => format!(" {value}"),
        _ => String::new(),
    }
}

/// The test `test` names against a target of `value`; `None` where the
/// test does not apply to the target's kind.
fn judge(value: Declared, wanted: Test, options: TextOptions) -> Result<Option<Judge>, String> {
    let equality = |order: Order| order.is_equality().then_some(order);
    Ok(match (value, wanted) {
        (Declared::Integer(value), Test::Order(order)) => Some(Judge::Integer(order, value)),
        (Declared::Number(value), Test::Order(order)) => Some(Judge::Number(order, value)),
        (Declared::Quantity(value, dimension), Test::Order(order)) => {
            Some(Judge::Quantity(order, value, dimension))
        }
        (Declared::Date(value), Test::Order(order)) => {
            Some(Judge::Temporal(order, PropertyValue::Date(value)))
        }
        (Declared::DateTime(value), Test::Order(order)) => {
            Some(Judge::Temporal(order, PropertyValue::DateTime(value)))
        }
        (Declared::Boolean(value), Test::Order(order)) => {
            equality(order).map(|order| Judge::Boolean(order, value))
        }
        (Declared::Texts(texts), Test::OneOf | Test::NoneOf) => Some(Judge::Member {
            texts: texts.iter().map(|text| options.fold(text)).collect(),
            none: wanted == Test::NoneOf,
            options,
        }),
        (Declared::Text(text), Test::Order(order)) => {
            equality(order).map(|order| Judge::Text(Some(order), options.fold(&text), options))
        }
        (Declared::Text(text), Test::Contains) => {
            Some(Judge::Text(None, options.fold(&text), options))
        }
        (Declared::Text(text), Test::Matches) => Some(Judge::Pattern(
            shared::pattern(Pattern::Matches, &text, options.case_sensitive)
                .map_err(|error| format!("invalid regular expression: {error}"))?,
            text,
            options.case_sensitive,
        )),
        _ => None,
    })
}

/// Binds `rule`'s statement of `comparison`, checking it in the order
/// [`Comparison`] states.
#[allow(clippy::too_many_lines)]
pub(super) fn bind(rule: &CompiledRule, comparison: &Comparison) -> Result<Bound, Unavailable> {
    let parameters = Parameters(rule);
    let operator = parameters.required_string(comparison.operator)?;
    let case_sensitive = match comparison.case_sensitive {
        Some(name) => parameters.boolean(name)?.unwrap_or(true),
        None => true,
    };
    let options = TextOptions::case(case_sensitive);
    let mut values = Vec::new();
    for target in &comparison.targets {
        values.push(declared(&parameters, target)?);
    }
    let given = values.iter().filter(|value| value.is_some()).count();
    let presence = comparison
        .presence
        .iter()
        .find(|presence| presence.word == operator);
    let expected = usize::from(presence.is_none());
    if given != expected {
        return Err(invalid(format!(
            "operator `{operator}` takes {expected} target value(s); {given} given"
        )));
    }
    let temporal: Vec<&str> = comparison
        .targets
        .iter()
        .filter(|target| target.kind.is_temporal())
        .map(|target| target.parameter)
        .collect();
    let refuse_precision = |name: &str| {
        invalid(format!(
            "`{name}` applies to a {} target only",
            temporal.join(" or ")
        ))
    };
    let mut stated_precision = None;
    let mut precision_read = false;
    let mut bound = None;
    for (target, value) in comparison.targets.iter().zip(values) {
        if !target.kind.is_numeric() && !precision_read {
            if let Some(name) = comparison.precision {
                stated_precision = precision(&parameters, name)?;
            }
            precision_read = true;
        }
        if !target.kind.is_numeric()
            && !target.kind.is_temporal()
            && let (Some(name), Some(_)) = (comparison.precision, stated_precision)
        {
            return Err(refuse_precision(name));
        }
        if let Some(value) = value {
            let test = target
                .operators
                .iter()
                .find(|operation| operation.word == operator)
                .map(|operation| operation.test);
            let judged = match test {
                Some(test) => judge(value, test, options).map_err(invalid)?,
                None => None,
            };
            let judge = judged.ok_or_else(|| {
                invalid(format!(
                    "operator `{operator}` does not apply to {}",
                    target.kind.noun()
                ))
            })?;
            bound = Some((judge, target));
            break;
        }
    }
    let (judge, target) = if let Some((judge, target)) = bound {
        (
            judge,
            rule.parameters
                .get(target.parameter)
                .map_or_else(String::new, shown),
        )
    } else {
        if !precision_read && let Some(name) = comparison.precision {
            stated_precision = precision(&parameters, name)?;
        }
        if let (Some(name), Some(_)) = (comparison.precision, stated_precision) {
            return Err(refuse_precision(name));
        }
        match presence {
            Some(presence) => (Judge::Presence(presence.defined), String::new()),
            None => return Err(invalid(format!("operator `{operator}` is unsupported"))),
        }
    };
    let tolerance = if comparison.tolerance {
        parameters.tolerance()?
    } else {
        Tolerance::EXACT
    };
    let numeric = matches!(
        judge,
        Judge::Integer(..) | Judge::Number(..) | Judge::Quantity(..)
    );
    if !tolerance.is_exact() && !numeric {
        return Err(invalid("a tolerance applies to a numeric target only"));
    }
    Ok(Bound {
        judge,
        tolerance,
        precision: stated_precision,
        target,
    })
}

impl Bound {
    /// ` (<tolerance>)`, or nothing when exact.
    pub(super) fn tolerance_suffix(&self) -> String {
        self.tolerance.suffix()
    }

    /// Whether what the source states, `actual` (`None` where it states
    /// the property absent), satisfies the test; `Err` when it cannot be
    /// judged.
    pub(super) fn holds(&self, actual: Option<&PropertyValue>) -> Result<bool, String> {
        let tolerance = &self.tolerance;
        if let Judge::Presence(defined) = self.judge {
            return Ok(undefined(actual) != defined);
        }
        // A comparison presupposes a value.
        let Some(actual) = actual else {
            return Ok(false);
        };
        if let Judge::Temporal(order, expected) = &self.judge {
            // A value of another type fails, as for every other target.
            return shared::temporal(*order, actual, expected, self.precision).unwrap_or(Ok(false));
        }
        match (&self.judge, actual) {
            (
                Judge::Quantity(order, expected, dimension),
                PropertyValue::Quantity {
                    value,
                    dimension: held,
                },
            ) => {
                return if held == dimension {
                    Ok(number(*order, *value, *expected, tolerance))
                } else {
                    Err(format!(
                        "a quantity in {} cannot be compared with one in {}",
                        held.unit_symbol(),
                        dimension.unit_symbol()
                    ))
                };
            }
            (Judge::Quantity(..), PropertyValue::Integer(_) | PropertyValue::Decimal(_)) => {
                return Err("a unit-less number cannot be compared with a quantity".into());
            }
            (_, PropertyValue::Quantity { .. }) => {
                return Err("a quantity cannot be compared with a unit-less target".into());
            }
            _ => {}
        }
        Ok(match (&self.judge, actual) {
            (Judge::Integer(order, expected), PropertyValue::Integer(value))
                if tolerance.is_exact() =>
            {
                shared::integers(*order, *value, *expected)
            }
            (Judge::Integer(order, expected), PropertyValue::Integer(value)) => {
                match (exact_f64(*value), exact_f64(*expected)) {
                    (Some(value), Some(expected)) => number(*order, value, expected, tolerance),
                    _ => {
                        return Err(
                            "an integer beyond 2^53 cannot be compared under a tolerance".into(),
                        );
                    }
                }
            }
            (Judge::Integer(order, expected), PropertyValue::Decimal(value)) => {
                exact_f64(*expected)
                    .is_some_and(|expected| number(*order, *value, expected, tolerance))
            }
            (Judge::Number(order, expected), PropertyValue::Decimal(value)) => {
                number(*order, *value, *expected, tolerance)
            }
            (Judge::Number(order, expected), PropertyValue::Integer(value)) => {
                exact_f64(*value).is_some_and(|value| number(*order, value, *expected, tolerance))
            }
            (Judge::Text(Some(order), text, options), PropertyValue::String(value)) => {
                shared::texts(*order, value, text, *options)
            }
            (Judge::Text(None, text, options), PropertyValue::String(value)) => {
                shared::contains(value, text, *options)
            }
            (Judge::Pattern(pattern, ..), PropertyValue::String(value)) => pattern.is_match(value),
            (
                Judge::Member {
                    texts,
                    none,
                    options,
                },
                PropertyValue::String(value),
            ) => shared::member(value, texts, *options) != *none,
            (Judge::Boolean(order, expected), PropertyValue::Boolean(value)) => {
                shared::booleans(*order, *value, *expected) == Some(true)
            }
            _ => false,
        })
    }

    /// The test as an expression over `subject`, its target a literal: the
    /// requirement of a rule forked from the template.
    ///
    /// # Errors
    ///
    /// A declared tolerance or date precision, which no expression states.
    pub(super) fn expression(&self, subject: Expression) -> Result<Expression, String> {
        if !self.tolerance.is_exact() {
            return Err("no expression states a tolerance".into());
        }
        if self.precision.is_some() {
            return Err("no expression states a date precision".into());
        }
        let literal = |value: ScalarValue| Expression::Literal { value, label: None };
        let boxed = Box::new;
        Ok(match &self.judge {
            Judge::Presence(true) => Expression::IsDefined {
                operand: boxed(subject),
                label: None,
            },
            Judge::Presence(false) => Expression::IsUndefined {
                operand: boxed(subject),
                label: None,
            },
            Judge::Integer(order, value) => Test::Order(*order).expression(
                subject,
                literal(ScalarValue::Integer { value: *value }),
                true,
            ),
            Judge::Number(order, value) => Test::Order(*order).expression(
                subject,
                literal(ScalarValue::Number { value: *value }),
                true,
            ),
            Judge::Quantity(order, value, dimension) => Test::Order(*order).expression(
                subject,
                literal(ScalarValue::Quantity {
                    value: *value,
                    unit: dimension.unit_symbol().replace('²', "2").replace('³', "3"),
                }),
                true,
            ),
            Judge::Temporal(order, value) => Test::Order(*order).expression(
                subject,
                literal(match value {
                    PropertyValue::Date(value) => ScalarValue::Date { value: *value },
                    PropertyValue::DateTime(value) => ScalarValue::DateTime { value: *value },
                    _ => unreachable!("a temporal target is a date or a date-time"),
                }),
                true,
            ),
            Judge::Boolean(order, value) => Test::Order(*order).expression(
                subject,
                literal(ScalarValue::Boolean { value: *value }),
                true,
            ),
            Judge::Text(order, text, options) => {
                let kind = order.map_or(Test::Contains, Test::Order);
                kind.expression(
                    subject,
                    literal(ScalarValue::String {
                        value: text.clone(),
                    }),
                    options.case_sensitive,
                )
            }
            Judge::Pattern(_, source, case_sensitive) => Test::Matches.expression(
                subject,
                literal(ScalarValue::String {
                    value: source.clone(),
                }),
                *case_sensitive,
            ),
            Judge::Member {
                texts,
                none,
                options,
            } => {
                let values = texts
                    .iter()
                    .map(|text| {
                        literal(ScalarValue::String {
                            value: text.clone(),
                        })
                    })
                    .collect();
                if *none {
                    Expression::NoneOf {
                        operand: boxed(subject),
                        values,
                        case_sensitive: options.case_sensitive,
                        label: None,
                    }
                } else {
                    Expression::OneOf {
                        operand: boxed(subject),
                        values,
                        case_sensitive: options.case_sensitive,
                        label: None,
                    }
                }
            }
        })
    }
}

/// `left order right` through the shared comparison; a value that is no
/// number fails.
fn number(order: Order, left: f64, right: f64, tolerance: &Tolerance) -> bool {
    shared::numbers(order, (left, left), (right, right), tolerance) == Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{Bound, bind};
    use axioval_engine::CompiledRule;
    use axioval_engine::comparison::Order;
    use axioval_engine::template::{
        Comparison, ComparisonTarget, Operation, Presence, TargetKind, Test,
    };
    use axioval_ir::contract::{ParameterValue, Selector, Severity};
    use axioval_ir::{PropertyValue, QuantityDimension, RuleId};

    const ORDERS: &[Operation] = &[
        Operation {
            word: "at_most",
            test: Test::Order(Order::LessOrEqual),
        },
        Operation {
            word: "is",
            test: Test::Order(Order::Equal),
        },
    ];
    const TEXT: &[Operation] = &[
        Operation {
            word: "is",
            test: Test::Order(Order::Equal),
        },
        Operation {
            word: "matches",
            test: Test::Matches,
        },
    ];

    /// A comparison of a length, a day or a text, as a template states it.
    fn comparison() -> Comparison {
        Comparison {
            operator: "op",
            presence: &[Presence {
                word: "stated",
                defined: true,
            }],
            targets: vec![
                ComparisonTarget {
                    parameter: "length",
                    kind: TargetKind::Quantity,
                    operators: ORDERS,
                },
                ComparisonTarget {
                    parameter: "day",
                    kind: TargetKind::Date,
                    operators: ORDERS,
                },
                ComparisonTarget {
                    parameter: "text",
                    kind: TargetKind::Text,
                    operators: TEXT,
                },
            ],
            case_sensitive: Some("case"),
            precision: Some("precision"),
            tolerance: true,
        }
    }

    fn bound(parameters: Vec<(&str, ParameterValue)>) -> Result<Bound, String> {
        let rule = CompiledRule {
            id: RuleId::new("rule").unwrap(),
            capability: "test".into(),
            severity: Severity::Error,
            selector: Selector::All,
            parameters: parameters
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        };
        bind(&rule, &comparison()).map_err(|(_, message)| message)
    }

    fn word(value: &str) -> ParameterValue {
        ParameterValue::String {
            value: value.into(),
        }
    }

    fn millimetres(value: f64) -> ParameterValue {
        ParameterValue::Quantity {
            value,
            unit: "mm".into(),
        }
    }

    fn length(value: f64) -> PropertyValue {
        PropertyValue::Quantity {
            value,
            dimension: QuantityDimension::Length,
        }
    }

    fn refusal(parameters: Vec<(&str, ParameterValue)>) -> String {
        match bound(parameters) {
            Ok(_) => "bound".into(),
            Err(message) => message,
        }
    }

    #[test]
    fn a_statement_is_checked_in_the_templates_order() {
        assert_eq!(
            refusal(vec![("op", word("is"))]),
            "operator `is` takes 1 target value(s); 0 given"
        );
        assert_eq!(
            refusal(vec![("op", word("stated")), ("length", millimetres(300.0))]),
            "operator `stated` takes 0 target value(s); 1 given"
        );
        assert_eq!(
            refusal(vec![
                ("op", word("matches")),
                ("length", millimetres(300.0))
            ]),
            "operator `matches` does not apply to a quantity"
        );
        assert_eq!(
            refusal(vec![
                ("op", word("is")),
                ("text", word("x")),
                ("precision", word("day")),
            ]),
            "`precision` applies to a day target only"
        );
        assert_eq!(
            refusal(vec![
                ("op", word("is")),
                ("text", word("x")),
                ("tolerance", ParameterValue::Number { value: 0.1 }),
            ]),
            "a tolerance applies to a numeric target only"
        );
        // A precision is not read before a numeric target decides.
        assert_eq!(
            refusal(vec![
                ("op", word("at_most")),
                ("length", millimetres(300.0)),
                ("precision", word("hour")),
            ]),
            "bound"
        );
    }

    #[test]
    fn the_judge_compares_what_the_source_states() {
        let Ok(at_most) = bound(vec![
            ("op", word("at_most")),
            ("length", millimetres(300.0)),
        ]) else {
            panic!("a length binds");
        };
        assert_eq!(at_most.target, " 300 mm");
        assert_eq!(at_most.holds(Some(&length(0.3))), Ok(true));
        assert_eq!(at_most.holds(Some(&length(0.31))), Ok(false));
        // Absent, `null` and another kind fail; a bare number is open.
        assert_eq!(at_most.holds(None), Ok(false));
        assert_eq!(at_most.holds(Some(&PropertyValue::Null)), Ok(false));
        assert_eq!(
            at_most.holds(Some(&PropertyValue::String("0.3".into()))),
            Ok(false)
        );
        assert!(at_most.holds(Some(&PropertyValue::Decimal(0.3))).is_err());
        let Ok(folded) = bound(vec![
            ("op", word("is")),
            ("text", word("F90")),
            ("case", ParameterValue::Boolean { value: false }),
        ]) else {
            panic!("a text binds");
        };
        assert_eq!(
            folded.holds(Some(&PropertyValue::String("f90".into()))),
            Ok(true)
        );
        let Ok(stated) = bound(vec![("op", word("stated"))]) else {
            panic!("a presence test binds");
        };
        assert_eq!(stated.target, "");
        assert_eq!(
            stated.holds(Some(&PropertyValue::String("  ".into()))),
            Ok(false)
        );
    }
}
