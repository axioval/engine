//! The value of an aggregate over its members.
//!
//! A member whose membership cannot be decided is never dropped: `count`,
//! `sum`, `min`, `max` and `distinctCount` widen to every result it allows,
//! `any`, `all` and `none` follow Kleene logic over membership and truth,
//! and `average` is not evaluated.
//!
//! A member value that is `null` is never skipped and never false: it
//! makes `sum`, `min`, `max`, `average` and `distinctCount` `null`, and it
//! is an unknown truth for `any`, `all` and `none`, as `or` and `and` read
//! `null`. Members that state nothing are left out by the author, with a
//! `where` or a `coalesce` in the `value`.

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

/// The truths a member's value may be: whether it can be true, false and
/// `null` (unknown). A value not evaluated may be any.
#[derive(Clone, Copy)]
struct Can {
    true_: bool,
    false_: bool,
    null: bool,
}

/// A member's possible truths.
fn truth(member: &Member) -> Result<Can, Failure> {
    let can = |true_, false_, null| Can {
        true_,
        false_,
        null,
    };
    match &member.value {
        Ok(Value::Boolean(value)) => Ok(can(*value, !*value, false)),
        Ok(Value::Null) => Ok(can(false, false, true)),
        Ok(other) => Err(Failure::Here(Reason::Mismatch(format!(
            "a member's value is {}, not a truth",
            other.kind()
        )))),
        Err(_) => Ok(can(true, true, true)),
    }
}

/// `any` (or `all`) over every way the undecided members may belong and
/// every truth a member not evaluated may have: true, false or `null` when
/// every way agrees, otherwise not evaluated, naming a member's own reason
/// where there is one.
///
/// `all` holds for a set of at least one member, each true, and fails for
/// an empty set or one with a false member; `any` holds for a set with a
/// true member and fails for one whose members are all false. Otherwise a
/// `null` member leaves either `null`, as Kleene's `and` and `or`.
fn quantified(all: bool, members: &[Member]) -> Result<Value, Failure> {
    let mut certain = Vec::new();
    let mut possible = Vec::new();
    let mut cause = None;
    for member in members {
        let can = truth(member)?;
        if let Err(why) = &member.value {
            cause.get_or_insert_with(|| why.clone());
        }
        if member.certain {
            certain.push(can);
        } else {
            possible.push(can);
        }
    }
    let every = |test: fn(&Can) -> bool| certain.iter().all(test);
    let some = |test: fn(&Can) -> bool| certain.iter().chain(&possible).any(test);
    let (may_hold, may_fail, may_be_null) = if all {
        (
            // Every included member true, and at least one included.
            every(|can| can.true_) && (!certain.is_empty() || some(|can| can.true_)),
            // An included member false, or none included.
            certain.is_empty() || some(|can| can.false_),
            // No included member false, and one unknown.
            every(|can| can.true_ || can.null) && some(|can| can.null),
        )
    } else {
        (
            some(|can| can.true_),
            every(|can| can.false_),
            every(|can| can.false_ || can.null) && some(|can| can.null),
        )
    };
    match (may_hold, may_fail, may_be_null) {
        (true, false, false) => Ok(Value::Boolean(true)),
        (false, true, false) => Ok(Value::Boolean(false)),
        (false, false, true) => Ok(Value::Null),
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

/// The members' numeric values: certain ones, then those of undecided
/// members, and their one unit.
type Numbers = (Vec<Interval>, Vec<Interval>, Option<Unit>);

/// Whether a member's `null` value makes the aggregate `null`: `Some(true)`
/// when a certain member states none, `Some(false)` when only undecided
/// ones do (the result depends on whether they belong), `None` when every
/// member states a value.
fn stated_absent(members: &[Member]) -> Option<bool> {
    let mut null = members
        .iter()
        .filter(|member| member.value == Ok(Value::Null));
    let first = null.next()?;
    Some(first.certain || null.any(|member| member.certain))
}

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
    // A certain member stating no value decides, whatever the others are.
    if stated_absent(members) == Some(true) {
        return Ok(Value::Null);
    }
    let (certain, possible, unit) = numbers(members)?;
    if stated_absent(members).is_some() {
        return Err(undecided(members));
    }
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
    if stated_absent(members) == Some(true) {
        return Ok(Value::Null);
    }
    if let Some(why) = members
        .iter()
        .find_map(|member| member.value.as_ref().err())
    {
        return Err(Failure::Member(why.clone()));
    }
    if stated_absent(members).is_some() {
        return Err(undecided(members));
    }
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
