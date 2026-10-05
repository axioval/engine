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
    /// Whether a finding states how far its value misses the bound it
    /// fails (the descriptor's `grades_deviation`), measured from the
    /// declared bound, never the widened one.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub grades: bool,
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
    /// An integer parameter is stated where the descriptor requires it,
    /// and is at least zero: otherwise `` parameter `<parameter>` is
    /// required `` or `` `<parameter>` is negative ``.
    Count { parameter: &'static str },
    /// The parameter, where stated, is of its descriptor's kind, refused
    /// as the parameter reader words a wrong one: a check placed where the
    /// capability read the parameter, so refusals keep their order.
    Kind { parameter: &'static str },
    /// Every number of `parameters` that is stated is at least zero: each
    /// is read first (a wrong type refused as the parameter reader words
    /// it), then any below zero fails with `message`.
    NonNegative {
        parameters: &'static [&'static str],
        message: &'static str,
    },
    /// The traversal the rule declares (`relationship`, `direction`,
    /// `follow_chain`, `path`, `skip_absent_relationship_ends`) is valid,
    /// as the traversal reader words a refusal, and is declared only
    /// together with one of `with`.
    Traversal {
        with: &'static [&'static str],
        message: &'static str,
    },
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
    /// A string-list parameter, where stated, lists at least one valid
    /// discipline: otherwise `` `<parameter>` is empty `` or the
    /// discipline's refusal.
    Disciplines { parameter: &'static str },
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
    TriangleCount,
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
            Self::TriangleCount => services
                .get::<crate::TriangleCountServiceHandle>()
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
    /// The value was read from evidence that is not exact: a count of a
    /// tessellation, a measurement within a chord deviation.
    Inexact { value: &'static str },
    /// The string parameter (or its default) is `value`. Several texts of
    /// one name may each hold under another condition; the first that
    /// holds is rendered.
    Equals {
        parameter: &'static str,
        value: &'static str,
    },
    /// The value's lower end is zero: nothing surely counted.
    Zero { value: &'static str },
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
    /// The members each selected object (an anchor) is judged through,
    /// where the form judges anchors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Members>,
    /// The report table the form fills with what it read, passing or not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
    /// Where the form decides, when not for each selected object: each
    /// source, or the project as a whole, over the objects the rule
    /// selects there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scopes>,
}

/// A form deciding once per source of the session (an empty source
/// included), or once for the whole project, over the objects the rule
/// selects in it: an existence or cardinality check an object rule cannot
/// make, since an object rule over an empty selection reports nothing.
///
/// A value reads the scope's objects as an aggregate over
/// [`Scopes::source`]: the objects surely selected are its members, those
/// whose selection is undecided possible members (a count widens over
/// them). A finding is scoped to the source or the project, relating the
/// objects surely selected and citing what selected them; a scope the
/// possible members leave undecided is not evaluated, each of them too,
/// for its own reason. Messages name the scope as `{place}` and the
/// possible members' count as `{undecided}`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scopes {
    /// The boolean parameter that, true, makes the project one scope;
    /// each source is one otherwise.
    pub across: &'static str,
    /// The string-list parameter naming the disciplines whose sources
    /// count, where the template takes one. A source of another
    /// discipline is left out; one declaring none is not evaluated per
    /// source (`undeclared`) and its selected objects are possible members
    /// of the project (`undeclared_member`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disciplines: Option<&'static str>,
    /// The scope's messages.
    pub messages: ScopeMessages,
}

/// The messages of a [`Scopes`] form. `{source}` names a source,
/// `{disciplines}` the declared disciplines (`` `mep` or `hvac` ``),
/// `{why}` a refusal.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeMessages {
    /// `{place}` for a source scope (``in source `{source}` ``).
    pub source: &'static str,
    /// `{place}` for the project.
    pub project: &'static str,
    /// The rule left open where there is no source to judge.
    pub no_source: &'static str,
    /// The rule left open where no source plays a declared discipline.
    pub no_discipline: &'static str,
    /// A source declaring no discipline, judged per source.
    pub undeclared: &'static str,
    /// A selected object of a source declaring no discipline, judged
    /// across sources.
    pub undeclared_member: &'static str,
    /// A source whose resource objects cannot be listed.
    pub unlisted: &'static str,
    /// The rule left open where disciplines are declared and the run
    /// states none.
    pub no_disciplines: &'static str,
}

