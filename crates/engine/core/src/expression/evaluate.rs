//! Evaluating an [`Expression`] for one object: values are intervals with
//! units, truth is three-valued, and `null` is never "not evaluated".
//!
//! - A value is [`Value`]: `null` (the source states none), a truth, a
//!   number as an [`Interval`] in coherent units with its [`Unit`], text,
//!   an enumeration value, a date or a date-time.
//! - Not evaluated is [`NotEvaluated`]: a value could not be read or
//!   measured, or a result cannot be decided. It names the subexpression by
//!   its path (`requirement.and[2].compare.left`) and carries a reason.
//! - Truth is Kleene's: `and` is false when any operand is false and `or`
//!   true when any is true, whatever the others; otherwise the first
//!   not-evaluated operand decides. `null` is not true: where a truth is
//!   needed it counts as false, as a comparison with `null` is false.
//! - A comparison of intervals is decided only when every value they allow
//!   gives the same answer; a straddling one is not evaluated.
use std::cmp::Ordering as Order;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use axioval_ir::contract::{
    AggregateFunction, AggregateSource, Branch, Expression, ExpressionComparison, PropertyScope,
    ScalarValue, Selector, SlopeForm,
};
use axioval_ir::{
    Date, DateTime, Evidence, Explanation, ExplanationEntry, MAX_EXPLANATION_ENTRIES,
    PropertyValue, QuantityDimension,
};

use super::interval::{Interval, IntervalFailure};
use super::unit::{Unit, parse_unit};

/// One value of an expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// The source states no value.
    Null,
    /// A truth.
    Boolean(bool),
    /// A number in coherent units: a plain number, or a quantity of `unit`.
    Number {
        /// Every value it may be; a point when known exactly.
        value: Interval,
        /// Its unit; [`Unit::NONE`] for a plain number.
        unit: Unit,
    },
    /// Text.
    Text(String),
    /// A value of an enumeration.
    Enum(String),
    /// A calendar date.
    Date(Date),
    /// A date and time with its offset.
    DateTime(DateTime),
}

impl Value {
    /// A plain number known exactly.
    #[must_use]
    pub fn number(value: f64) -> Self {
        Self::Number {
            value: Interval::point(value),
            unit: Unit::NONE,
        }
    }

    /// A quantity of `dimension` within `value`, in its SI unit.
    #[must_use]
    pub fn quantity(value: Interval, dimension: QuantityDimension) -> Self {
        Self::Number {
            value,
            unit: Unit::of(Some(dimension)),
        }
    }

    /// The value a literal states.
    ///
    /// # Errors
    ///
    /// A quantity whose unit does not parse.
    pub fn from_literal(literal: &ScalarValue) -> Result<Self, String> {
        Ok(match literal {
            ScalarValue::Boolean { value } => Self::Boolean(*value),
            ScalarValue::Integer { value } => Self::integer(*value),
            ScalarValue::Number { value } => Self::number(*value),
            ScalarValue::Quantity { value, unit } => {
                let (scale, unit) = parse_unit(unit)?;
                let coherent = coherent(*value, scale);
                if !coherent.is_finite() {
                    return Err(format!("`{value}` overflows in coherent units"));
                }
                Self::Number {
                    value: Interval::point(coherent),
                    unit,
                }
            }
            ScalarValue::String { value } => Self::Text(value.clone()),
            ScalarValue::Enum { value } => Self::Enum(value.clone()),
            ScalarValue::Date { value } => Self::Date(*value),
            ScalarValue::DateTime { value } => Self::DateTime(*value),
        })
    }

    /// An integer, widened where it has no exact binary value.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn integer(value: i64) -> Self {
        let float = value as f64;
        let interval = if float as i64 == value && float.abs() < 9.0e15 {
            Interval::point(float)
        } else {
            Interval {
                lower: float.next_down(),
                upper: float.next_up(),
            }
        };
        Self::Number {
            value: interval,
            unit: Unit::NONE,
        }
    }

    /// The value a source states.
    ///
    /// # Errors
    ///
    /// A value that is no single scalar: a list, a bounded value, a table,
    /// a reference or a complex property.
    pub fn from_property(value: &PropertyValue) -> Result<Self, String> {
        Ok(match value {
            PropertyValue::Null => Self::Null,
            PropertyValue::Boolean(value) => Self::Boolean(*value),
            PropertyValue::Integer(value) => Self::integer(*value),
            PropertyValue::Decimal(value) => Self::number(*value),
            PropertyValue::Quantity { value, dimension } => {
                Self::quantity(Interval::point(*value), *dimension)
            }
            PropertyValue::Measured {
                lower,
                upper,
                dimension,
            } => Self::Number {
                value: Interval::new(*lower, *upper).ok_or("a measured interval is not ordered")?,
                unit: Unit::of(*dimension),
            },
            PropertyValue::String(value) => Self::Text(value.clone()),
            PropertyValue::Date(value) => Self::Date(*value),
            PropertyValue::DateTime(value) => Self::DateTime(*value),
            PropertyValue::List(_) => return Err("a list is no single value".into()),
            PropertyValue::Bounded { .. } => {
                return Err("a bounded value is no single value".into());
            }
            PropertyValue::Table(_) => return Err("a table value is no single value".into()),
            PropertyValue::Reference(_) => {
                return Err("a reference to another instance is no value".into());
            }
            PropertyValue::Complex => return Err("a complex property is no value".into()),
        })
    }

    /// The kind of value, for messages.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean(_) => "a truth",
            Self::Number { unit, .. } if unit.is_plain() => "a number",
            Self::Number { .. } => "a quantity",
            Self::Text(_) => "text",
            Self::Enum(_) => "an enumeration value",
            Self::Date(_) => "a date",
            Self::DateTime(_) => "a date-time",
        }
    }
}

