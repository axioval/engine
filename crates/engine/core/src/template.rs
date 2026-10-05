//! Built-in capabilities as preconfigured compositions ("templates").
//!
//! A template keeps a capability's outside contract (its id, parameter
//! descriptor, defaults, finding wording and three-valued outcomes) and
//! states, as data, how a rule bound to it is decided from the shared
//! parts: measured and stated values read by expressions the engine's one
//! evaluator evaluates, and a decision over them. Packages never supply a
//! template; templates are engine-owned and built in, and a rule bound to
//! one is still exported as that capability with its parameters.
//!
//! A template is [`Template`]: its descriptor, declaration [`Check`]s run
//! once per rule, the host [`Service`]s it needs, named message
//! [`Text`]s, and one or more [`Form`]s. The first form whose `when`
//! parameters are all stated applies; it lists its [`TemplateValue`]s (each an
//! expression, read in order, the first that cannot be read leaving the
//! object open) and its [`Decision`]. Messages are written with
//! placeholders: a [`Text`]'s name, a parameter's name (its value as
//! stated: a string, a property reference as `set.name`), a value's name
//! with a format (`{extent:length}`: `0.3 m` or `between 0.48 m and
//! 0.52 m`; `{target:stated}`: a stated property's value as the source
//! states it), `{bound}` (the bound a [`Decision::Within`] failed or
//! straddled, `at least 0.26 m`) and `{why}` (the refusal of a value that
//! cannot be read). The runner, in the rules crate, binds a rule's
//! parameters into the expressions as constants (slots `{parameter}`,
//! `{parameter.set}` and `{parameter.name}` in a string field, and
//! `parameter` reads), evaluates the values for each selected object with
//! the shared evaluator, decides and words the outcome.
//!
//! [`Form::requirement`] states a form as one expression, the decision's
//! expression form with every value inlined: what the catalogue shows a
//! block editor, and what a rule forked from the template starts from.

use axioval_ir::contract::{Expression, ExpressionComparison, ScalarValue};
use serde::Serialize;

use crate::ParameterDescriptor;

/// Four units in the last place of the largest magnitude a comparison
/// involves: decimal coordinates and lengths read in binary differ from
/// what was meant by that much, and no more. A [`Decision::Within`]
/// widens its bounds by it.
pub const ROUNDING_ULPS: f64 = 4.0 * f64::EPSILON;

/// A built-in capability as a composition of shared parts.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    /// The capability id rules bind to.
    #[serde(skip)]
    pub id: &'static str,
    /// The capability's parameter descriptor, unchanged by the template.
    #[serde(skip)]
    pub parameters: Vec<ParameterDescriptor>,
    /// How messages about the rule as a whole name the capability, such as
    /// `body-extent` in the message ``body-extent: `minimum` exceeds `maximum` ``.
    pub name: &'static str,
    /// Values an optional parameter takes when a rule leaves it unstated,
    /// applied after the declaration checks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<ParameterDefault>,
    /// What a rule's parameters must satisfy, in order; the first that
    /// fails leaves the rule not evaluated as an invalid declaration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub declaration: Vec<Check>,
    /// The host services every value needs. Without any of them the rule
    /// as a whole is left not evaluated (`MissingService`), before any
    /// object is selected, as the capability always did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Services>,
    /// Named message parts, which placeholders name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<Text>,
    /// The compositions, the first whose `when` parameters are all stated
    /// applying.
    pub forms: Vec<Form>,
}

/// A value an optional parameter takes when unstated.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDefault {
    pub parameter: &'static str,
    pub value: ScalarValue,
}

