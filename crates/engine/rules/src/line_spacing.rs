//! Repeated members along a straight line as a measured member list
//! (`line_spacing`): the members an object reaches, each placed along a
//! direction through the object by its body's directional extent, ordered
//! by station, with the gap from the member before it and, on the first
//! and the last, the gap from the object's ends.
//!
//! A station is measured from where the object begins along the direction
//! (the lower end of its own extent), to a member's centre (the middle of
//! its extent); `from` and `to` are where the member begins and ends. A
//! gap is between centres, or with `gap=clear` between the faces the
//! extents bound. Every value is an interval holding the exact one, rounded
//! outward, and exact only as every extent it comes from is.
//!
//! The order is never guessed: two neighbours in the listing by station
//! are consecutive only where every order the station intervals allow
//! keeps them so (as `pairs` decides it), otherwise the later's `gap` is
//! undecided, and so are `first`, `last` and the end gaps where the end
//! member is not surely the end one.

use std::collections::BTreeMap;

use axioval_engine::{
    DirectionalExtent, MeasuredMember, MeasuredProvider, Measurement, MemberValue, MetricDirection,
    NotEvaluatedReason, PropertyResolutionError, RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use crate::body_extent::extent_error;
use crate::measured_kinds::{interval, objects_of_kinds, refused};
use crate::support::{Traversal, Unavailable, invalid};

/// Measures `line_spacing`.
pub(crate) struct LineSpacingMeasures;

const LIST: &str = "line_spacing";

/// An interval `[lower, upper]` of metres along the direction.
type Span = (f64, f64);

/// `left - right` for intervals, rounded outward.
fn minus(left: Span, right: Span) -> Span {
    ((left.0 - right.1).next_down(), (left.1 - right.0).next_up())
}

/// The middle of the interval between `low` and `high`, rounded outward.
fn middle(low: Span, high: Span) -> Span {
    (
        f64::midpoint(low.0, high.0).next_down(),
        f64::midpoint(low.1, high.1).next_up(),
    )
}

/// One body along the direction: where it begins, ends and its centre.
struct Placed {
    object: ObjectId,
    low: Span,
    high: Span,
    centre: Span,
    evidence: Evidence,
}

impl Placed {
    fn of(object: ObjectId, extent: &DirectionalExtent) -> Self {
        let end = |end: axioval_engine::ElevationInterval| (end.lower_metres(), end.upper_metres());
        let (low, high) = (end(extent.lower()), end(extent.upper()));
        Self {
            object,
            low,
            high,
            centre: middle(low, high),
            evidence: extent.evidence().clone(),
        }
    }
}

fn direction(call: &MeasuredCall) -> Result<MetricDirection, Unavailable> {
    match call.argument("direction") {
        Some(MeasuredArgument::Vector(vector)) => {
            MetricDirection::try_new(*vector).map_err(|_| invalid("`direction` is no direction"))
        }
        _ => Err(invalid("`direction` is required")),
    }
}

/// The members `object` reaches, placed along the direction, and the
/// object itself.
fn placed(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Placed, Vec<Placed>), Unavailable> {
    let along = direction(call)?;
    let extents = context
        .services
        .get::<VerticalExtentServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "the vertical-extent service is not registered".to_owned(),
            )
        })?;
    let Some(MeasuredArgument::Path(steps)) = call.argument("member_path") else {
        return Err(invalid("`member_path` is required"));
    };
    let kinds = objects_of_kinds(context, call, "members", object)
        .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
    let universe: Vec<_> = context
        .project
        .objects()
        .filter(|candidate| kinds.contains(&candidate.id))
        .collect();
    let (reached, _) = Traversal::path(steps)?.related(context, object, &universe)?;
    let measure = |id: &ObjectId| {
        extents
            .measure_directional_extent(id, along)
            .map(|extent| Placed::of(id.clone(), &extent))
            .map_err(|error| {
                let (reason, why) = extent_error(&error);
                (reason, format!("{id}: {why}"))
            })
    };
    let line = measure(object)?;
    let mut members = reached
        .iter()
        .map(&measure)
        .collect::<Result<Vec<_>, _>>()?;
    members.sort_by(|left, right| {
        left.centre
            .0
            .total_cmp(&right.centre.0)
            .then(left.centre.1.total_cmp(&right.centre.1))
            .then(left.object.cmp(&right.object))
    });
    Ok((line, members))
}

/// Whether a member is the first (or the last): surely, surely not, or
/// undecided (`None`).
type End = Option<bool>;