impl Scopes {
    /// The aggregate source a value reads a scope's objects through, as a
    /// block editor shows it: the objects the rule selects there.
    #[must_use]
    pub fn source() -> axioval_ir::contract::AggregateSource {
        axioval_ir::contract::AggregateSource::Selector {
            selector: Box::new(axioval_ir::contract::Selector::Expression {
                expression: Box::new(Expression::Parameter {
                    name: SELECTION.to_owned(),
                    label: Some("the objects the rule selects in the scope".to_owned()),
                }),
            }),
        }
    }
}

/// The name a template reads the rule's own selection under: no parameter
/// of any capability, only a block editor's name for it.
pub const SELECTION: &str = "selection";

/// The members of an anchor: the objects a selector parameter picks that
/// the rule's traversal (`relationship` or `path`) reaches from the
/// anchor, or, without one, every such object of the anchor's own source
/// but the anchor. A value reads them as an aggregate over
/// [`Members::source`]; the runner narrows it to the anchor's members.
/// Findings relate the members surely picked.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Members {
    /// The selector parameter picking members.
    pub selector: &'static str,
    /// What members the selector cannot decide leave.
    pub undecided: UndecidedMembers,
}

impl Members {
    /// The aggregate source a value reads an anchor's members through:
    /// the objects the member selector picks, as a block editor shows it.
    /// Run, it is the anchor's members.
    #[must_use]
    pub fn source(selector: &str) -> axioval_ir::contract::AggregateSource {
        axioval_ir::contract::AggregateSource::Selector {
            selector: Box::new(axioval_ir::contract::Selector::Expression {
                expression: Box::new(Expression::Parameter {
                    name: selector.to_owned(),
                    label: None,
                }),
            }),
        }
    }
}

/// How an anchor with members the selector cannot decide is judged.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UndecidedMembers {
    /// Undecided members can only add to the value (a sum of areas): only
    /// a value surely above the maximum stands, and the anchor is otherwise
    /// left not evaluated with `message` (`{undecided}` the count of
    /// undecided members, `{relation}` how they are reached). The anchor
    /// has no row in the form's table: its value is known only from below.
    OnlyExcess { message: &'static str },
}

/// A report table a form fills: one row per selected object whose values
/// were read, keyed by the object.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// The table's name, a report contract.
    pub name: &'static str,
    /// Its columns, in order.
    pub columns: Vec<Column>,
}

/// One column of a [`Table`]: a value of the form, under an id that may
/// name a [`Text`] (`{column}`), in a quantity's dimension.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: &'static str,
    pub value: &'static str,
    pub dimension: axioval_ir::QuantityDimension,
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
    /// The generic comparison judge: the stated value `value` against the
    /// target a rule states, by the operator it names ([`Comparison`]),
    /// decided by the one comparison every rule uses
    /// ([`crate::comparison`]).
    Compare {
        value: &'static str,
        comparison: Comparison,
    },
}

/// A comparison a rule states as an operator word and at most one target
/// parameter: what [`Decision::Compare`] judges.
///
/// Binding checks the statement in the order of `targets`: the operator
/// (`operator`) takes exactly one stated target, none for a `presence`
/// word (``operator `x` takes 1 target value(s); 0 given``); the stated
/// target's kind admits the operator (``operator `x` does not apply to a
/// number``); `precision` (read before the first target that is not a
/// number) applies to a date or date-time target only; a `matches`
/// pattern compiles; and a declared tolerance (`tolerance`,
/// `relative_tolerance` or `decimals`, where `tolerance` holds) applies
/// to a numeric target only.
///
/// The judge compares what the source states. A comparison presupposes a
/// value: an absent property, `null` or a value of another kind than the
/// target fails every operator but a presence test, which reads blank
/// text as undefined. A quantity is compared only with a quantity target
/// of its dimension: against a unit-less target, or a unit-less number
/// against a quantity target, it leaves the object not evaluated, as an
/// integer beyond 2^53 compared under a tolerance does.
///
/// Messages read `{target}`, the stated target as the rule declares it
/// after a space (nothing for a presence test), and `{tolerance:suffix}`,
/// the declared tolerance (`within tolerance 0.01` in parentheses, nothing
/// when exact).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// The string parameter naming the operator.
    pub operator: &'static str,
    /// The operator words that test presence and take no target.
    pub presence: &'static [Presence],
    /// The target parameters, in the order a statement is checked.
    pub targets: Vec<ComparisonTarget>,
    /// The boolean parameter that, `false`, folds the case of text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<&'static str>,
    /// The string parameter stating a date comparison's precision (`day`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<&'static str>,
    /// Whether the rule's tolerance parameters apply to numeric targets.
    pub tolerance: bool,
}