/// One check of a rule's parameters. Each names the parameters it reads;
/// its message is the rule's not-evaluated message after `<name>: `.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Check {
    /// A string parameter is one of `options`: otherwise
    /// `` <parameter> `<value>` is unsupported; use `a`, `b` or `c` ``.
    Choice {
        parameter: &'static str,
        options: &'static [&'static str],
    },
    /// A quantity parameter, where stated, is a length of at least zero:
    /// otherwise `` `<parameter>` is negative `` or
    /// `` `<parameter>` is not a length ``.
    Length { parameter: &'static str },
    /// No parameter of `one` is stated together with one of `other`.
    Exclusive {
        one: &'static [&'static str],
        other: &'static [&'static str],
        message: &'static str,
    },
    /// At least one of `parameters` is stated.
    AnyOf {
        parameters: &'static [&'static str],
        message: &'static str,
    },
    /// `parameter` is stated only together with one of `with`.
    Requires {
        parameter: &'static str,
        with: &'static [&'static str],
        message: &'static str,
    },
    /// Where both are stated, `low` is at most `high`.
    Ordered {
        low: &'static str,
        high: &'static str,
        message: &'static str,
    },
}

/// The host services a template's values need, and the message leaving
/// the rule open without them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Services {
    pub needs: Vec<Service>,
    pub message: &'static str,
}

/// A host service a template may need, by the name the measured-value
/// registry lists it under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    ObjectFrame,
    VerticalExtent,
}

impl Service {
    /// Whether `services` registers it.
    #[must_use]
    pub fn registered(self, services: &crate::ServiceRegistry) -> bool {
        match self {
            Self::ObjectFrame => services.get::<crate::ObjectFrameServiceHandle>().is_some(),
            Self::VerticalExtent => services
                .get::<crate::VerticalExtentServiceHandle>()
                .is_some(),
        }
    }
}

/// A named message part. `{name}` in a message renders it, or nothing
/// where its condition does not hold.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    pub text: &'static str,
}

/// When a [`Text`] is rendered.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Condition {
    /// The parameter (or its default) is a number above zero.
    Positive { parameter: &'static str },
}

/// One composition of a template.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Form {
    /// The parameters whose statement selects this form.
    pub when: &'static [&'static str],
    /// The values read for each selected object, in order.
    pub values: Vec<TemplateValue>,
    /// How the values decide.
    pub decision: Decision,
    /// The finding's message where the decision fails.
    pub fail: &'static str,
    /// The not-evaluated message where the values cannot decide.
    pub undecided: &'static str,
}

/// One value of a form: an expression the shared evaluator evaluates for
/// the object. A value that cannot be read leaves the object not
/// evaluated with the reason and the refusal of whatever it read
/// (`{why}`); a `null` one is a missing-information finding, `absent`
/// wording it; one not of the `expect`ed kind leaves the object not
/// evaluated as invalid evidence, `mismatch` wording it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateValue {
    pub name: &'static str,
    pub expression: Expression,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect: Option<Expect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absent: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mismatch: Option<&'static str>,
}

/// The kind of value a [`TemplateValue`] must have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Expect {
    /// A finite length: a stated property must state a length quantity.
    Length,
}

/// How a form's values decide.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Decision {
    /// The generic range judge: the value `value` lies within `minimum`
    /// and `maximum` (each inclusive, either absent where a parameter it
    /// sums is unstated), both widened by [`ROUNDING_ULPS`] times the
    /// largest of the `rounding` magnitudes. A verdict needs the whole
    /// interval on one side of a bound; one straddling it is undecided,
    /// naming the bound (`{bound}`: `at least 0.26 m`).
    Within {
        value: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        minimum: Option<Vec<Term>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        maximum: Option<Vec<Term>>,
        rounding: Vec<Magnitude>,
    },
}

/// One term of a bound: an operand added or subtracted, left to right.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub sign: Sign,
    pub operand: Operand,
}

impl Term {
    #[must_use]
    pub const fn plus(operand: Operand) -> Self {
        Self {
            sign: Sign::Plus,
            operand,
        }
    }