/// Whether each member surely follows the one before it, and whether each
/// surely is or is not the first and the last: a neighbour is decided only
/// where every order the centres allow keeps the two neighbours.
fn decided(members: &[Placed]) -> (Vec<bool>, Vec<End>, Vec<End>) {
    let count = members.len();
    let mut before = vec![f64::NEG_INFINITY; count + 1];
    for (place, member) in members.iter().enumerate() {
        before[place + 1] = before[place].max(member.centre.1);
    }
    let mut after = vec![f64::INFINITY; count + 1];
    for (place, member) in members.iter().enumerate().rev() {
        after[place] = after[place + 1].min(member.centre.0);
    }
    let follows = (0..count)
        .map(|place| {
            place > 0 && {
                let (earlier, later) = (members[place - 1].centre, members[place].centre);
                earlier.1 < later.0 && before[place - 1] < earlier.0 && after[place + 1] > later.1
            }
        })
        .collect();
    // Surely first where it surely lies before every other, surely not
    // where another surely lies before it; the mirror for the last.
    let end = |place: usize, first: bool| -> End {
        let centre = members[place].centre;
        let others = members
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != place)
            .map(|(_, member)| member.centre);
        if first {
            let (beyond, before) = others
                .fold((f64::INFINITY, f64::INFINITY), |(low, up), other| {
                    (low.min(other.0), up.min(other.1))
                });
            if centre.1 < beyond {
                Some(true)
            } else if before < centre.0 {
                Some(false)
            } else {
                None
            }
        } else {
            let (beyond, after) = others.fold(
                (f64::NEG_INFINITY, f64::NEG_INFINITY),
                |(up, low), other| (up.max(other.1), low.max(other.0)),
            );
            if centre.0 > beyond {
                Some(true)
            } else if after > centre.1 {
                Some(false)
            } else {
                None
            }
        }
    };
    let first = (0..count).map(|place| end(place, true)).collect();
    let last = (0..count).map(|place| end(place, false)).collect();
    (follows, first, last)
}

/// The items of one object: its members in order along the line.
#[allow(clippy::too_many_lines)] // One field after another, each worded.
fn items(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, Unavailable> {
    let clear = call.choice("gap") == Some("clear");
    let (line, members) = placed(call, object, context)?;
    let (follows, first, last) = decided(&members);
    let count = members.len();
    let length = Some(QuantityDimension::Length);
    let mut items = Vec::with_capacity(count);
    for (place, member) in members.iter().enumerate() {
        let exact = line.evidence.exact
            && member.evidence.exact
            && (place == 0 || members[place - 1].evidence.exact);
        let locator = |field: &str| format!("{LIST}:{object}:{}:{field}", member.object);
        let along = |span: Span, field: &str| {
            MemberValue::Measured(interval(
                minus(span, line.low),
                length,
                line.evidence.exact && member.evidence.exact,
                locator(field),
            ))
        };
        let undecided = |what: &str| MemberValue::Undecided {
            why: format!(
                "whether {} comes {what} along the line is undecided by the stations measured",
                member.object
            ),
        };
        let absent = |field: &str| {
            MemberValue::Measured(Measurement::Absent {
                locator: locator(field),
            })
        };
        let gap = if first[place] == Some(true) {
            absent("gap")
        } else if place > 0 && follows[place] {
            let earlier = &members[place - 1];
            let span = if clear {
                minus(member.low, earlier.high)
            } else {
                minus(member.centre, earlier.centre)
            };
            MemberValue::Measured(interval(span, length, exact, locator("gap")))
        } else {
            MemberValue::Undecided {
                why: format!(
                    "which member {} follows along the line is undecided by the stations \
                     measured",
                    member.object
                ),
            }
        };
        let end_exact = line.evidence.exact && member.evidence.exact;
        let start_gap = match first[place] {
            Some(true) => MemberValue::Measured(interval(
                minus(if clear { member.low } else { member.centre }, line.low),
                length,
                end_exact,
                locator("start_gap"),
            )),
            Some(false) => absent("start_gap"),
            None => undecided("first"),
        };
        let end_gap = match last[place] {
            Some(true) => MemberValue::Measured(interval(
                minus(line.high, if clear { member.high } else { member.centre }),
                length,
                end_exact,
                locator("end_gap"),
            )),
            Some(false) => absent("end_gap"),
            None => undecided("last"),
        };
        let truth = |end: End, what: &str| match end {
            Some(value) => MemberValue::Truth {
                value,
                locator: locator(what),
            },
            None => undecided(what),
        };
        items.push(MeasuredMember {
            certain: true,
            exact,
            evidence: std::iter::once(member.evidence.clone())
                .chain(
                    place
                        .checked_sub(1)
                        .map(|before| members[before].evidence.clone()),
                )
                .chain(std::iter::once(line.evidence.clone()))
                .collect(),
            fields: BTreeMap::from([
                (
                    "member",
                    MemberValue::Objects {
                        objects: vec![member.object.clone()],
                    },
                ),
                (
                    "what",
                    MemberValue::Text {
                        text: member.object.to_string(),
                    },
                ),
                ("station", along(member.centre, "station")),
                ("from", along(member.low, "from")),
                ("to", along(member.high, "to")),
                ("gap", gap),
                ("start_gap", start_gap),
                ("end_gap", end_gap),
                ("first", truth(first[place], "first")),
                ("last", truth(last[place], "last")),
            ]),
        });
    }
    Ok(items)
}

impl MeasuredProvider for LineSpacingMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[LIST]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        items(call, object, context).map_err(refused(call.name(), object))
    }
}