/// A stated quantity in coherent units: the double nearest the decimal it
/// states, as a source states its values. A decimal prefix (`mm`, `cm`)
/// divides by an exact power of ten, which rounds correctly, so `30 mm` is
/// the same double as `0.030 m`.
#[allow(clippy::float_cmp)]
fn coherent(value: f64, scale: f64) -> f64 {
    if scale < 1.0 {
        let inverse = (1.0 / scale).round();
        let power_of_ten = (0..=15).any(|exponent| inverse == 10f64.powi(exponent));
        if power_of_ten && (1.0 / inverse - scale).abs() <= f64::EPSILON * scale {
            return value / inverse;
        }
    }
    value * scale
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Boolean(value) => write!(f, "{value}"),
            Self::Number { value, unit } => {
                if value.is_point() {
                    write!(f, "{}", value.lower)?;
                } else {
                    write!(f, "{}..{}", value.lower, value.upper)?;
                }
                if !unit.is_plain() {
                    write!(f, " {unit}")?;
                }
                Ok(())
            }
            Self::Text(value) | Self::Enum(value) => write!(f, "`{value}`"),
            Self::Date(value) => write!(f, "{value}"),
            Self::DateTime(value) => write!(f, "{value}"),
        }
    }
}

/// Why an expression, or one of its subexpressions, is not evaluated.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// A property, parameter, derived value or table cell could not be
    /// read or measured.
    Unreadable(String),
    /// A comparison's operands allow values that answer it both ways.
    Straddles {
        /// The left operand.
        left: Box<Value>,
        /// The right operand.
        right: Box<Value>,
    },
    /// An `if` condition is not evaluated and the branches it chooses
    /// between do not agree.
    UndecidedCondition(Box<NotEvaluated>),
    /// A divisor may be zero.
    ZeroDivisor,
    /// A result is not finite.
    Overflow,
    /// An operand lies outside a function's domain.
    Domain,
    /// A value is not of the kind the operation needs: a property whose
    /// type is only known when read, or units that differ.
    Mismatch(String),
    /// A text pattern does not compile.
    InvalidPattern(String),
    /// The run's evaluation budget is spent.
    BudgetExhausted,
    /// An aggregate's result depends on members whose membership cannot
    /// be decided, this many.
    UndecidedMembers(usize),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "cannot be read: {why}"),
            Self::Straddles { left, right } => {
                write!(f, "{left} and {right} allow both answers")
            }
            Self::UndecidedCondition(why) => {
                write!(f, "the branches disagree and the condition {why}")
            }
            Self::ZeroDivisor => f.write_str("the divisor may be zero"),
            Self::Overflow => f.write_str("the result is not finite"),
            Self::Domain => f.write_str("the operand lies outside the function's domain"),
            Self::Mismatch(why) => f.write_str(why),
            Self::InvalidPattern(why) => write!(f, "the pattern is invalid: {why}"),
            Self::BudgetExhausted => f.write_str("the run's expression evaluation budget is spent"),
            Self::UndecidedMembers(count) => write!(
                f,
                "it depends on {count} member(s) whose membership cannot be decided"
            ),
        }
    }
}

/// A subexpression that is not evaluated, by its path, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct NotEvaluated {
    /// Where in the expression, such as `requirement.and[2].compare.left`.
    pub path: String,
    /// The node's author label, if it carries one.
    pub label: Option<String>,
    /// Why.
    pub reason: Reason,
}

impl fmt::Display for NotEvaluated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.label {
            Some(label) => write!(f, "`{label}` ({}) {}", self.path, self.reason),
            None => write!(f, "`{}` {}", self.path, self.reason),
        }
    }
}

/// What a leaf of an expression reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A property of the object in scope.
    Property {
        /// Its set, if named.
        set: Option<String>,
        /// Its name.
        name: String,
    },
    /// A parameter of the rule.
    Parameter(String),
    /// A derived value of the ruleset.
    Derived(String),
    /// The members of an aggregate.
    Aggregate(AggregateFunction),
    /// Another rule's outcome about the object in scope.
    Rule {
        /// The rule.
        rule: String,
        /// What was read of it.
        read: RuleRead,
    },
    /// A cell of a table parameter.
    Lookup {
        /// The table parameter.
        table: String,
        /// The column read.
        column: String,
    },
}

/// A leaf's answer: its value, or why it has none, and the evidence it
/// rests on.
#[derive(Clone, Debug, PartialEq)]
pub struct Leaf {
    /// The value read, or why none could be.
    pub value: Result<Value, String>,
    /// The evidence of the read.
    pub evidence: Vec<Evidence>,
}

impl Leaf {
    /// A value read without evidence of its own (a rule parameter).
    #[must_use]
    pub fn stated(value: Value) -> Self {
        Self {
            value: Ok(value),
            evidence: Vec::new(),
        }
    }