/// An operator word testing presence: whether the value must be defined.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub word: &'static str,
    pub defined: bool,
}

/// A parameter a comparison may take its target from, its kind and the
/// operator words that apply to it.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonTarget {
    pub parameter: &'static str,
    pub kind: TargetKind,
    pub operators: &'static [Operation],
}

/// The kind of value a comparison target states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    Integer,
    Number,
    Quantity,
    Date,
    DateTime,
    Boolean,
    Texts,
    Text,
}

impl TargetKind {
    /// How a message names the kind: `operator `x` does not apply to …`.
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            Self::Integer => "an integer",
            Self::Number => "a number",
            Self::Quantity => "a quantity",
            Self::Date | Self::DateTime => "a date",
            Self::Boolean => "a boolean",
            Self::Texts => "a text list",
            Self::Text => "text",
        }
    }

    /// Whether a tolerance applies to the kind.
    #[must_use]
    pub fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Number | Self::Quantity)
    }

    /// Whether a precision applies to the kind.
    #[must_use]
    pub fn is_temporal(self) -> bool {
        matches!(self, Self::Date | Self::DateTime)
    }
}

/// An operator word and the test it names.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub word: &'static str,
    pub test: Test,
}

/// What an operator tests of a value against its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "order", rename_all = "camelCase")]
pub enum Test {
    /// An order or equality.
    Order(crate::comparison::Order),
    /// The text contains the target.
    Contains,
    /// The whole text matches the target regular expression.
    Matches,
    /// The text is one of the target list.
    OneOf,
    /// The text is none of the target list.
    NoneOf,
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
    /// `4ε · max(|m|…)` over its magnitudes, none without one. Evaluated by the expression
    /// evaluator's sound interval arithmetic, it reaches the judge's
    /// verdicts except where a value lies within a unit in the last place
    /// of the widened bound, which the expression leaves undecided.
    #[must_use]
    #[allow(clippy::too_many_lines)]
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
                // Without a magnitude to scale with there is no allowance:
                // the bound is compared as it is.
                let allowance = (!rounding.is_empty()).then(|| Expression::Multiply {
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
                });
                let widened = |bound: Expression, up: bool| match allowance.clone() {
                    None => bound,
                    Some(allowance) if up => Expression::Add {
                        left: boxed(bound),
                        right: boxed(allowance),
                        label: None,
                    },
                    Some(allowance) => Expression::Subtract {
                        left: boxed(bound),
                        right: boxed(allowance),
                        label: None,
                    },
                };
                let mut operands = Vec::new();
                if let Some(terms) = minimum {
                    operands.push(Expression::Compare {
                        operator: ExpressionComparison::GreaterThanOrEquals,
                        left: boxed(value(subject)),
                        right: boxed(widened(sum(terms), false)),
                        case_sensitive: true,
                        label: Some("at least the lower bound".into()),
                    });
                }
                if let Some(terms) = maximum {
                    operands.push(Expression::Compare {
                        operator: ExpressionComparison::LessThanOrEquals,
                        left: boxed(value(subject)),
                        right: boxed(widened(sum(terms), true)),
                        case_sensitive: true,
                        label: Some("at most the upper bound".into()),
                    });
                }
                Expression::And {
                    operands,
                    label: None,
                }
            }
            Self::Compare {
                value: subject,
                comparison,
            } => comparison.expression(&value(subject)),
        }
    }
}

