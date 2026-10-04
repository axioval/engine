//! The value of an aggregate over its members.
//!
//! A member whose membership cannot be decided is never dropped: `count`,
//! `sum`, `min`, `max` and `distinctCount` widen to every result it allows,
//! `any`, `all` and `none` follow Kleene logic over membership and truth,
//! and `average` is not evaluated. `null` member values are skipped by
//! numeric aggregates and are not true for truth aggregates.

use std::collections::BTreeSet;

use axioval_ir::contract::AggregateFunction;

use super::evaluate::{Member, NotEvaluated, Reason, Value};
use super::interval::Interval;
use super::unit::Unit;

/// Why an aggregate has no value: a member's own value is not evaluated,
/// or the aggregate itself cannot be decided.
pub(super) enum Failure {
    Member(NotEvaluated),
    Here(Reason),
}

fn undecided(members: &[Member]) -> Failure {
    Failure::Here(Reason::UndecidedMembers(
        members.iter().filter(|member| !member.certain).count(),
    ))
}

/// A member's truth: decided, or not evaluated and why.
fn truth(member: &Member) -> Result<Option<bool>, Failure> {
    match &member.value {
        Ok(Value::Boolean(value)) => Ok(Some(*value)),
        Ok(Value::Null) => Ok(Some(false)),
        Ok(other) => Err(Failure::Here(Reason::Mismatch(format!(
            "a member's value is {}, not a truth",
            other.kind()
        )))),
        Err(_) => Ok(None),
    }
}

/// `any` (or `all`) over every way the undecided members may belong: true
/// or false when every way agrees, otherwise not evaluated, naming a
/// member's own reason where there is one.
///
/// `all` holds for a set of at least one member, each true; `any` for a
/// set with a true member.
fn quantified(all: bool, members: &[Member]) -> Result<Value, Failure> {
    let mut certain = Vec::new();
    let mut possible = Vec::new();
    let mut cause = None;
    for member in members {
        let value = truth(member)?;
        if let (None, Err(why)) = (value, &member.value) {
            cause.get_or_insert_with(|| why.clone());
        }
        // Whether it can be true, and whether it can be false.
        let can = (value != Some(false), value != Some(true));
        if member.certain {
            certain.push(can);
        } else {
            possible.push(can);
        }
    }
    let (may_hold, may_fail) = if all {
        (
            certain.iter().all(|(true_, _)| *true_)
                && (!certain.is_empty() || possible.iter().any(|(true_, _)| *true_)),
            certain.is_empty()
                || certain.iter().any(|(_, false_)| *false_)
                || possible.iter().any(|(_, false_)| *false_),
        )
    } else {
        (
            certain.iter().chain(&possible).any(|(true_, _)| *true_),
            certain.iter().all(|(_, false_)| *false_),
        )
    };
    match (may_hold, may_fail) {
        (true, false) => Ok(Value::Boolean(true)),
        (false, true) => Ok(Value::Boolean(false)),
        _ => Err(cause.map_or_else(|| undecided(members), Failure::Member)),
    }
}

/// The aggregate `function` over `members`.
pub(super) fn aggregate(function: AggregateFunction, members: &[Member]) -> Result<Value, Failure> {
    use AggregateFunction as F;
    match function {
        F::Count => {
            let certain = members.iter().filter(|member| member.certain).count();
            Ok(count(certain, members.len()))
        }
        F::Any => quantified(false, members),
        F::None => match quantified(false, members)? {
            Value::Boolean(any) => Ok(Value::Boolean(!any)),
            other => Ok(other),
        },
        F::All => quantified(true, members),
        F::Sum | F::Min | F::Max | F::Average => numeric(function, members),
        F::DistinctCount => distinct(members),
    }
}

/// A count between `lower` and `upper`, exact when they agree.
#[allow(clippy::cast_precision_loss)]
fn count(lower: usize, upper: usize) -> Value {
    Value::Number {
        value: Interval {
            lower: lower as f64,
            upper: upper as f64,
        },
        unit: Unit::NONE,
    }
}

/// The members' numeric values, `null` ones skipped: certain ones, then
/// those of undecided members, and their one unit.
type Numbers = (Vec<Interval>, Vec<Interval>, Option<Unit>);