    /// No value, and why.
    #[must_use]
    pub fn unreadable(why: impl Into<String>) -> Self {
        Self {
            value: Err(why.into()),
            evidence: Vec::new(),
        }
    }
}

/// One leaf the evaluation read, in reading order.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    /// Where in the expression.
    pub path: String,
    /// What was read.
    pub source: Source,
    /// The leaf's answer.
    pub leaf: Leaf,
}

/// What an expression reads of another rule's outcome about the object
/// in scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleRead {
    /// Whether it passed the object: true, false when it reported a
    /// finding, `null` when it did not select it.
    Outcome,
    /// How many findings it reported about the object.
    FindingCount,
    /// The greatest graded deviation of its findings about the object.
    Deviation,
}

/// One member of an aggregate: whether it surely is one, and its value
/// with that member in scope.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// Whether the member surely belongs; `false` when its path or filter
    /// could not decide it.
    pub certain: bool,
    /// Its value: `null` for a count, which reads none.
    pub value: Result<Value, NotEvaluated>,
    /// The evidence of its membership and its value.
    pub evidence: Vec<Evidence>,
}

/// The default number of expression nodes one run may evaluate.
pub const DEFAULT_EVALUATION_BUDGET: u64 = 100_000_000;

/// The work a run may spend evaluating expressions, in nodes, shared by
/// every rule, value and aggregate member of the run.
#[derive(Debug)]
pub struct EvaluationBudget {
    remaining: AtomicU64,
    limit: u64,
}

impl EvaluationBudget {
    /// A budget of `limit` nodes.
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self {
            remaining: AtomicU64::new(limit),
            limit,
        }
    }

    /// Spends one node; false once the budget is spent.
    pub fn spend(&self) -> bool {
        self.remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(1)
            })
            .is_ok()
    }

    /// The budget's size.
    #[must_use]
    pub fn limit(&self) -> u64 {
        self.limit
    }
}

/// Answers an expression's leaves for one object in scope.
pub trait ExpressionContext {
    /// Spends the work of evaluating one node; false once the run's budget
    /// is spent, which leaves the expression not evaluated.
    fn spend(&mut self) -> bool {
        true
    }

    /// A property of the object, `set` and `name` as the expression names
    /// them.
    fn property(&mut self, set: Option<&str>, name: &str) -> Leaf;

    /// A property of the rule's checked object (`of: subject`): the object
    /// in scope, except inside an aggregate's member scope.
    fn subject_property(&mut self, set: Option<&str>, name: &str) -> Leaf {
        self.property(set, name)
    }

    /// A parameter of the rule.
    fn parameter(&mut self, name: &str) -> Leaf;

    /// A derived value of the ruleset.
    fn derived(&mut self, name: &str) -> Leaf {
        Leaf::unreadable(format!("no derived value `{name}` is declared"))
    }

    /// The `column` cell of the most specific row of the table parameter
    /// `table` whose key columns match `keys`.
    fn lookup(&mut self, table: &str, keys: &BTreeMap<String, Value>, column: &str) -> Leaf {
        let _ = (keys, column);
        Leaf::unreadable(format!("table `{table}` cannot be looked up here"))
    }

    /// What the rule `rule` of the ruleset concluded about the object in
    /// scope, as `read` asks.
    fn rule(&mut self, rule: &str, read: RuleRead) -> Leaf {
        let _ = read;
        Leaf::unreadable(format!(
            "the outcomes of rule `{rule}` are not available here"
        ))
    }

    /// The members `over` reaches from the object in scope that `filter`
    /// selects, each with `value` evaluated with it in scope (none for a
    /// count), the subexpression's path `path`.
    ///
    /// # Errors
    ///
    /// Why the members cannot be listed: a path the source cannot answer.
    fn members(
        &mut self,
        over: &AggregateSource,
        filter: Option<&Selector>,
        value: Option<&Expression>,
        path: &str,
    ) -> Result<Vec<Member>, String> {
        let _ = (over, filter, value, path);
        Err("aggregates cannot be listed here".into())
    }
}

/// An expression's value for one object, and every leaf it read.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// The value, or the subexpression that is not evaluated.
    pub outcome: Result<Value, NotEvaluated>,
    /// The leaves read, in order, with their evidence.
    pub reads: Vec<Read>,
    /// The value of every labelled subexpression evaluated, by label: the
    /// first, where a label repeats.
    pub labelled: BTreeMap<String, Value>,
    /// Every subexpression evaluated, operands before the node they feed.
    pub trace: Vec<ExplanationEntry>,
}

impl Evaluation {
    /// The explanation of a verdict decided at `deciding` (the path of the
    /// subexpression that failed or was not evaluated): every step on its
    /// path from the root and below it, then the other steps in evaluation
    /// order while [`MAX_EXPLANATION_ENTRIES`] allows.
    #[must_use]
    pub fn explain(&self, deciding: &str) -> Explanation {
        let on_path = |path: &str| {
            let within = |outer: &str, inner: &str| {
                inner == outer
                    || inner
                        .strip_prefix(outer)
                        .is_some_and(|rest| rest.starts_with(['.', '[']))
            };
            within(path, deciding) || within(deciding, path)
        };
        let deciding_steps = self.trace.iter().filter(|step| on_path(&step.path)).count();
        let mut room = MAX_EXPLANATION_ENTRIES.saturating_sub(deciding_steps);
        let mut truncated = false;
        let steps = self
            .trace
            .iter()
            .filter_map(|step| {
                let deciding = on_path(&step.path);
                if !deciding {
                    if room == 0 {
                        truncated = true;
                        return None;
                    }
                    room -= 1;
                }
                Some(ExplanationEntry {
                    deciding,
                    ..step.clone()
                })
            })
            .collect();
        Explanation {
            entries: steps,
            truncated,
        }
    }
}

