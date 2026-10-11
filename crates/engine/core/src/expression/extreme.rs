//! The value of an `argmin` or `argmax`: the members that may be the
//! least (greatest) by their `by`, and the `value` they agree on.
//!
//! A member may be the greatest when its `by` may reach the greatest lower
//! bound of the members that surely belong (`upper >= max lower`), and the
//! least when it may reach their least upper bound. Every such member is
//! chosen, a tie or an overlap of intervals included, so a finding names
//! each member that may decide it.
//!
//! - A member whose membership is undecided may be chosen, but never
//!   bounds the others, since it may not belong. With no member sure to
//!   belong there may be none: not evaluated, unless there is no member at
//!   all, which is `null`.
//! - A member whose `by` is not evaluated may be the extreme whatever the
//!   others are: it is chosen and bounds no other. The value stands only
//!   where every chosen member's value agrees; otherwise the member's own
//!   reason leaves it not evaluated.
//! - A member surely belonging whose `by` is `null` (the source states
//!   none) makes the result `null`, as it makes a `max` aggregate `null`;
//!   an undecided member's `null` leaves it not evaluated. `null` and not
//!   evaluated never mix.
//! - The chosen members' values must agree: equal values, or numbers of
//!   one unit, whose interval then holds each of them. Values that differ,
//!   or `null` beside a value, leave the result not evaluated.

use super::aggregate::Failure;
use super::evaluate::{Ranked, Reason, Value};
use super::interval::Interval;
use super::unit::Unit;

fn undecided(members: &[Ranked]) -> Failure {
    Failure::Here(Reason::UndecidedMembers(
        members.iter().filter(|member| !member.certain).count(),
    ))
}

/// The `value` of the members that may be the greatest (`greatest`) or
/// least by their `by`, and their indices in `members`: none where the
/// result is `null`.
pub(super) fn extreme(greatest: bool, members: &[Ranked]) -> Result<(Value, Vec<usize>), Failure> {
    // A member surely belonging that states no `by` decides.
    if members
        .iter()
        .any(|member| member.certain && member.by == Ok(Value::Null))
    {
        return Ok((Value::Null, Vec::new()));
    }
    let mut unit: Option<&Unit> = None;
    let mut ranks: Vec<Option<Interval>> = Vec::with_capacity(members.len());
    for member in members {
        ranks.push(match &member.by {
            Ok(Value::Number { value, unit: own }) => {
                if let Some(unit) = unit
                    && unit != own
                {
                    return Err(Failure::Here(Reason::Mismatch(format!(
                        "the members' `by` are in {unit} and {own}, which differ"
                    ))));
                }
                unit = Some(own);
                Some(*value)
            }
            // An undecided member stating none: there may be no value.
            Ok(Value::Null) => return Err(undecided(members)),
            Ok(other) => {
                return Err(Failure::Here(Reason::Mismatch(format!(
                    "a member's `by` is {}, not a number",
                    other.kind()
                ))));
            }
            Err(_) => None,
        });
    }
    if members.is_empty() {
        return Ok((Value::Null, Vec::new()));
    }
    if !members.iter().any(|member| member.certain) {
        // There may be no member at all.
        return Err(undecided(members));
    }
    // The bound every member surely belonging reaches: the greatest lower
    // bound (least upper bound) among them.
    let bound = members
        .iter()
        .zip(&ranks)
        .filter(|(member, _)| member.certain)
        .filter_map(|(_, rank)| *rank)
        .map(|rank| if greatest { rank.lower } else { rank.upper })
        .reduce(if greatest { f64::max } else { f64::min });
    let chosen: Vec<usize> = ranks
        .iter()
        .enumerate()
        .filter(|(_, rank)| match (rank, bound) {
            (Some(rank), Some(bound)) => {
                if greatest {
                    rank.upper >= bound
                } else {
                    rank.lower <= bound
                }
            }
            // Unknown, or nothing known bounds it.
            _ => true,
        })
        .map(|(index, _)| index)
        .collect();
    let value = agreed(members, &chosen).map_err(|failure| {
        // A chosen member whose `by` is not evaluated is why they may differ.
        chosen
            .iter()
            .find_map(|index| members[*index].by.as_ref().err())
            .map_or(failure, |why| Failure::Member(why.clone()))
    })?;
    Ok((value, chosen))
}

/// The value the `chosen` members agree on.
fn agreed(members: &[Ranked], chosen: &[usize]) -> Result<Value, Failure> {
    let mut agreed: Option<Value> = None;
    for index in chosen {
        let value = members[*index]
            .value
            .as_ref()
            .map_err(|why| Failure::Member(why.clone()))?;
        agreed = Some(match (agreed, value) {
            (None, value) => value.clone(),
            (Some(Value::Number { value: held, unit }), Value::Number { value, unit: own }) => {
                if unit != *own {
                    return Err(Failure::Here(Reason::Mismatch(format!(
                        "the chosen members' values are in {unit} and {own}, which differ"
                    ))));
                }
                Value::Number {
                    value: held.hull(*value),
                    unit,
                }
            }
            (Some(held), value) if held == *value => held,
            (Some(_), _) => {
                return Err(Failure::Here(Reason::UndecidedExtreme(chosen.len())));
            }
        });
    }
    Ok(agreed.unwrap_or(Value::Null))
}
