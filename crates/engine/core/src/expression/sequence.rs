//! The value of a `pairs`: its members ordered by their `by`, the
//! consecutive pairs of that order, and the function over a value of each.
//!
//! The members are listed by the lower bound of their `by`, then its upper
//! bound, then as listed. A pair of neighbours in that listing is
//! *decided* when every order the intervals allow keeps the two
//! neighbours: the earlier surely comes before the later (its upper bound
//! below the later's lower bound), every member listed before the earlier
//! surely comes before it, every member listed after the later surely
//! comes after it, and every member listed between them surely lies
//! between. Equal points are no order. A pair that is not decided is not
//! evaluated (`Reason::UndecidedOrder`), never ordered by a guess: an
//! `all` it cannot decide leaves the result open unless a decided pair
//! fails, and a numeric function over it is not evaluated.
//!
//! - A member whose membership is undecided may be absent, and the pair
//!   bridging it is then consecutive: every pair a choice of present
//!   members makes consecutive is listed, uncertain unless both members
//!   surely belong and none lies between them, and the function widens or
//!   leaves the result open as an aggregate's does.
//! - A member whose `by` is not evaluated may lie anywhere, so no pair is
//!   decided: the result is not evaluated, for its reason.
//! - A member surely belonging whose `by` is `null` (the source states no
//!   position) makes the result `null`, as a `null` member makes a `max`
//!   `null`; an undecided member's `null` leaves it not evaluated. `null`
//!   and not evaluated never mix.

use axioval_ir::Evidence;
use axioval_ir::contract::AggregateFunction;

use super::aggregate::{Failure, aggregate};
use super::evaluate::{Member, NotEvaluated, PairRead, Reason, Sequenced, Value};
use super::interval::Interval;
use super::unit::Unit;

/// The most pairs one `pairs` lists: members of undecided membership
/// multiply the pairs bridging them.
const MAX_PAIRS: usize = 100_000;

/// Two members that may follow each other, by their index in the listing.
pub(super) struct Pair {
    earlier: usize,
    later: usize,
    /// Whether both surely belong and none lies between them.
    certain: bool,
    /// Whether every order the members' `by` allow keeps them neighbours.
    decided: bool,
}

fn undecided(members: &[Sequenced]) -> Failure {
    Failure::Here(Reason::UndecidedMembers(
        members.iter().filter(|member| !member.certain).count(),
    ))
}

/// The consecutive pairs of `members`; `None` where the result is `null`.
pub(super) fn pairs(members: &[Sequenced]) -> Result<Option<Vec<Pair>>, Failure> {
    // A member surely belonging that states no position decides.
    if members
        .iter()
        .any(|member| member.certain && member.by == Ok(Value::Null))
    {
        return Ok(None);
    }
    let mut unit: Option<&Unit> = None;
    let mut keys: Vec<Interval> = Vec::with_capacity(members.len());
    for member in members {
        match &member.by {
            Ok(Value::Number { value, unit: own }) => {
                if let Some(unit) = unit
                    && unit != own
                {
                    return Err(Failure::Here(Reason::Mismatch(format!(
                        "the members' `by` are in {unit} and {own}, which differ"
                    ))));
                }
                unit = Some(own);
                keys.push(*value);
            }
            // An undecided member stating none: there may be no order.
            Ok(Value::Null) => return Err(undecided(members)),
            Ok(other) => {
                return Err(Failure::Here(Reason::Mismatch(format!(
                    "a member's `by` is {}, not a number",
                    other.kind()
                ))));
            }
            // It may lie anywhere: no pair is decided.
            Err(why) => return Err(Failure::Member(why.clone())),
        }
    }
    let count = members.len();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|left, right| {
        keys[*left]
            .lower
            .total_cmp(&keys[*right].lower)
            .then(keys[*left].upper.total_cmp(&keys[*right].upper))
            .then(left.cmp(right))
    });
    // The greatest upper bound listed before each place, and the least
    // lower bound listed from it on.
    let mut before = vec![f64::NEG_INFINITY; count + 1];
    for (place, index) in order.iter().enumerate() {
        before[place + 1] = before[place].max(keys[*index].upper);
    }
    let mut after = vec![f64::INFINITY; count + 1];
    for (place, index) in order.iter().enumerate().rev() {
        after[place] = after[place + 1].min(keys[*index].lower);
    }
    let mut pairs = Vec::new();
    for first in 0..count {
        let earlier = order[first];
        for second in first + 1..count {
            let later = order[second];
            let (low, high) = (keys[earlier], keys[later]);
            let between = order[first + 1..second]
                .iter()
                .all(|index| low.upper < keys[*index].lower && keys[*index].upper < high.lower);
            pairs.push(Pair {
                earlier,
                later,
                certain: members[earlier].certain && members[later].certain && second == first + 1,
                decided: low.upper < high.lower
                    && before[first] < low.lower
                    && after[second + 1] > high.upper
                    && between,
            });
            if pairs.len() > MAX_PAIRS {
                return Err(undecided(members));
            }
            // No pair reaches past a member surely belonging.
            if members[later].certain {
                break;
            }
        }
    }
    Ok(Some(pairs))
}