fn numbers(members: &[Member]) -> Result<Numbers, Failure> {
    let (mut certain, mut possible, mut unit): Numbers = (Vec::new(), Vec::new(), None);
    for member in members {
        let value = match &member.value {
            Err(why) => return Err(Failure::Member(why.clone())),
            Ok(Value::Null) => continue,
            Ok(Value::Number { value, unit: own }) => {
                match &unit {
                    Some(unit) if unit != own => {
                        return Err(Failure::Here(Reason::Mismatch(format!(
                            "the members' values are in {unit} and {own}, which differ"
                        ))));
                    }
                    _ => unit = Some(own.clone()),
                }
                *value
            }
            Ok(other) => {
                return Err(Failure::Here(Reason::Mismatch(format!(
                    "a member's value is {}, not a number",
                    other.kind()
                ))));
            }
        };
        if member.certain {
            certain.push(value);
        } else {
            possible.push(value);
        }
    }
    Ok((certain, possible, unit))
}

fn numeric(function: AggregateFunction, members: &[Member]) -> Result<Value, Failure> {
    use AggregateFunction as F;
    let (certain, possible, unit) = numbers(members)?;
    let Some(unit) = unit else {
        // No member has a value, or none is surely there.
        return if members.iter().any(|member| !member.certain) && function != F::Sum {
            Err(undecided(members))
        } else if function == F::Sum {
            Ok(Value::Number {
                value: Interval::point(0.0),
                unit: Unit::NONE,
            })
        } else {
            Ok(Value::Null)
        };
    };
    let overflow = |_| Failure::Here(Reason::Overflow);
    let value = match function {
        F::Sum => {
            let mut total = Interval::point(0.0);
            for value in &certain {
                total = total.plus(*value).map_err(overflow)?;
            }
            // An undecided member adds its value or nothing.
            for value in &possible {
                total = total
                    .plus(value.hull(Interval::point(0.0)))
                    .map_err(overflow)?;
            }
            total
        }
        F::Min | F::Max => {
            let least = function == F::Min;
            let pick = |left: Interval, right: Interval| {
                if least {
                    left.min(right)
                } else {
                    left.max(right)
                }
            };
            let Some(sure) = certain.iter().copied().reduce(pick) else {
                // Every value belongs to an undecided member: there may be none.
                return Err(undecided(members));
            };
            // An undecided member may lower a minimum (raise a maximum), and
            // otherwise leaves the certain ones' value.
            possible
                .iter()
                .fold(sure, |result, value| result.hull(pick(result, *value)))
        }
        _ => {
            if !possible.is_empty() {
                return Err(undecided(members));
            }
            let mut total = Interval::point(0.0);
            for value in &certain {
                total = total.plus(*value).map_err(overflow)?;
            }
            #[allow(clippy::cast_precision_loss)]
            let count = Interval::point(certain.len() as f64);
            total
                .divided_by(count)
                .map_err(|_| Failure::Here(Reason::Overflow))?
        }
    };
    Ok(Value::Number { value, unit })
}

/// A value's identity for counting distinct values: its text, or its
/// exact number with unit.
fn key(value: &Value) -> Result<Option<String>, Failure> {
    Ok(Some(match value {
        Value::Null => return Ok(None),
        Value::Number { value, unit } if value.is_point() => format!("{} {unit}", value.lower),
        Value::Number { .. } => {
            return Err(Failure::Here(Reason::Mismatch(
                "a member's value is an interval, which no distinct count can tell apart".into(),
            )));
        }
        other => other.to_string(),
    }))
}

fn distinct(members: &[Member]) -> Result<Value, Failure> {
    let mut certain = BTreeSet::new();
    let mut possible = BTreeSet::new();
    for member in members {
        let value = member
            .value
            .as_ref()
            .map_err(|why| Failure::Member(why.clone()))?;
        if let Some(key) = key(value)? {
            if member.certain {
                certain.insert(key);
            } else {
                possible.insert(key);
            }
        }
    }
    let upper = certain.union(&possible).count();
    Ok(count(certain.len(), upper))
}
