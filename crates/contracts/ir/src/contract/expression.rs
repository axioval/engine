//! Typed expressions: values a rule computes and combines, as data.
//!
//! The language is total. It has no loops, no recursion, no user-defined
//! functions and no package-provided code, so evaluating a finite tree
//! always terminates. The engine evaluates it; this module only states it.
use super::ParameterValue;
use crate::{Date, DateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How deeply an expression may nest; a deeper one is refused by
/// [`Expression::validate`].
pub const MAX_EXPRESSION_DEPTH: usize = 64;

/// A node of an expression tree, tagged by `kind` like a [`Selector`].
///
/// Every node may carry an author `label` that findings name in place of
/// the node's rendered form. Operands are evaluated by the engine over
/// intervals with three-valued truth; `null` is a value the source states
/// as absent, never a value that could not be read.
///
/// [`Selector`]: super::Selector
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Expression {
    /// A constant of one scalar kind.
    Literal {
        value: ScalarValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The source states no value.
    Null {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A property of the object in scope, as a property selector names it;
    /// `axioval:measured` names carry their parameters
    /// (`bottom_above_level;path=…`).
    Property {
        #[serde(
            rename = "propertySet",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        property_set: Option<String>,
        property: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A parameter of the rule, by name.
    Parameter {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A derived value the ruleset names.
    Derived {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The `column` cell of the most specific row of the `table` parameter
    /// whose key columns match `keys`, as `keyed-limit` selects a row.
    Lookup {
        table: String,
        /// The value each key column is matched against, by column ID.
        keys: BTreeMap<String, Expression>,
        column: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Not {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    And {
        operands: Vec<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Or {
        operands: Vec<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// `antecedent → consequent`.
    Implies {
        antecedent: Box<Expression>,
        consequent: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Xor {
        left: Box<Expression>,
        right: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// `left operator right`.
    Compare {
        operator: ExpressionComparison,
        left: Box<Expression>,
        right: Box<Expression>,
        /// Whether text comparisons respect case; `false` folds both sides.
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// `low ≤ operand ≤ high`, each bound inclusive unless stated
    /// otherwise.
    Between {
        operand: Box<Expression>,
        low: Box<Expression>,
        high: Box<Expression>,
        #[serde(
            rename = "lowInclusive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        low_inclusive: bool,
        #[serde(
            rename = "highInclusive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        high_inclusive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Whether `operand` equals one of `values`.
    OneOf {
        operand: Box<Expression>,
        values: Vec<Expression>,
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Whether `operand` equals none of `values`.
    NoneOf {
        operand: Box<Expression>,
        values: Vec<Expression>,
        #[serde(
            rename = "caseSensitive",
            default = "yes",
            skip_serializing_if = "is_true"
        )]
        case_sensitive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Whether `operand` has a value: not `null`.
    IsDefined {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Whether `operand` is `null`.
    IsUndefined {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The `then` of the first branch whose `when` holds, else `else`. A
    /// ternary is the one-branch case.
    If {
        branches: Vec<Branch>,
        #[serde(rename = "else")]
        otherwise: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The first operand that is not `null`.
    Coalesce {
        operands: Vec<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Add {
        left: Box<Expression>,
        right: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Subtract {
        left: Box<Expression>,
        right: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Multiply {
        left: Box<Expression>,
        right: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Divide {
        left: Box<Expression>,
        right: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Unary minus.
    Negate {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Abs {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Min {
        operands: Vec<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Max {
        operands: Vec<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// `operand` rounded to the nearest multiple of `step`.
    Round {
        operand: Box<Expression>,
        step: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Floor {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Ceil {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Sqrt {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The sine of a plane angle.
    Sin {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The cosine of a plane angle.
    Cos {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The tangent of a plane angle.
    Tan {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The plane angle of the vector `(x, y)`.
    Atan2 {
        y: Box<Expression>,
        x: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A slope restated in another form: `1:12` as a ratio is `8.33` as a
    /// percentage and about `4.76°` as an angle.
    ConvertSlope {
        operand: Box<Expression>,
        from: SlopeForm,
        to: SlopeForm,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Concat {
        operands: Vec<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The number of characters of a text.
    Length {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Lower {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Upper {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Trim {
        operand: Box<Expression>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

/// One `when` → `then` branch of an [`Expression::If`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Branch {
    pub when: Expression,
    pub then: Expression,
}

/// The binary comparisons of an [`Expression::Compare`].
///
/// The ordered operators also take dates and date-times, compared
/// chronologically. `like` is a wildcard pattern (`*`, `?`, `\` escapes)
/// and `matches` a regular expression, both over the whole text;
/// `contains` asks whether the right text occurs in the left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExpressionComparison {
    Equals,
    NotEquals,
    LessThan,
    LessThanOrEquals,
    GreaterThan,
    GreaterThanOrEquals,
    Like,
    Matches,
    Contains,
}

/// How a slope is stated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlopeForm {
    /// Rise over run, a plain number.
    Ratio,
    /// Rise over run times 100, a plain number.
    Percent,
    /// The inclination, a plane angle.
    Angle,
}

/// A literal: one value of a scalar [`ParameterValue`] kind, in the same
/// wire form. Lists, tables, references and selectors are not literals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ScalarValue {
    Boolean { value: bool },
    Integer { value: i64 },
    Number { value: f64 },
    Quantity { value: f64, unit: String },
    String { value: String },
    Enum { value: String },
    Date { value: Date },
    DateTime { value: DateTime },
}

impl From<ScalarValue> for ParameterValue {
    fn from(value: ScalarValue) -> Self {
        match value {
            ScalarValue::Boolean { value } => Self::Boolean { value },
            ScalarValue::Integer { value } => Self::Integer { value },
            ScalarValue::Number { value } => Self::Number { value },
            ScalarValue::Quantity { value, unit } => Self::Quantity { value, unit },
            ScalarValue::String { value } => Self::String { value },
            ScalarValue::Enum { value } => Self::Enum { value },
            ScalarValue::Date { value } => Self::Date { value },
            ScalarValue::DateTime { value } => Self::DateTime { value },
        }
    }
}

impl TryFrom<ParameterValue> for ScalarValue {
    type Error = ParameterValue;

    /// The scalar a parameter value states; any other value is handed back.
    fn try_from(value: ParameterValue) -> Result<Self, Self::Error> {
        Ok(match value {
            ParameterValue::Boolean { value } => Self::Boolean { value },
            ParameterValue::Integer { value } => Self::Integer { value },
            ParameterValue::Number { value } => Self::Number { value },
            ParameterValue::Quantity { value, unit } => Self::Quantity { value, unit },
            ParameterValue::String { value } => Self::String { value },
            ParameterValue::Enum { value } => Self::Enum { value },
            ParameterValue::Date { value } => Self::Date { value },
            ParameterValue::DateTime { value } => Self::DateTime { value },
            other => return Err(other),
        })
    }
}

/// Why an expression's structure is refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExpressionError {
    #[error("an expression nests deeper than {MAX_EXPRESSION_DEPTH} levels")]
    TooDeep,
    #[error("a `{kind}` expression has no operands")]
    NoOperands { kind: &'static str },
    #[error("an `if` expression has no branch")]
    NoBranches,
    #[error("a `{kind}` expression names a blank {field}")]
    Blank {
        kind: &'static str,
        field: &'static str,
    },
    #[error("a `lookup` expression of `{table}` matches no key column")]
    NoKeys { table: String },
    #[error("a literal number is not finite")]
    NotFinite,
}

impl Expression {
    /// Every node `kind`, in declaration order.
    pub const KINDS: [&'static str; 41] = [
        "literal",
        "null",
        "property",
        "parameter",
        "derived",
        "lookup",
        "not",
        "and",
        "or",
        "implies",
        "xor",
        "compare",
        "between",
        "oneOf",
        "noneOf",
        "isDefined",
        "isUndefined",
        "if",
        "coalesce",
        "add",
        "subtract",
        "multiply",
        "divide",
        "negate",
        "abs",
        "min",
        "max",
        "round",
        "floor",
        "ceil",
        "sqrt",
        "sin",
        "cos",
        "tan",
        "atan2",
        "convertSlope",
        "concat",
        "length",
        "lower",
        "upper",
        "trim",
    ];

    /// The node's `kind` as written in a package.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Literal { .. } => "literal",
            Self::Null { .. } => "null",
            Self::Property { .. } => "property",
            Self::Parameter { .. } => "parameter",
            Self::Derived { .. } => "derived",
            Self::Lookup { .. } => "lookup",
            Self::Not { .. } => "not",
            Self::And { .. } => "and",
            Self::Or { .. } => "or",
            Self::Implies { .. } => "implies",
            Self::Xor { .. } => "xor",
            Self::Compare { .. } => "compare",
            Self::Between { .. } => "between",
            Self::OneOf { .. } => "oneOf",
            Self::NoneOf { .. } => "noneOf",
            Self::IsDefined { .. } => "isDefined",
            Self::IsUndefined { .. } => "isUndefined",
            Self::If { .. } => "if",
            Self::Coalesce { .. } => "coalesce",
            Self::Add { .. } => "add",
            Self::Subtract { .. } => "subtract",
            Self::Multiply { .. } => "multiply",
            Self::Divide { .. } => "divide",
            Self::Negate { .. } => "negate",
            Self::Abs { .. } => "abs",
            Self::Min { .. } => "min",
            Self::Max { .. } => "max",
            Self::Round { .. } => "round",
            Self::Floor { .. } => "floor",
            Self::Ceil { .. } => "ceil",
            Self::Sqrt { .. } => "sqrt",
            Self::Sin { .. } => "sin",
            Self::Cos { .. } => "cos",
            Self::Tan { .. } => "tan",
            Self::Atan2 { .. } => "atan2",
            Self::ConvertSlope { .. } => "convertSlope",
            Self::Concat { .. } => "concat",
            Self::Length { .. } => "length",
            Self::Lower { .. } => "lower",
            Self::Upper { .. } => "upper",
            Self::Trim { .. } => "trim",
        }
    }

    /// The author's label, if the node carries one.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Literal { label, .. }
            | Self::Null { label }
            | Self::Property { label, .. }
            | Self::Parameter { label, .. }
            | Self::Derived { label, .. }
            | Self::Lookup { label, .. }
            | Self::Not { label, .. }
            | Self::And { label, .. }
            | Self::Or { label, .. }
            | Self::Implies { label, .. }
            | Self::Xor { label, .. }
            | Self::Compare { label, .. }
            | Self::Between { label, .. }
            | Self::OneOf { label, .. }
            | Self::NoneOf { label, .. }
            | Self::IsDefined { label, .. }
            | Self::IsUndefined { label, .. }
            | Self::If { label, .. }
            | Self::Coalesce { label, .. }
            | Self::Add { label, .. }
            | Self::Subtract { label, .. }
            | Self::Multiply { label, .. }
            | Self::Divide { label, .. }
            | Self::Negate { label, .. }
            | Self::Abs { label, .. }
            | Self::Min { label, .. }
            | Self::Max { label, .. }
            | Self::Round { label, .. }
            | Self::Floor { label, .. }
            | Self::Ceil { label, .. }
            | Self::Sqrt { label, .. }
            | Self::Sin { label, .. }
            | Self::Cos { label, .. }
            | Self::Tan { label, .. }
            | Self::Atan2 { label, .. }
            | Self::ConvertSlope { label, .. }
            | Self::Concat { label, .. }
            | Self::Length { label, .. }
            | Self::Lower { label, .. }
            | Self::Upper { label, .. }
            | Self::Trim { label, .. } => label.as_deref(),
        }
    }

    /// The node's direct operands, in written order: a lookup's keys by
    /// column ID, an `if`'s branches `when` before `then`, `else` last.
    #[must_use]
    pub fn children(&self) -> Vec<&Self> {
        match self {
            Self::Literal { .. }
            | Self::Null { .. }
            | Self::Property { .. }
            | Self::Parameter { .. }
            | Self::Derived { .. } => Vec::new(),
            Self::Lookup { keys, .. } => keys.values().collect(),
            Self::Not { operand, .. }
            | Self::IsDefined { operand, .. }
            | Self::IsUndefined { operand, .. }
            | Self::Negate { operand, .. }
            | Self::Abs { operand, .. }
            | Self::Floor { operand, .. }
            | Self::Ceil { operand, .. }
            | Self::Sqrt { operand, .. }
            | Self::Sin { operand, .. }
            | Self::Cos { operand, .. }
            | Self::Tan { operand, .. }
            | Self::ConvertSlope { operand, .. }
            | Self::Length { operand, .. }
            | Self::Lower { operand, .. }
            | Self::Upper { operand, .. }
            | Self::Trim { operand, .. } => vec![operand],
            Self::And { operands, .. }
            | Self::Or { operands, .. }
            | Self::Coalesce { operands, .. }
            | Self::Min { operands, .. }
            | Self::Max { operands, .. }
            | Self::Concat { operands, .. } => operands.iter().collect(),
            Self::Implies {
                antecedent,
                consequent,
                ..
            } => vec![antecedent, consequent],
            Self::Xor { left, right, .. }
            | Self::Compare { left, right, .. }
            | Self::Add { left, right, .. }
            | Self::Subtract { left, right, .. }
            | Self::Multiply { left, right, .. }
            | Self::Divide { left, right, .. } => vec![left, right],
            Self::Between {
                operand, low, high, ..
            } => vec![operand, low, high],
            Self::OneOf {
                operand, values, ..
            }
            | Self::NoneOf {
                operand, values, ..
            } => std::iter::once(operand.as_ref()).chain(values).collect(),
            Self::If {
                branches,
                otherwise,
                ..
            } => branches
                .iter()
                .flat_map(|branch| [&branch.when, &branch.then])
                .chain(std::iter::once(otherwise.as_ref()))
                .collect(),
            Self::Round { operand, step, .. } => vec![operand, step],
            Self::Atan2 { y, x, .. } => vec![y, x],
        }
    }

    /// Checks what serde cannot: nesting depth, non-empty operand lists
    /// and branches, names and labels that are not blank, and finite
    /// literal numbers. Types and units are checked when a ruleset is
    /// compiled, not here.
    ///
    /// # Errors
    ///
    /// The first structural problem found, depth first.
    pub fn validate(&self) -> Result<(), ExpressionError> {
        self.validate_at(1)
    }

    fn validate_at(&self, depth: usize) -> Result<(), ExpressionError> {
        if depth > MAX_EXPRESSION_DEPTH {
            return Err(ExpressionError::TooDeep);
        }
        let kind = self.kind();
        if self.label().is_some_and(|label| label.trim().is_empty()) {
            return Err(ExpressionError::Blank {
                kind,
                field: "label",
            });
        }
        match self {
            Self::Literal {
                value: ScalarValue::Number { value } | ScalarValue::Quantity { value, .. },
                ..
            } if !value.is_finite() => return Err(ExpressionError::NotFinite),
            Self::Literal {
                value: ScalarValue::Quantity { unit, .. },
                ..
            } => blank(kind, "unit", unit)?,
            Self::Property { property, .. } => blank(kind, "property", property)?,
            Self::Parameter { name, .. } | Self::Derived { name, .. } => {
                blank(kind, "name", name)?;
            }
            Self::Lookup {
                table,
                keys,
                column,
                ..
            } => {
                blank(kind, "table", table)?;
                blank(kind, "column", column)?;
                if keys.is_empty() {
                    return Err(ExpressionError::NoKeys {
                        table: table.clone(),
                    });
                }
                for key in keys.keys() {
                    blank(kind, "key column", key)?;
                }
            }
            Self::And { operands, .. }
            | Self::Or { operands, .. }
            | Self::Coalesce { operands, .. }
            | Self::Min { operands, .. }
            | Self::Max { operands, .. }
            | Self::Concat { operands, .. }
            | Self::OneOf {
                values: operands, ..
            }
            | Self::NoneOf {
                values: operands, ..
            } if operands.is_empty() => return Err(ExpressionError::NoOperands { kind }),
            Self::If { branches, .. } if branches.is_empty() => {
                return Err(ExpressionError::NoBranches);
            }
            _ => {}
        }
        self.children()
            .into_iter()
            .try_for_each(|child| child.validate_at(depth + 1))
    }
}

fn blank(kind: &'static str, field: &'static str, text: &str) -> Result<(), ExpressionError> {
    if text.trim().is_empty() {
        Err(ExpressionError::Blank { kind, field })
    } else {
        Ok(())
    }
}

const fn yes() -> bool {
    true
}
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_true(value: &bool) -> bool {
    *value
}