/// `function` over the value of each pair of `pairs` (`None`: `null`),
/// read by `value` for decided pairs, and the pairs that decide it, by
/// their members' indices: those failing an `all`, holding an `any`,
/// holding a `none` that fails, or that may be the least or greatest.
/// Every value read cites its evidence in `evidence`; `typed` is the unit
/// a sum over no pair takes, `here` words a pair whose order is undecided.
pub(super) fn over_pairs(
    function: AggregateFunction,
    (members, pairs): (&[Sequenced], Option<Vec<Pair>>),
    value: &mut dyn FnMut(usize, usize) -> PairRead,
    (typed, here): (&dyn Fn() -> Option<Unit>, &dyn Fn(Reason) -> NotEvaluated),
    evidence: &mut Vec<Evidence>,
) -> Result<(Value, Vec<(usize, usize)>), Failure> {
    let Some(pairs) = pairs else {
        return Ok((Value::Null, Vec::new()));
    };
    let counting = matches!(function, AggregateFunction::Count);
    if counting {
        // One pair fewer than the members present, whatever their order.
        let certain = members.iter().filter(|member| member.certain).count();
        #[allow(clippy::cast_precision_loss)]
        let value = Interval {
            lower: certain.saturating_sub(1) as f64,
            upper: members.len().saturating_sub(1) as f64,
        };
        return Ok((
            Value::Number {
                value,
                unit: Unit::NONE,
            },
            Vec::new(),
        ));
    }
    let mut listed = Vec::with_capacity(pairs.len());
    for pair in &pairs {
        let read = if pair.decided {
            value(pair.earlier, pair.later)
        } else {
            (
                Err(here(Reason::UndecidedOrder(
                    members[pair.earlier].name.clone(),
                    members[pair.later].name.clone(),
                ))),
                Vec::new(),
            )
        };
        evidence.extend(read.1.iter().cloned());
        listed.push(Member {
            certain: pair.certain,
            value: read.0,
            evidence: read.1,
        });
    }
    // Two members surely belonging make a pair whichever others do, which
    // an aggregate over possible pairs cannot know: every possible pair
    // holding, the `all` holds.
    if matches!(function, AggregateFunction::All)
        && members.iter().filter(|member| member.certain).count() >= 2
        && listed
            .iter()
            .all(|pair| pair.value == Ok(Value::Boolean(true)))
    {
        return Ok((Value::Boolean(true), Vec::new()));
    }
    // Undecided pairs stand for the members whose membership is undecided.
    let result = aggregate(function, &listed, typed).map_err(|failure| match failure {
        Failure::Here(Reason::UndecidedMembers(_)) => undecided(members),
        other => other,
    })?;
    let deciding = |pair: &Member| -> bool {
        use AggregateFunction as F;
        match (function, &result, &pair.value) {
            (F::All, Value::Boolean(false), Ok(Value::Boolean(value))) => pair.certain && !value,
            (F::Any, Value::Boolean(true), Ok(Value::Boolean(value)))
            | (F::None, Value::Boolean(false), Ok(Value::Boolean(value))) => pair.certain && *value,
            (F::Max, Value::Number { value: extreme, .. }, Ok(Value::Number { value, .. })) => {
                value.upper >= extreme.lower
            }
            (F::Min, Value::Number { value: extreme, .. }, Ok(Value::Number { value, .. })) => {
                value.lower <= extreme.upper
            }
            _ => false,
        }
    };
    let chosen = pairs
        .iter()
        .zip(&listed)
        .filter(|(_, member)| deciding(member))
        .map(|(pair, _)| (pair.earlier, pair.later))
        .collect();
    Ok((result, chosen))
}
