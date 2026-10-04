//! Type and unit checking of an [`Expression`] before it is evaluated.
//!
//! Every node's type is inferred bottom up. Units combine through
//! arithmetic (`m × m = m²`, `m ÷ m` is a plain number); `+`, `−`,
//! comparisons, `min`, `max`, `round`, and the branches of `if` and
//! `coalesce` need one unit, and a quantity never meets a plain number.
//! A leaf whose type is known only when read is [`Type::Any`]: it passes
//! here and is checked when evaluated, where a mismatch is not evaluated.
//! An error names the subexpression by the same path the evaluator uses.
use std::collections::BTreeMap;
use std::fmt;

use axioval_ir::contract::{Expression, ExpressionComparison, ScalarValue, SlopeForm};

use super::unit::{Unit, parse_unit};

/// The type of an expression's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// A truth.
    Boolean,
    /// A whole plain number.
    Integer,
    /// A number of `unit`; [`Unit::NONE`] for a plain number.
    Number(Unit),
    /// Text.
    Text,
    /// A value of an enumeration, of these values when they are declared.
    Enum(Option<Vec<String>>),
    /// A calendar date.
    Date,
    /// A date-time.
    DateTime,
    /// Only `null`: the literal `null`.
    Null,
    /// Known only when read; checked when evaluated.
    Any,
}

impl Type {
    /// A plain number.
    pub const NUMBER: Self = Self::Number(Unit::NONE);

    fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Integer | Self::Number(_) | Self::Null | Self::Any
        )
    }

    fn unit(&self) -> Option<Unit> {
        match self {
            Self::Integer => Some(Unit::NONE),
            Self::Number(unit) => Some(unit.clone()),
            _ => None,
        }
    }

    fn is_textual(&self) -> bool {
        matches!(self, Self::Text | Self::Enum(_) | Self::Null | Self::Any)
    }

    fn is_loose(&self) -> bool {
        matches!(self, Self::Null | Self::Any)
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean => f.write_str("a truth"),
            Self::Integer => f.write_str("an integer"),
            Self::Number(unit) if unit.is_plain() => f.write_str("a number"),
            Self::Number(unit) => write!(f, "a quantity in {unit}"),
            Self::Text => f.write_str("text"),
            Self::Enum(_) => f.write_str("an enumeration value"),
            Self::Date => f.write_str("a date"),
            Self::DateTime => f.write_str("a date-time"),
            Self::Null => f.write_str("null"),
            Self::Any => f.write_str("a value of unknown type"),
        }
    }
}

/// What a type error is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeErrorKind {
    /// A property, parameter, derived value, table or column that does not
    /// exist, or a measured name the registry refuses.
    Unknown(String),
    /// A truth is needed.
    NotBoolean(Type),
    /// A number or quantity is needed.
    NotNumeric(Type),
    /// Text is needed.
    NotText(Type),
    /// A plane angle is needed.
    NotAngle(Type),
    /// Two units that must be one differ; a plain number is unit `1`.
    UnitMismatch {
        /// What combines them: `adds`, `subtracts`, `compares`, `combines`.
        operation: &'static str,
        /// The first operand's unit.
        left: Unit,
        /// The other's.
        right: Unit,
    },
    /// Two values that must be of one type are not.
    Mismatch {
        /// The first operand's type.
        left: Type,
        /// The other's.
        right: Type,
    },
    /// A unit has no square root (`m³`).
    NoSquareRoot(Unit),
    /// A literal unit does not parse.
    InvalidUnit(String),
    /// A literal pattern does not compile.
    InvalidPattern(String),
    /// The value is not of the type its place needs: a requirement must be
    /// a truth, a parameter of its declared kind.
    Expected {
        /// The type needed.
        expected: Type,
        /// The type found.
        found: Type,
    },
}