impl Test {
    /// The expression operator of a test that compares two operands.
    fn operator(self) -> Option<ExpressionComparison> {
        use crate::comparison::Order;
        Some(match self {
            Self::Order(Order::Equal) => ExpressionComparison::Equals,
            Self::Order(Order::NotEqual) => ExpressionComparison::NotEquals,
            Self::Order(Order::Less) => ExpressionComparison::LessThan,
            Self::Order(Order::LessOrEqual) => ExpressionComparison::LessThanOrEquals,
            Self::Order(Order::Greater) => ExpressionComparison::GreaterThan,
            Self::Order(Order::GreaterOrEqual) => ExpressionComparison::GreaterThanOrEquals,
            Self::Contains => ExpressionComparison::Contains,
            Self::Matches => ExpressionComparison::Matches,
            Self::OneOf | Self::NoneOf => return None,
        })
    }

    /// `subject` tested against `target`.
    #[must_use]
    pub fn expression(
        self,
        subject: Expression,
        target: Expression,
        case_sensitive: bool,
    ) -> Expression {
        match self.operator() {
            Some(operator) => Expression::Compare {
                operator,
                left: boxed(subject),
                right: boxed(target),
                case_sensitive,
                label: None,
            },
            None if self == Self::OneOf => Expression::OneOf {
                operand: boxed(subject),
                values: vec![target],
                case_sensitive,
                label: None,
            },
            None => Expression::NoneOf {
                operand: boxed(subject),
                values: vec![target],
                case_sensitive,
                label: None,
            },
        }
    }
}

impl Comparison {
    /// The comparison as one expression over `subject`, for a block editor:
    /// an `if` choosing, by the operator word and the stated target, the
    /// test it names (`isDefined`, `isUndefined`, `compare`, `oneOf`,
    /// `noneOf`), each reading the target as a `parameter`. A rule forked
    /// from the template states only its own test, its target a literal.
    #[must_use]
    pub fn expression(&self, subject: &Expression) -> Expression {
        let parameter = |name: &str| Expression::Parameter {
            name: name.to_owned(),
            label: None,
        };
        let names = |word: &str| Expression::Compare {
            operator: ExpressionComparison::Equals,
            left: boxed(parameter(self.operator)),
            right: boxed(Expression::Literal {
                value: ScalarValue::String {
                    value: word.to_owned(),
                },
                label: None,
            }),
            case_sensitive: true,
            label: Some(format!("`{}` is `{word}`", self.operator)),
        };
        let mut branches: Vec<axioval_ir::contract::Branch> = self
            .presence
            .iter()
            .map(|presence| axioval_ir::contract::Branch {
                when: names(presence.word),
                then: if presence.defined {
                    Expression::IsDefined {
                        operand: boxed(subject.clone()),
                        label: None,
                    }
                } else {
                    Expression::IsUndefined {
                        operand: boxed(subject.clone()),
                        label: None,
                    }
                },
            })
            .collect();
        for target in &self.targets {
            for operation in target.operators {
                branches.push(axioval_ir::contract::Branch {
                    when: Expression::And {
                        operands: vec![
                            Expression::IsDefined {
                                operand: boxed(parameter(target.parameter)),
                                label: Some(format!("`{}` is stated", target.parameter)),
                            },
                            names(operation.word),
                        ],
                        label: None,
                    },
                    then: operation.test.expression(
                        subject.clone(),
                        parameter(target.parameter),
                        true,
                    ),
                });
            }
        }
        Expression::If {
            branches,
            otherwise: boxed(Expression::Null { label: None }),
            label: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn within(rounding: Vec<Magnitude>) -> Decision {
        Decision::Within {
            value: "count",
            minimum: None,
            maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
            rounding,
        }
    }

    /// Without a magnitude to scale with, a bound is compared as it is;
    /// with one, it is widened by the rounding allowance.
    #[test]
    fn a_bound_is_widened_only_by_a_magnitude() {
        let value = |name: &str| Expression::Derived {
            name: name.to_owned(),
            label: None,
        };
        let plain = serde_json::to_string(&within(Vec::new()).expression(&value)).unwrap();
        assert!(!plain.contains("rounding allowance"), "{plain}");
        assert!(
            plain.contains("\"right\":{\"kind\":\"parameter\",\"name\":\"maximum\"}"),
            "{plain}"
        );
        let widened = serde_json::to_string(
            &within(vec![Magnitude {
                end: End::Upper,
                operand: Operand::Value("count"),
            }])
            .expression(&value),
        )
        .unwrap();
        assert!(widened.contains("rounding allowance"), "{widened}");
    }
}