/// Evaluates `expression`, whose path is `root` (`requirement`).
pub fn evaluate(
    expression: &Expression,
    root: &str,
    context: &mut dyn ExpressionContext,
) -> Evaluation {
    let mut evaluator = Evaluator {
        context,
        reads: Vec::new(),
        labelled: BTreeMap::new(),
        trace: Vec::new(),
    };
    let outcome = evaluator.eval(expression, root);
    Evaluation {
        outcome,
        reads: evaluator.reads,
        labelled: evaluator.labelled,
        trace: evaluator.trace,
    }
}

type Outcome = Result<Value, NotEvaluated>;

struct Evaluator<'c> {
    context: &'c mut dyn ExpressionContext,
    reads: Vec<Read>,
    labelled: BTreeMap<String, Value>,
    trace: Vec<ExplanationEntry>,
}

fn fail(expression: &Expression, path: &str, reason: Reason) -> NotEvaluated {
    NotEvaluated {
        path: path.to_owned(),
        label: expression.label().map(str::to_owned),
        reason,
    }
}

fn interval_reason(failure: IntervalFailure) -> Reason {
    match failure {
        IntervalFailure::ZeroDivisor => Reason::ZeroDivisor,
        IntervalFailure::Overflow => Reason::Overflow,
        IntervalFailure::Domain => Reason::Domain,
    }
}

/// A three-valued truth: decided, or not evaluated.
type Truth = Result<bool, NotEvaluated>;