impl fmt::Display for TypeErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(what) => f.write_str(what),
            Self::NotBoolean(found) => write!(f, "a truth is needed, not {found}"),
            Self::NotNumeric(found) => write!(f, "a number is needed, not {found}"),
            Self::NotText(found) => write!(f, "text is needed, not {found}"),
            Self::NotAngle(found) => write!(f, "a plane angle is needed, not {found}"),
            // As takeoff columns have always worded it: the added unit first.
            Self::UnitMismatch {
                operation,
                left,
                right,
            } => write!(f, "it {operation} {right} and {left}, which differ"),
            Self::Mismatch { left, right } => {
                write!(f, "it combines {left} and {right}, which differ")
            }
            Self::NoSquareRoot(unit) => write!(f, "{unit} has no square root"),
            Self::InvalidUnit(why) | Self::InvalidPattern(why) => f.write_str(why),
            Self::Expected { expected, found } => write!(f, "{expected} is needed, not {found}"),
        }
    }
}

/// A type error at a subexpression.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{path}`: {kind}")]
pub struct TypeError {
    /// Where, such as `requirement.and[2].compare.left`.
    pub path: String,
    /// What is wrong.
    pub kind: TypeErrorKind,
}

/// The types of the leaves an expression may read.
pub trait TypeEnvironment {
    /// A property's type; [`Type::Any`] when only a read can tell.
    ///
    /// # Errors
    ///
    /// Why the property cannot be read here.
    fn property(&self, set: Option<&str>, name: &str) -> Result<Type, String>;

    /// A rule parameter's type.
    ///
    /// # Errors
    ///
    /// The parameter is not declared.
    fn parameter(&self, name: &str) -> Result<Type, String>;

    /// A derived value's type.
    ///
    /// # Errors
    ///
    /// The derived value is not declared.
    fn derived(&self, name: &str) -> Result<Type, String> {
        Err(format!("no derived value `{name}` is declared"))
    }

    /// The types of a table parameter's key columns and of `column`.
    ///
    /// # Errors
    ///
    /// The table or the column is not declared.
    fn lookup(&self, table: &str, column: &str) -> Result<(BTreeMap<String, Type>, Type), String> {
        let _ = column;
        Err(format!("no table `{table}` is declared"))
    }
}

/// The type of a measured value of `axioval:measured`, from the registry.
///
/// # Errors
///
/// The registry's message for an unknown name or a wrong parameter.
pub fn measured_type(name: &str) -> Result<Type, String> {
    let call = axioval_ir::measured::parse(name).map_err(|error| error.to_string())?;
    Ok(Type::Number(Unit::of(Some(call.descriptor.dimension))))
}

/// Infers the type of `expression`, whose path is `root`.
///
/// # Errors
///
/// The first type error found, depth first.
pub fn check(
    expression: &Expression,
    root: &str,
    environment: &dyn TypeEnvironment,
) -> Result<Type, TypeError> {
    Checker { environment }.check(expression, root)
}

/// Checks that `expression` is of type `expected` (a truth for a
/// requirement, a parameter's kind for a parameter expression).
///
/// # Errors
///
/// A type error inside, or a value of another type.
pub fn check_as(
    expression: &Expression,
    root: &str,
    expected: &Type,
    environment: &dyn TypeEnvironment,
) -> Result<(), TypeError> {
    let found = check(expression, root, environment)?;
    if join(expected, &found).as_ref() == Some(expected) || found.is_loose() {
        return Ok(());
    }
    Err(TypeError {
        path: root.to_owned(),
        kind: TypeErrorKind::Expected {
            expected: expected.clone(),
            found,
        },
    })
}

/// The one type two values of types `left` and `right` share, if any: the
/// type of an `if` or a `coalesce` over them.
fn join(left: &Type, right: &Type) -> Option<Type> {
    use Type as T;
    Some(match (left, right) {
        (T::Any, _) | (_, T::Any) => T::Any,
        (T::Null, other) | (other, T::Null) => other.clone(),
        (T::Integer, T::Integer) => T::Integer,
        (T::Integer, T::Number(unit)) | (T::Number(unit), T::Integer) if unit.is_plain() => {
            T::NUMBER
        }
        (T::Number(left), T::Number(right)) if left == right => T::Number(left.clone()),
        (T::Enum(left), T::Enum(right)) => T::Enum(match (left, right) {
            (Some(left), Some(right)) => {
                let mut values: Vec<String> = left.iter().chain(right).cloned().collect();
                values.sort();
                values.dedup();
                Some(values)
            }
            _ => None,
        }),
        (T::Enum(_) | T::Text, T::Enum(_) | T::Text) => T::Text,
        (left, right) if left == right => left.clone(),
        _ => return None,
    })
}