    #[must_use]
    pub const fn minus(operand: Operand) -> Self {
        Self {
            sign: Sign::Minus,
            operand,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Sign {
    Plus,
    Minus,
}

/// A value of the form or a parameter of the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "camelCase")]
pub enum Operand {
    Value(&'static str),
    Parameter(&'static str),
}

/// A magnitude the rounding allowance scales with: one end of an
/// operand's interval.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Magnitude {
    pub end: End,
    pub operand: Operand,
}

/// An end of an interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum End {
    Lower,
    Upper,
}

impl Form {
    /// The form as one truth expression: the decision's expression form
    /// with each value's expression in place of its name. It still holds
    /// the template's slots (`{axis}`) and `parameter` reads until a rule's
    /// parameters are bound into it.
    #[must_use]
    pub fn requirement(&self) -> Expression {
        self.decision.expression(&|name| {
            self.values
                .iter()
                .find(|value| value.name == name)
                .map_or_else(
                    || Expression::Derived {
                        name: name.to_owned(),
                        label: None,
                    },
                    |value| value.expression.clone(),
                )
        })
    }
}

fn boxed(expression: Expression) -> Box<Expression> {
    Box::new(expression)
}

impl Decision {
    /// The decision as a truth expression, `value` giving each value's
    /// expression and parameters read as `parameter`. A
    /// [`Decision::Within`] is `value ≥ minimum − allowance` and
    /// `value ≤ maximum + allowance`, the allowance
    /// `4ε · max(|m|…)` over its magnitudes. Evaluated by the expression
    /// evaluator's sound interval arithmetic, it reaches the judge's
    /// verdicts except where a value lies within a unit in the last place
    /// of the widened bound, which the expression leaves undecided.
    #[must_use]
    pub fn expression(&self, value: &dyn Fn(&str) -> Expression) -> Expression {
        match self {
            Self::Within {
                value: subject,
                minimum,
                maximum,
                rounding,
            } => {
                let operand = |operand: &Operand| match operand {
                    Operand::Value(name) => value(name),
                    Operand::Parameter(name) => Expression::Parameter {
                        name: (*name).to_owned(),
                        label: None,
                    },
                };
                let sum = |terms: &[Term]| {
                    let mut terms = terms.iter();
                    let first = terms
                        .next()
                        .map_or(Expression::Null { label: None }, |term| {
                            let first = operand(&term.operand);
                            match term.sign {
                                Sign::Plus => first,
                                Sign::Minus => Expression::Negate {
                                    operand: boxed(first),
                                    label: None,
                                },
                            }
                        });
                    terms.fold(first, |sum, term| match term.sign {
                        Sign::Plus => Expression::Add {
                            left: boxed(sum),
                            right: boxed(operand(&term.operand)),
                            label: None,
                        },
                        Sign::Minus => Expression::Subtract {
                            left: boxed(sum),
                            right: boxed(operand(&term.operand)),
                            label: None,
                        },
                    })
                };
                let allowance = Expression::Multiply {
                    left: boxed(Expression::Literal {
                        value: ScalarValue::Number {
                            value: ROUNDING_ULPS,
                        },
                        label: None,
                    }),
                    right: boxed(Expression::Max {
                        operands: rounding
                            .iter()
                            .map(|magnitude| Expression::Abs {
                                operand: boxed(operand(&magnitude.operand)),
                                label: None,
                            })
                            .collect(),
                        label: None,
                    }),
                    label: Some("rounding allowance".into()),
                };
                let mut operands = Vec::new();
                if let Some(terms) = minimum {
                    operands.push(Expression::Compare {
                        operator: ExpressionComparison::GreaterThanOrEquals,
                        left: boxed(value(subject)),
                        right: boxed(Expression::Subtract {
                            left: boxed(sum(terms)),
                            right: boxed(allowance.clone()),
                            label: None,
                        }),
                        case_sensitive: true,
                        label: Some("at least the lower bound".into()),
                    });
                }
                if let Some(terms) = maximum {
                    operands.push(Expression::Compare {
                        operator: ExpressionComparison::LessThanOrEquals,
                        left: boxed(value(subject)),
                        right: boxed(Expression::Add {
                            left: boxed(sum(terms)),
                            right: boxed(allowance),
                            label: None,
                        }),
                        case_sensitive: true,
                        label: Some("at most the upper bound".into()),
                    });
                }
                Expression::And {
                    operands,
                    label: None,
                }
            }
        }
    }
}