impl Evaluator<'_> {
    fn eval(&mut self, expression: &Expression, path: &str) -> Outcome {
        let outcome = if self.context.spend() {
            self.eval_node(expression, path)
        } else {
            Err(fail(expression, path, Reason::BudgetExhausted))
        };
        if let (Some(label), Ok(value)) = (expression.label(), &outcome) {
            self.labelled
                .entry(label.to_owned())
                .or_insert_with(|| value.clone());
        }
        let (value, not_evaluated) = match &outcome {
            Ok(value) => (Some(value.to_string()), None),
            Err(why) => (None, Some(why.reason.to_string())),
        };
        self.trace.push(ExplanationEntry {
            path: path.to_owned(),
            kind: expression.kind().to_owned(),
            label: expression.label().map(str::to_owned),
            value,
            not_evaluated,
            deciding: false,
        });
        outcome
    }

    #[allow(clippy::too_many_lines)]
    fn eval_node(&mut self, expression: &Expression, path: &str) -> Outcome {
        let kind = expression.kind();
        let child = |field: &str| format!("{path}.{kind}.{field}");
        let item = |index: usize| format!("{path}.{kind}[{index}]");
        let here = |reason: Reason| fail(expression, path, reason);
        match expression {
            Expression::Literal { value, .. } => {
                Value::from_literal(value).map_err(|why| here(Reason::Mismatch(why)))
            }
            Expression::Null { .. } => Ok(Value::Null),
            Expression::Property {
                property_set,
                property,
                of,
                ..
            } => {
                let leaf = match of {
                    Some(PropertyScope::Subject) => self
                        .context
                        .subject_property(property_set.as_deref(), property),
                    None => self.context.property(property_set.as_deref(), property),
                };
                self.read(
                    expression,
                    path,
                    Source::Property {
                        set: property_set.clone(),
                        name: property.clone(),
                    },
                    leaf,
                )
            }
            Expression::Parameter { name, .. } => {
                let leaf = self.context.parameter(name);
                self.read(expression, path, Source::Parameter(name.clone()), leaf)
            }
            Expression::Derived { name, .. } => {
                let leaf = self.context.derived(name);
                self.read(expression, path, Source::Derived(name.clone()), leaf)
            }
            Expression::Lookup {
                table,
                keys,
                column,
                ..
            } => {
                let mut values = BTreeMap::new();
                for (key, operand) in keys {
                    let value = self.eval(operand, &format!("{path}.lookup.keys[{key}]"))?;
                    values.insert(key.clone(), value);
                }
                let leaf = self.context.lookup(table, &values, column);
                self.read(
                    expression,
                    path,
                    Source::Lookup {
                        table: table.clone(),
                        column: column.clone(),
                    },
                    leaf,
                )
            }
            Expression::Not { operand, .. } => {
                let truth = self.truth(operand, &child("operand"))?;
                Ok(Value::Boolean(!truth))
            }
            Expression::And { operands, .. } => {
                self.junction(operands, &item, false).map(Value::Boolean)
            }
            Expression::Or { operands, .. } => {
                self.junction(operands, &item, true).map(Value::Boolean)
            }
            Expression::Implies {
                antecedent,
                consequent,
                ..
            } => {
                let antecedent = self.truth(antecedent, &child("antecedent"));
                if antecedent == Ok(false) {
                    return Ok(Value::Boolean(true));
                }
                let consequent = self.truth(consequent, &child("consequent"));
                match (antecedent, consequent) {
                    (_, Ok(true)) => Ok(Value::Boolean(true)),
                    (Ok(true), Ok(false)) => Ok(Value::Boolean(false)),
                    (Err(why), _) | (_, Err(why)) => Err(why),
                    (Ok(false), _) => unreachable!("returned above"),
                }
            }
            Expression::Xor { left, right, .. } => {
                let left = self.truth(left, &child("left"))?;
                let right = self.truth(right, &child("right"))?;
                Ok(Value::Boolean(left != right))
            }
            Expression::Compare {
                operator,
                left,
                right,
                case_sensitive,
                ..
            } => {
                let left = self.eval(left, &child("left"));
                let right = self.eval(right, &child("right"));
                let (left, right) = (left?, right?);
                compare(*operator, &left, &right, *case_sensitive)
                    .map(Value::Boolean)
                    .map_err(here)
            }
            Expression::Between {
                operand,
                low,
                high,
                low_inclusive,
                high_inclusive,
                ..
            } => {
                let value = self.eval(operand, &child("operand"));
                let low = self.eval(low, &child("low"));
                let high = self.eval(high, &child("high"));
                let (value, low, high) = (value?, low?, high?);
                let above = if *low_inclusive {
                    ExpressionComparison::GreaterThanOrEquals
                } else {
                    ExpressionComparison::GreaterThan
                };
                let below = if *high_inclusive {
                    ExpressionComparison::LessThanOrEquals
                } else {
                    ExpressionComparison::LessThan
                };
                let above = compare(above, &value, &low, true).map_err(here);
                let below = compare(below, &value, &high, true).map_err(here);
                kleene(&[above, below], false).map(Value::Boolean)
            }
            Expression::OneOf {
                operand,
                values,
                case_sensitive,
                ..
            }
            | Expression::NoneOf {
                operand,
                values,
                case_sensitive,
                ..
            } => {
                let none = matches!(expression, Expression::NoneOf { .. });
                let value = self.eval(operand, &child("operand"))?;
                if value == Value::Null {
                    return Ok(Value::Boolean(false));
                }
                let mut truths = Vec::new();
                for (index, candidate) in values.iter().enumerate() {
                    let candidate =
                        self.eval(candidate, &format!("{path}.{kind}.values[{index}]"))?;
                    let operator = if none {
                        ExpressionComparison::NotEquals
                    } else {
                        ExpressionComparison::Equals
                    };
                    truths
                        .push(compare(operator, &value, &candidate, *case_sensitive).map_err(here));
                }
                kleene(&truths, !none).map(Value::Boolean)
            }
            Expression::IsDefined { operand, .. } => Ok(Value::Boolean(
                self.eval(operand, &child("operand"))? != Value::Null,
            )),
            Expression::IsUndefined { operand, .. } => Ok(Value::Boolean(
                self.eval(operand, &child("operand"))? == Value::Null,
            )),
            Expression::If {
                branches,
                otherwise,
                ..
            } => self.choose(branches, 0, otherwise, path),
            Expression::Coalesce { operands, .. } => {
                for (index, operand) in operands.iter().enumerate() {
                    let value = self.eval(operand, &item(index))?;
                    if value != Value::Null {
                        return Ok(value);
                    }
                }
                Ok(Value::Null)
            }
            Expression::Add { left, right, .. }
            | Expression::Subtract { left, right, .. }
            | Expression::Multiply { left, right, .. }
            | Expression::Divide { left, right, .. } => {
                let left = self.eval(left, &child("left"));
                let right = self.eval(right, &child("right"));
                let (left, right) = match (left, right) {
                    (Ok(Value::Null), _) | (_, Ok(Value::Null)) => return Ok(Value::Null),
                    (left, right) => (left?, right?),
                };
                arithmetic(expression, &left, &right).map_err(here)
            }
            Expression::Min { operands, .. } | Expression::Max { operands, .. } => {
                let least = matches!(expression, Expression::Min { .. });
                let mut values = Vec::new();
                for (index, operand) in operands.iter().enumerate() {
                    values.push(self.eval(operand, &item(index)));
                }
                if values.iter().any(|value| value == &Ok(Value::Null)) {
                    return Ok(Value::Null);
                }
                let mut result: Option<(Interval, Unit)> = None;
                for value in values {
                    let (value, unit) = number(&value?).map_err(here)?;
                    result = Some(match result {
                        None => (value, unit),
                        Some((_, ref own)) if *own != unit => {
                            return Err(here(Reason::Mismatch(format!(
                                "`{kind}` compares {own} and {unit}, which differ"
                            ))));
                        }
                        Some((current, own)) => {
                            let next = if least {
                                current.min(value)
                            } else {
                                current.max(value)
                            };
                            (next, own)
                        }
                    });
                }
                let (value, unit) = result.ok_or_else(|| here(Reason::Domain))?;
                Ok(Value::Number { value, unit })
            }
            Expression::Round { operand, step, .. } => {
                let value = self.eval(operand, &child("operand"));
                let step = self.eval(step, &child("step"));
                let (value, step) = match (value, step) {
                    (Ok(Value::Null), _) | (_, Ok(Value::Null)) => return Ok(Value::Null),
                    (value, step) => (value?, step?),
                };
                let (value, unit) = number(&value).map_err(here)?;
                let (step, step_unit) = number(&step).map_err(here)?;
                if unit != step_unit {
                    return Err(here(Reason::Mismatch(format!(
                        "it rounds {unit} to a step of {step_unit}, which differ"
                    ))));
                }
                let value = value
                    .round_to(step)
                    .map_err(|failure| here(interval_reason(failure)))?;
                Ok(Value::Number { value, unit })
            }
            Expression::Negate { operand, .. }
            | Expression::Abs { operand, .. }
            | Expression::Floor { operand, .. }
            | Expression::Ceil { operand, .. }
            | Expression::Sqrt { operand, .. }
            | Expression::Sin { operand, .. }
            | Expression::Cos { operand, .. }
            | Expression::Tan { operand, .. }
            | Expression::ConvertSlope { operand, .. } => {
                let value = self.eval(operand, &child("operand"))?;
                if value == Value::Null {
                    return Ok(Value::Null);
                }
                let (value, unit) = number(&value).map_err(here)?;
                unary(expression, value, &unit).map_err(here)
            }
            Expression::Atan2 { y, x, .. } => {
                let y = self.eval(y, &child("y"));
                let x = self.eval(x, &child("x"));
                let (y, x) = match (y, x) {
                    (Ok(Value::Null), _) | (_, Ok(Value::Null)) => return Ok(Value::Null),
                    (y, x) => (y?, x?),
                };
                let (y, y_unit) = number(&y).map_err(here)?;
                let (x, x_unit) = number(&x).map_err(here)?;
                if y_unit != x_unit {
                    return Err(here(Reason::Mismatch(format!(
                        "`atan2` takes {y_unit} and {x_unit}, which differ"
                    ))));
                }
                let value =
                    Interval::atan2(y, x).map_err(|failure| here(interval_reason(failure)))?;
                Ok(Value::Number {
                    value,
                    unit: Unit::RADIAN,
                })
            }
            Expression::Aggregate {
                function,
                over,
                filter,
                value,
                ..
            } => {
                let value_path = format!("{path}.aggregate.value");
                let members =
                    self.context
                        .members(over, filter.as_deref(), value.as_deref(), &value_path);
                let leaf = Leaf {
                    value: Ok(Value::Null),
                    evidence: members
                        .iter()
                        .flatten()
                        .flat_map(|member| member.evidence.iter().cloned())
                        .collect(),
                };
                self.reads.push(Read {
                    path: path.to_owned(),
                    source: Source::Aggregate(*function),
                    leaf,
                });
                let members = members.map_err(|why| here(Reason::Unreadable(why)))?;
                super::aggregate::aggregate(*function, &members).map_err(|why| match why {
                    super::aggregate::Failure::Member(inner) => inner,
                    super::aggregate::Failure::Here(reason) => here(reason),
                })
            }
            Expression::RuleOutcome { rule, .. }
            | Expression::FindingCount { rule, .. }
            | Expression::Deviation { rule, .. } => {
                let read = match expression {
                    Expression::RuleOutcome { .. } => RuleRead::Outcome,
                    Expression::FindingCount { .. } => RuleRead::FindingCount,
                    _ => RuleRead::Deviation,
                };
                let leaf = self.context.rule(rule, read);
                self.read(
                    expression,
                    path,
                    Source::Rule {
                        rule: rule.clone(),
                        read,
                    },
                    leaf,
                )
            }
            Expression::Concat { operands, .. } => {
                let mut text = String::new();
                let mut null = false;
                for (index, operand) in operands.iter().enumerate() {
                    match self.eval(operand, &item(index))? {
                        Value::Null => null = true,
                        Value::Text(part) | Value::Enum(part) => text.push_str(&part),
                        other => {
                            return Err(here(Reason::Mismatch(format!(
                                "`concat` joins text, not {}",
                                other.kind()
                            ))));
                        }
                    }
                }
                Ok(if null { Value::Null } else { Value::Text(text) })
            }
            Expression::Length { operand, .. }
            | Expression::Lower { operand, .. }
            | Expression::Upper { operand, .. }
            | Expression::Trim { operand, .. } => {
                let text = match self.eval(operand, &child("operand"))? {
                    Value::Null => return Ok(Value::Null),
                    Value::Text(text) | Value::Enum(text) => text,
                    other => {
                        return Err(here(Reason::Mismatch(format!(
                            "`{kind}` takes text, not {}",
                            other.kind()
                        ))));
                    }
                };
                Ok(match expression {
                    Expression::Length { .. } => {
                        Value::integer(i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
                    }
                    Expression::Lower { .. } => Value::Text(text.to_lowercase()),
                    Expression::Upper { .. } => Value::Text(text.to_uppercase()),
                    _ => Value::Text(text.trim().to_owned()),
                })
            }
        }
    }

    fn read(&mut self, expression: &Expression, path: &str, source: Source, leaf: Leaf) -> Outcome {
        let outcome = leaf
            .value
            .clone()
            .map_err(|why| fail(expression, path, Reason::Unreadable(why)));
        self.reads.push(Read {
            path: path.to_owned(),
            source,
            leaf,
        });
        outcome
    }

    /// The truth of `operand`; `null` is not true.
    fn truth(&mut self, operand: &Expression, path: &str) -> Truth {
        match self.eval(operand, path)? {
            Value::Boolean(value) => Ok(value),
            Value::Null => Ok(false),
            other => Err(fail(
                operand,
                path,
                Reason::Mismatch(format!("a truth is needed, not {}", other.kind())),
            )),
        }
    }

    /// `and` (`decisive` false) or `or` (`decisive` true), Kleene's way: a
    /// decisive operand decides and ends the evaluation.
    fn junction(
        &mut self,
        operands: &[Expression],
        item: &dyn Fn(usize) -> String,
        decisive: bool,
    ) -> Truth {
        let mut undecided = None;
        for (index, operand) in operands.iter().enumerate() {
            match self.truth(operand, &item(index)) {
                Ok(truth) if truth == decisive => return Ok(decisive),
                Ok(_) => {}
                Err(why) => {
                    undecided.get_or_insert(why);
                }
            }
        }
        undecided.map_or(Ok(!decisive), Err)
    }

    /// The value of the first branch from `from` whose condition holds. A
    /// condition not evaluated still decides when the branch it would take
    /// and the rest agree on one value.
    fn choose(
        &mut self,
        branches: &[Branch],
        from: usize,
        otherwise: &Expression,
        path: &str,
    ) -> Outcome {
        let Some(branch) = branches.get(from) else {
            return self.eval(otherwise, &format!("{path}.if.else"));
        };
        let when = format!("{path}.if.branches[{from}].when");
        let then = format!("{path}.if.branches[{from}].then");
        match self.truth(&branch.when, &when) {
            Ok(true) => self.eval(&branch.then, &then),
            Ok(false) => self.choose(branches, from + 1, otherwise, path),
            Err(why) => {
                let taken = self.eval(&branch.then, &then);
                let rest = self.choose(branches, from + 1, otherwise, path);
                match (taken, rest) {
                    (Ok(taken), Ok(rest)) if taken == rest => Ok(taken),
                    _ => Err(NotEvaluated {
                        path: path.to_owned(),
                        label: None,
                        reason: Reason::UndecidedCondition(Box::new(why)),
                    }),
                }
            }
        }
    }
}