struct Checker<'e> {
    environment: &'e dyn TypeEnvironment,
}

impl Checker<'_> {
    #[allow(clippy::too_many_lines)]
    fn check(&self, expression: &Expression, path: &str) -> Result<Type, TypeError> {
        let kind = expression.kind();
        let child = |field: &str| format!("{path}.{kind}.{field}");
        let item = |index: usize| format!("{path}.{kind}[{index}]");
        let here = |kind: TypeErrorKind| TypeError {
            path: path.to_owned(),
            kind,
        };
        Ok(match expression {
            Expression::Literal { value, .. } => match value {
                ScalarValue::Boolean { .. } => Type::Boolean,
                ScalarValue::Integer { .. } => Type::Integer,
                ScalarValue::Number { .. } => Type::NUMBER,
                ScalarValue::Quantity { unit, .. } => Type::Number(
                    parse_unit(unit)
                        .map_err(|why| here(TypeErrorKind::InvalidUnit(why)))?
                        .1,
                ),
                ScalarValue::String { .. } => Type::Text,
                ScalarValue::Enum { value } => Type::Enum(Some(vec![value.clone()])),
                ScalarValue::Date { .. } => Type::Date,
                ScalarValue::DateTime { .. } => Type::DateTime,
            },
            Expression::Null { .. } => Type::Null,
            Expression::Property {
                property_set,
                property,
                ..
            } => self
                .environment
                .property(property_set.as_deref(), property)
                .map_err(|why| here(TypeErrorKind::Unknown(why)))?,
            Expression::Parameter { name, .. } => self
                .environment
                .parameter(name)
                .map_err(|why| here(TypeErrorKind::Unknown(why)))?,
            Expression::Derived { name, .. } => self
                .environment
                .derived(name)
                .map_err(|why| here(TypeErrorKind::Unknown(why)))?,
            Expression::Lookup {
                table,
                keys,
                column,
                ..
            } => {
                let (key_types, result) = self
                    .environment
                    .lookup(table, column)
                    .map_err(|why| here(TypeErrorKind::Unknown(why)))?;
                for (key, operand) in keys {
                    let key_path = format!("{path}.lookup.keys[{key}]");
                    let found = self.check(operand, &key_path)?;
                    let expected = key_types.get(key).ok_or_else(|| {
                        here(TypeErrorKind::Unknown(format!(
                            "table `{table}` has no key column `{key}`"
                        )))
                    })?;
                    if join(expected, &found).is_none() {
                        return Err(TypeError {
                            path: key_path,
                            kind: TypeErrorKind::Expected {
                                expected: expected.clone(),
                                found,
                            },
                        });
                    }
                }
                result
            }
            Expression::Not { operand, .. } => {
                self.boolean(operand, &child("operand"))?;
                Type::Boolean
            }
            Expression::And { operands, .. } | Expression::Or { operands, .. } => {
                for (index, operand) in operands.iter().enumerate() {
                    self.boolean(operand, &item(index))?;
                }
                Type::Boolean
            }
            Expression::Implies {
                antecedent,
                consequent,
                ..
            } => {
                self.boolean(antecedent, &child("antecedent"))?;
                self.boolean(consequent, &child("consequent"))?;
                Type::Boolean
            }
            Expression::Xor { left, right, .. } => {
                self.boolean(left, &child("left"))?;
                self.boolean(right, &child("right"))?;
                Type::Boolean
            }
            Expression::Compare {
                operator,
                left,
                right,
                case_sensitive,
                ..
            } => {
                let left_type = self.check(left, &child("left"))?;
                let right_type = self.check(right, &child("right"))?;
                comparable(*operator, &left_type, &right_type).map_err(here)?;
                if let (
                    ExpressionComparison::Matches | ExpressionComparison::Like,
                    Expression::Literal {
                        value: ScalarValue::String { value },
                        ..
                    },
                ) = (operator, right.as_ref())
                {
                    pattern(*operator, value, *case_sensitive).map_err(|why| TypeError {
                        path: child("right"),
                        kind: TypeErrorKind::InvalidPattern(why),
                    })?;
                }
                Type::Boolean
            }
            Expression::Between {
                operand, low, high, ..
            } => {
                let value = self.check(operand, &child("operand"))?;
                let low = self.check(low, &child("low"))?;
                let high = self.check(high, &child("high"))?;
                comparable(ExpressionComparison::LessThan, &low, &value).map_err(here)?;
                comparable(ExpressionComparison::LessThan, &value, &high).map_err(here)?;
                Type::Boolean
            }
            Expression::OneOf {
                operand, values, ..
            }
            | Expression::NoneOf {
                operand, values, ..
            } => {
                let value = self.check(operand, &child("operand"))?;
                for (index, candidate) in values.iter().enumerate() {
                    let candidate_path = format!("{path}.{kind}.values[{index}]");
                    let candidate = self.check(candidate, &candidate_path)?;
                    comparable(ExpressionComparison::Equals, &value, &candidate).map_err(
                        |kind| TypeError {
                            path: candidate_path.clone(),
                            kind,
                        },
                    )?;
                }
                Type::Boolean
            }
            Expression::IsDefined { operand, .. } | Expression::IsUndefined { operand, .. } => {
                self.check(operand, &child("operand"))?;
                Type::Boolean
            }
            Expression::If {
                branches,
                otherwise,
                ..
            } => {
                let mut result = Type::Null;
                for (index, branch) in branches.iter().enumerate() {
                    self.boolean(&branch.when, &format!("{path}.if.branches[{index}].when"))?;
                    let then_path = format!("{path}.if.branches[{index}].then");
                    let then = self.check(&branch.then, &then_path)?;
                    result = joined(&result, then, &then_path)?;
                }
                let else_path = format!("{path}.if.else");
                let otherwise = self.check(otherwise, &else_path)?;
                joined(&result, otherwise, &else_path)?
            }
            Expression::Coalesce { operands, .. } => {
                let mut result = Type::Null;
                for (index, operand) in operands.iter().enumerate() {
                    let found = self.check(operand, &item(index))?;
                    result = joined(&result, found, &item(index))?;
                }
                result
            }
            Expression::Add { left, right, .. } | Expression::Subtract { left, right, .. } => {
                let left = self.numeric(left, &child("left"))?;
                let right = self.numeric(right, &child("right"))?;
                if left == Type::Integer && right == Type::Integer {
                    Type::Integer
                } else {
                    let operation = if matches!(expression, Expression::Add { .. }) {
                        "adds"
                    } else {
                        "subtracts"
                    };
                    same_unit(operation, &left, &right).map_err(here)?
                }
            }
            Expression::Multiply { left, right, .. } | Expression::Divide { left, right, .. } => {
                let left = self.numeric(left, &child("left"))?;
                let right = self.numeric(right, &child("right"))?;
                let dividing = matches!(expression, Expression::Divide { .. });
                match (left.unit(), right.unit()) {
                    _ if !dividing && left == Type::Integer && right == Type::Integer => {
                        Type::Integer
                    }
                    (Some(left), Some(right)) => Type::Number(
                        left.times(&right, if dividing { -1 } else { 1 })
                            .map_err(|why| here(TypeErrorKind::InvalidUnit(why)))?,
                    ),
                    _ => Type::Any,
                }
            }
            Expression::Negate { operand, .. } | Expression::Abs { operand, .. } => {
                self.numeric(operand, &child("operand"))?
            }
            Expression::Floor { operand, .. } | Expression::Ceil { operand, .. } => {
                match self.numeric(operand, &child("operand"))? {
                    Type::Number(unit) if unit.is_plain() => Type::Integer,
                    other => other,
                }
            }
            Expression::Min { operands, .. } | Expression::Max { operands, .. } => {
                let mut result = Type::Null;
                for (index, operand) in operands.iter().enumerate() {
                    let found = self.numeric(operand, &item(index))?;
                    result = same_unit("compares", &result, &found).map_err(|kind| TypeError {
                        path: item(index),
                        kind,
                    })?;
                }
                result
            }
            Expression::Round { operand, step, .. } => {
                let value = self.numeric(operand, &child("operand"))?;
                let step = self.numeric(step, &child("step"))?;
                same_unit("rounds", &value, &step).map_err(here)?
            }
            Expression::Sqrt { operand, .. } => match self.numeric(operand, &child("operand"))? {
                Type::Integer => Type::NUMBER,
                Type::Number(unit) => Type::Number(
                    unit.sqrt()
                        .ok_or_else(|| here(TypeErrorKind::NoSquareRoot(unit)))?,
                ),
                other => other,
            },
            Expression::Sin { operand, .. }
            | Expression::Cos { operand, .. }
            | Expression::Tan { operand, .. } => {
                self.angle(operand, &child("operand"))?;
                Type::NUMBER
            }
            Expression::Atan2 { y, x, .. } => {
                let y = self.numeric(y, &child("y"))?;
                let x = self.numeric(x, &child("x"))?;
                same_unit("combines", &y, &x).map_err(here)?;
                Type::Number(Unit::RADIAN)
            }
            Expression::ConvertSlope {
                operand, from, to, ..
            } => {
                if *from == SlopeForm::Angle {
                    self.angle(operand, &child("operand"))?;
                } else {
                    let found = self.numeric(operand, &child("operand"))?;
                    if found.unit().is_some_and(|unit| !unit.is_plain()) {
                        return Err(TypeError {
                            path: child("operand"),
                            kind: TypeErrorKind::Expected {
                                expected: Type::NUMBER,
                                found,
                            },
                        });
                    }
                }
                if *to == SlopeForm::Angle {
                    Type::Number(Unit::RADIAN)
                } else {
                    Type::NUMBER
                }
            }
            Expression::Aggregate {
                function,
                filter,
                value,
                ..
            } => {
                use axioval_ir::contract::AggregateFunction as F;
                if let Some(filter) = filter {
                    for nested in filter.expressions() {
                        self.boolean(nested, &child("where"))?;
                    }
                }
                let value_path = child("value");
                match (function, value) {
                    (F::Count | F::DistinctCount, value) => {
                        if let Some(value) = value {
                            self.check(value, &value_path)?;
                        }
                        Type::Integer
                    }
                    (F::Any | F::All | F::None, Some(value)) => {
                        self.boolean(value, &value_path)?;
                        Type::Boolean
                    }
                    (F::Sum | F::Min | F::Max, Some(value)) => {
                        match self.numeric(value, &value_path)? {
                            Type::Null => Type::Any,
                            found => found,
                        }
                    }
                    (F::Average, Some(value)) => match self.numeric(value, &value_path)? {
                        Type::Integer => Type::NUMBER,
                        Type::Null => Type::Any,
                        found => found,
                    },
                    (_, None) => Type::Any,
                }
            }
            Expression::RuleOutcome { .. } => Type::Boolean,
            Expression::FindingCount { .. } => Type::Integer,
            Expression::Deviation { .. } => Type::NUMBER,
            Expression::Concat { operands, .. } => {
                for (index, operand) in operands.iter().enumerate() {
                    self.text(operand, &item(index))?;
                }
                Type::Text
            }
            Expression::Length { operand, .. } => {
                self.text(operand, &child("operand"))?;
                Type::Integer
            }
            Expression::Lower { operand, .. }
            | Expression::Upper { operand, .. }
            | Expression::Trim { operand, .. } => {
                self.text(operand, &child("operand"))?;
                Type::Text
            }
        })
    }

    fn boolean(&self, expression: &Expression, path: &str) -> Result<(), TypeError> {
        match self.check(expression, path)? {
            Type::Boolean | Type::Null | Type::Any => Ok(()),
            other => Err(TypeError {
                path: path.to_owned(),
                kind: TypeErrorKind::NotBoolean(other),
            }),
        }
    }

    fn numeric(&self, expression: &Expression, path: &str) -> Result<Type, TypeError> {
        let found = self.check(expression, path)?;
        if found.is_numeric() {
            Ok(found)
        } else {
            Err(TypeError {
                path: path.to_owned(),
                kind: TypeErrorKind::NotNumeric(found),
            })
        }
    }

    fn text(&self, expression: &Expression, path: &str) -> Result<(), TypeError> {
        let found = self.check(expression, path)?;
        if found.is_textual() {
            Ok(())
        } else {
            Err(TypeError {
                path: path.to_owned(),
                kind: TypeErrorKind::NotText(found),
            })
        }
    }

    fn angle(&self, expression: &Expression, path: &str) -> Result<(), TypeError> {
        match self.check(expression, path)? {
            Type::Null | Type::Any => Ok(()),
            Type::Number(unit) if unit == Unit::RADIAN => Ok(()),
            other => Err(TypeError {
                path: path.to_owned(),
                kind: TypeErrorKind::NotAngle(other),
            }),
        }
    }
}