/// Kleene's `and` (`decisive` false) or `or` (`decisive` true) over truths
/// already evaluated.
fn kleene(truths: &[Truth], decisive: bool) -> Truth {
    if truths.contains(&Ok(decisive)) {
        return Ok(decisive);
    }
    truths
        .iter()
        .find_map(|truth| truth.clone().err())
        .map_or(Ok(!decisive), Err)
}

fn number(value: &Value) -> Result<(Interval, Unit), Reason> {
    match value {
        Value::Number { value, unit } => Ok((*value, unit.clone())),
        other => Err(Reason::Mismatch(format!(
            "a number is needed, not {}",
            other.kind()
        ))),
    }
}

fn arithmetic(expression: &Expression, left: &Value, right: &Value) -> Result<Value, Reason> {
    let (left, left_unit) = number(left)?;
    let (right, right_unit) = number(right)?;
    let same = || {
        if left_unit == right_unit {
            Ok(left_unit.clone())
        } else {
            Err(Reason::Mismatch(format!(
                "`{}` takes {left_unit} and {right_unit}, which differ",
                expression.kind()
            )))
        }
    };
    let (value, unit) = match expression {
        Expression::Add { .. } => (left.plus(right), same()?),
        Expression::Subtract { .. } => (left.minus(right), same()?),
        Expression::Multiply { .. } => (
            left.times(right),
            left_unit.times(&right_unit, 1).map_err(Reason::Mismatch)?,
        ),
        _ => (
            left.divided_by(right),
            left_unit.times(&right_unit, -1).map_err(Reason::Mismatch)?,
        ),
    };
    Ok(Value::Number {
        value: value.map_err(interval_reason)?,
        unit,
    })
}

fn unary(expression: &Expression, value: Interval, unit: &Unit) -> Result<Value, Reason> {
    let kind = expression.kind();
    let angle = || {
        if *unit == Unit::RADIAN {
            Ok(())
        } else {
            Err(Reason::Mismatch(format!(
                "`{kind}` takes a plane angle, not {unit}"
            )))
        }
    };
    let plain = |value: Interval| Value::Number {
        value,
        unit: Unit::NONE,
    };
    let same = |value: Interval| Value::Number {
        value,
        unit: unit.clone(),
    };
    Ok(match expression {
        Expression::Negate { .. } => same(value.negate()),
        Expression::Abs { .. } => same(value.abs()),
        Expression::Floor { .. } => same(value.floor()),
        Expression::Ceil { .. } => same(value.ceil()),
        Expression::Sqrt { .. } => Value::Number {
            value: value.sqrt().map_err(interval_reason)?,
            unit: unit
                .sqrt()
                .ok_or_else(|| Reason::Mismatch(format!("{unit} has no square root")))?,
        },
        Expression::Sin { .. } => {
            angle()?;
            plain(value.sin())
        }
        Expression::Cos { .. } => {
            angle()?;
            plain(value.cos())
        }
        Expression::Tan { .. } => {
            angle()?;
            plain(value.tan().map_err(interval_reason)?)
        }
        Expression::ConvertSlope { from, to, .. } => {
            let expected = if *from == SlopeForm::Angle {
                Unit::RADIAN
            } else {
                Unit::NONE
            };
            if *unit != expected {
                return Err(Reason::Mismatch(format!(
                    "a slope stated as {} is {expected}, not {unit}",
                    slope_name(*from)
                )));
            }
            let hundred = Interval::point(100.0);
            let ratio = match from {
                SlopeForm::Ratio => value,
                SlopeForm::Percent => value.divided_by(hundred).map_err(interval_reason)?,
                SlopeForm::Angle => value.tan().map_err(interval_reason)?,
            };
            match to {
                SlopeForm::Ratio => plain(ratio),
                SlopeForm::Percent => plain(ratio.times(hundred).map_err(interval_reason)?),
                SlopeForm::Angle => Value::Number {
                    value: ratio.atan(),
                    unit: Unit::RADIAN,
                },
            }
        }
        _ => unreachable!("only unary numeric kinds reach here"),
    })
}