fn joined(left: &Type, right: Type, path: &str) -> Result<Type, TypeError> {
    join(left, &right).ok_or_else(|| TypeError {
        path: path.to_owned(),
        kind: match (left.unit(), right.unit()) {
            (Some(left), Some(right)) => TypeErrorKind::UnitMismatch {
                operation: "combines",
                left,
                right,
            },
            _ => TypeErrorKind::Mismatch {
                left: left.clone(),
                right,
            },
        },
    })
}

/// The type of `+`, `−`, `min`, `max` or `round` over two numeric types,
/// `operation` naming it in an error.
fn same_unit(operation: &'static str, left: &Type, right: &Type) -> Result<Type, TypeErrorKind> {
    match (left.unit(), right.unit()) {
        (Some(left_unit), Some(right_unit)) if left_unit != right_unit => {
            Err(TypeErrorKind::UnitMismatch {
                operation,
                left: left_unit,
                right: right_unit,
            })
        }
        _ => Ok(match (left, right) {
            // A leaf known only when read takes the other operand's unit.
            (Type::Any, other) | (other, Type::Any) if other.unit().is_some() => other.clone(),
            _ => join(left, right).unwrap_or(Type::Any),
        }),
    }
}

/// Whether `operator` may compare values of these types.
fn comparable(
    operator: ExpressionComparison,
    left: &Type,
    right: &Type,
) -> Result<(), TypeErrorKind> {
    use ExpressionComparison as C;
    use Type as T;
    if left.is_loose() || right.is_loose() {
        return Ok(());
    }
    let mismatch = || TypeErrorKind::Mismatch {
        left: left.clone(),
        right: right.clone(),
    };
    match operator {
        C::Like | C::Matches | C::Contains => {
            if !left.is_textual() {
                return Err(TypeErrorKind::NotText(left.clone()));
            }
            if !right.is_textual() {
                return Err(TypeErrorKind::NotText(right.clone()));
            }
            Ok(())
        }
        ordered => match (left.unit(), right.unit()) {
            (Some(left), Some(right)) if left != right => Err(TypeErrorKind::UnitMismatch {
                operation: "compares",
                left,
                right,
            }),
            (Some(_), Some(_)) => Ok(()),
            _ => match (left, right) {
                (T::Boolean, T::Boolean) if matches!(ordered, C::Equals | C::NotEquals) => Ok(()),
                (T::Date, T::Date)
                | (T::DateTime, T::DateTime)
                | (T::Text | T::Enum(_), T::Text | T::Enum(_)) => Ok(()),
                _ => Err(mismatch()),
            },
        },
    }
}

fn pattern(operator: ExpressionComparison, text: &str, case_sensitive: bool) -> Result<(), String> {
    let source = if operator == ExpressionComparison::Like {
        crate::wildcard_regex(text)?
    } else {
        format!("^(?:{text})$")
    };
    regex::RegexBuilder::new(&source)
        .case_insensitive(!case_sensitive)
        .build()
        .map(drop)
        .map_err(|error| error.to_string())
}