fn slope_name(form: SlopeForm) -> &'static str {
    match form {
        SlopeForm::Ratio => "a ratio",
        SlopeForm::Percent => "a percentage",
        SlopeForm::Angle => "an angle",
    }
}

/// `left operator right`: false when either is `null`, otherwise decided
/// only when every value the operands allow gives one answer.
fn compare(
    operator: ExpressionComparison,
    left: &Value,
    right: &Value,
    case_sensitive: bool,
) -> Result<bool, Reason> {
    use ExpressionComparison as C;
    if *left == Value::Null || *right == Value::Null {
        return Ok(false);
    }
    let fold = |text: &str| {
        if case_sensitive {
            text.to_owned()
        } else {
            text.to_lowercase()
        }
    };
    let text = |value: &Value| match value {
        Value::Text(text) | Value::Enum(text) => Some(fold(text)),
        _ => None,
    };
    let mismatch = || {
        Reason::Mismatch(format!(
            "`{}` does not compare {} with {}",
            operator_name(operator),
            left.kind(),
            right.kind()
        ))
    };
    match operator {
        C::Like | C::Matches | C::Contains => {
            let (Some(subject), Some(pattern)) = (text(left), text(right)) else {
                return Err(mismatch());
            };
            if operator == C::Contains {
                return Ok(subject.contains(&pattern));
            }
            let source = if operator == C::Like {
                crate::wildcard_regex(&pattern).map_err(Reason::InvalidPattern)?
            } else {
                format!("^(?:{pattern})$")
            };
            let regex = regex::RegexBuilder::new(&source)
                .case_insensitive(!case_sensitive)
                .build()
                .map_err(|error| Reason::InvalidPattern(error.to_string()))?;
            Ok(regex.is_match(&subject))
        }
        ordered => {
            let order = match (left, right) {
                (
                    Value::Number {
                        value: left_value,
                        unit: left_unit,
                    },
                    Value::Number {
                        value: right_value,
                        unit: right_unit,
                    },
                ) => {
                    if left_unit != right_unit {
                        return Err(Reason::Mismatch(format!(
                            "`{}` compares {left_unit} with {right_unit}, which differ",
                            operator_name(operator)
                        )));
                    }
                    return decide(ordered, *left_value, *right_value).ok_or_else(|| {
                        Reason::Straddles {
                            left: Box::new(left.clone()),
                            right: Box::new(right.clone()),
                        }
                    });
                }
                (Value::Boolean(left), Value::Boolean(right))
                    if matches!(ordered, C::Equals | C::NotEquals) =>
                {
                    left.cmp(right)
                }
                (Value::Date(left), Value::Date(right)) => left.cmp(right),
                (Value::DateTime(left), Value::DateTime(right)) => left.cmp_instant(*right),
                _ => match (text(left), text(right)) {
                    (Some(left), Some(right)) => left.cmp(&right),
                    _ => return Err(mismatch()),
                },
            };
            Ok(holds(ordered, order))
        }
    }
}

fn holds(operator: ExpressionComparison, order: Order) -> bool {
    use ExpressionComparison as C;
    match operator {
        C::Equals => order == Order::Equal,
        C::NotEquals => order != Order::Equal,
        C::LessThan => order == Order::Less,
        C::LessThanOrEquals => order != Order::Greater,
        C::GreaterThan => order == Order::Greater,
        C::GreaterThanOrEquals => order != Order::Less,
        C::Like | C::Matches | C::Contains => unreachable!("not an ordering"),
    }
}

/// An ordered comparison of two intervals, when every pair of values they
/// allow answers it alike.
fn decide(operator: ExpressionComparison, left: Interval, right: Interval) -> Option<bool> {
    use ExpressionComparison as C;
    let below = left.upper < right.lower;
    let above = left.lower > right.upper;
    let equal = left.is_point() && right.is_point() && left == right;
    let at_most = left.upper <= right.lower;
    let at_least = left.lower >= right.upper;
    match operator {
        C::Equals => (equal || below || above).then_some(equal),
        C::NotEquals => (equal || below || above).then_some(!equal),
        C::LessThan => (below || at_least).then_some(below),
        C::LessThanOrEquals => (at_most || above).then_some(at_most),
        C::GreaterThan => (above || at_most).then_some(above),
        C::GreaterThanOrEquals => (at_least || below).then_some(at_least),
        C::Like | C::Matches | C::Contains => None,
    }
}

fn operator_name(operator: ExpressionComparison) -> &'static str {
    use ExpressionComparison as C;
    match operator {
        C::Equals => "equals",
        C::NotEquals => "notEquals",
        C::LessThan => "lessThan",
        C::LessThanOrEquals => "lessThanOrEquals",
        C::GreaterThan => "greaterThan",
        C::GreaterThanOrEquals => "greaterThanOrEquals",
        C::Like => "like",
        C::Matches => "matches",
        C::Contains => "contains",
    }
}
