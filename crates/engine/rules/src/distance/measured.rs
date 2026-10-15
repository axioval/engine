//! Distances as measured values: the nearest or farthest counterpart's
//! distance, and how many counterparts lie within a radius, measured
//! exactly as `distance` measures them for its verdicts.
//!
//! The counterparts are the objects of the source kinds `to` names. The
//! search reuses the broad phase within `within` (or the `radius`), so a
//! counterpart it does not propose is proven beyond it. An undecided
//! counterpart, one whose distance could not be read or whose interval
//! straddles, widens the value towards the bound it could move, as
//! `distance` counts it as unknown.
//!
//! A value over the counterparts within a walk (`measure=walking`) walks
//! to those the plan broad phase proposes within the radius, through the
//! metric-routing service, from the representative point `space-distance`
//! walks from up to each counterpart: to where the body stands next to
//! it, and the straight reach from there to its representative point. One over a shared derived group
//! (`group=`) reads the run's groups and measures nothing. One over a
//! context zone (`zone=`, also a reach of `count_within`) locates every
//! counterpart against the zone by its footprint or, lacking one of any
//! area, its placement point (`context_zone::locate`).

use std::collections::BTreeMap;

use axioval_engine::expression::{Interval, Value};
use axioval_engine::{
    Citation, CompiledRule, DerivedGroups, MeasuredProvider, Measurement, Membership,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingServiceHandle, MobilityProfile,
    NotEvaluatedReason, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity};
use axioval_ir::measured::{
    COUNT_WITHIN, DISTANCE, MAX_WITHIN, MEAN_WITHIN, MIN_WITHIN, MeasuredArgument, MeasuredCall,
    SUM_WITHIN,
};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, RuleId};

use super::{Bodies, Caches, Candidate, Scope, declaration};
use crate::pairs::{Voids, prepare};
use crate::support::{PropertyRef, Unavailable};

/// Measures `distance`, `count_within`, and `sum_within`, `min_within`,
/// `max_within` and `mean_within` of a value the counterparts state.
pub struct DistanceMeasures;

/// The distance declaration `call` stands for, around one subject.
fn rule(
    call: &MeasuredCall,
    object: &ObjectId,
    counterparts: std::collections::BTreeSet<ObjectId>,
    (reach, walking): (f64, bool),
) -> CompiledRule {
    let text = |value: &str| ParameterValue::String {
        value: value.to_owned(),
    };
    let mut parameters = BTreeMap::from([
        (
            "counterparts".to_owned(),
            ParameterValue::Selector {
                value: Box::new(Selector::Objects {
                    objects: counterparts,
                }),
            },
        ),
        (
            "maximum_metres".to_owned(),
            ParameterValue::Number { value: reach },
        ),
    ]);
    // A walk is never shorter than the plan distance between the bodies
    // its ends stand in: the plan broad phase proposes every counterpart
    // that may be walked to within the reach.
    if walking {
        parameters.insert("projection".to_owned(), text("horizontal"));
    }
    for (key, parameter) in [
        ("projection", "projection"),
        ("direction", "vertical_direction"),
        ("subject_surface", "subject_surface"),
        ("counterpart_surface", "counterpart_surface"),
    ] {
        if let Some(choice) = call.choice(key)
            && !walking
        {
            parameters.insert(parameter.to_owned(), text(choice));
        }
    }
    CompiledRule {
        id: RuleId::new("axioval-measured-distance").expect("a valid rule id"),
        capability: "axioval:capability.distance".into(),
        severity: Severity::Info,
        selector: Selector::Objects {
            objects: std::collections::BTreeSet::from([object.clone()]),
        },
        parameters,
    }
}

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

/// The interval a candidate's distance surely lies in: unknown when it
/// could not be measured. An infinite lower bound is no relation in the
/// projection at all.
fn interval(candidate: &Candidate) -> (f64, f64) {
    candidate.interval().unwrap_or((0.0, f64::INFINITY))
}

/// The nearest (or farthest) counterpart's distance from what is known of
/// every candidate.
fn extreme(all: &[&Candidate], farthest: bool) -> Result<Measurement, String> {
    let related: Vec<&&Candidate> = all
        .iter()
        .filter(|candidate| interval(candidate).0.is_finite())
        .collect();
    let possible: Vec<(f64, f64)> = related
        .iter()
        .filter(|candidate| candidate.possibly_in_scope())
        .map(|candidate| interval(candidate))
        .collect();
    let certain: Vec<(f64, f64)> = related
        .iter()
        .filter(|candidate| candidate.certainly_in_scope() && candidate.interval().is_some())
        .map(|candidate| interval(candidate))
        .collect();
    if possible.is_empty() {
        return Ok(Measurement::Absent {
            locator: "no counterpart within the search".into(),
        });
    }
    let (lower, upper) = if farthest {
        (
            certain
                .iter()
                .map(|(lower, _)| *lower)
                .fold(f64::NEG_INFINITY, f64::max),
            possible
                .iter()
                .map(|(_, upper)| *upper)
                .fold(f64::NEG_INFINITY, f64::max),
        )
    } else {
        (
            possible
                .iter()
                .map(|(lower, _)| *lower)
                .fold(f64::INFINITY, f64::min),
            certain
                .iter()
                .map(|(_, upper)| *upper)
                .fold(f64::INFINITY, f64::min),
        )
    };
    if !(lower.is_finite() && upper.is_finite()) {
        return Err(format!(
            "the {} counterpart is undecided: one may or may not count",
            if farthest { "farthest" } else { "nearest" }
        ));
    }
    Ok(Measurement::Cited {
        lower: lower.max(0.0),
        upper,
        dimension: Some(QuantityDimension::Length),
        locator: String::new(),
        // Exact as the counterparts that may be the nearest (or farthest)
        // were measured; one surely beyond it decides nothing.
        exact: measured_exactly(
            related
                .iter()
                .map(|candidate| **candidate)
                .filter(|candidate| {
                    let (low, high) = interval(candidate);
                    if farthest {
                        high >= lower
                    } else {
                        low <= upper
                    }
                }),
        ),
    })
}

/// Whether every possible counterpart measured was measured exactly, as
/// `distance` cites its verdicts: an unmeasurable counterpart widens the
/// value without evidence of its own.
fn measured_exactly<'a>(candidates: impl Iterator<Item = &'a Candidate>) -> bool {
    candidates
        .filter(|candidate| candidate.possibly_in_scope())
        .all(|candidate| {
            candidate.measured.as_ref().map_or(true, |measured| {
                measured.evidence.iter().all(|evidence| evidence.exact)
            })
        })
}

/// Every counterpart that may lie within `[from, radius]`, with whether it
/// surely does: in scope, measured, and its whole interval within.
fn within<'c>(all: &[&'c Candidate], from: f64, radius: f64) -> Vec<(&'c Candidate, bool)> {
    let mut reached = Vec::new();
    for candidate in all {
        let (lower, upper) = interval(candidate);
        if !lower.is_finite() || !candidate.possibly_in_scope() {
            continue;
        }
        if upper >= from && lower <= radius {
            let surely = candidate.certainly_in_scope()
                && candidate.interval().is_some()
                && from <= lower
                && upper <= radius;
            reached.push((*candidate, surely));
        }
    }
    reached
}

/// The counterparts that may count towards a `*_within`, each with whether
/// it surely does, and whether everything that placed them is exact.
struct Reached<'c> {
    counterparts: Vec<(&'c ObjectId, bool)>,
    exact: bool,
}

impl<'c> Reached<'c> {
    /// Those within `[from, radius]` of their measured distances.
    fn measured(all: &[&'c Candidate], from: f64, radius: f64) -> Self {
        let reached = within(all, from, radius);
        Self {
            exact: measured_exactly(reached.iter().map(|(candidate, _)| *candidate)),
            counterparts: reached
                .into_iter()
                .map(|(candidate, surely)| (&candidate.counterpart, surely))
                .collect(),
        }
    }

    /// Those within `[from, radius]` walking from the object's
    /// representative point up to each counterpart (#361): to where the
    /// body stands next to it, plus the straight reach from there to its
    /// representative point, which may lie inside the counterpart's own
    /// body (a wall-hung extinguisher), among the candidates the plan
    /// broad phase proposes within `radius`: no walk is shorter than the
    /// plan distance between the bodies its ends stand in, so one it does
    /// not propose is surely beyond.
    ///
    /// A walk the route service proves blocked is surely beyond; one it
    /// bounds counts as a measured distance does, surely where its whole
    /// interval lies within; one it refuses, or whose end has no
    /// representative point, may or may not count.
    fn walked(
        context: &RuleContext<'_>,
        call: &MeasuredCall,
        object: &ObjectId,
        (all, from, radius): (&[&'c Candidate], f64, f64),
    ) -> Result<Self, Unavailable> {
        let profile = profile(call)?;
        let routes = context
            .services
            .get::<MetricRoutingServiceHandle>()
            .ok_or_else(|| crate::space_distance::missing("metric-routing"))?;
        let (origin, cited) = crate::space_distance::representative_point(context, object)?;
        let mut reached = Self {
            counterparts: Vec::new(),
            exact: cited.iter().all(|evidence| evidence.exact),
        };
        for (candidate, _) in within(all, 0.0, radius) {
            let counterpart = &candidate.counterpart;
            let walked = crate::space_distance::representative_point(context, counterpart)
                .and_then(|(end, cited)| {
                    routes
                        .route(&MetricRouteRequest::new(origin.clone(), end, profile).approaching())
                        .map(|outcome| (outcome, cited))
                        .map_err(|error| {
                            crate::space_distance::incomplete(format!(
                                "walking from {object} to {counterpart}: {error}"
                            ))
                        })
                });
            match walked {
                // Proven unreachable: surely beyond any radius.
                Ok((MetricRouteOutcome::Blocked(_), _)) => {}
                Ok((MetricRouteOutcome::Reachable(route), cited)) => {
                    let length = route.shortest_distance();
                    let (lower, upper) = (length.lower_metres(), length.upper_metres());
                    if upper < from || lower > radius {
                        continue;
                    }
                    reached.exact &= route.evidence().exact && cited.iter().all(|e| e.exact);
                    let surely = candidate.certainly_in_scope() && from <= lower && upper <= radius;
                    reached.counterparts.push((counterpart, surely));
                }
                Err(_) => reached.counterparts.push((counterpart, false)),
            }
        }
        Ok(reached)
    }
}

impl<'c> Reached<'c> {
    /// Those sharing the object's group of `grouping`, or, where the
    /// object is a group of it, its members.
    ///
    /// A member of the group surely shares it. Where the group's members
    /// are not all known (a member's key is unreadable, or it may be one
    /// compartment with another), a counterpart of its source whose own
    /// membership is undecided, or which belongs to another group not
    /// known completely, may or may not share it. An object of no group
    /// shares none; one whose membership is undecided leaves the value
    /// not evaluated.
    fn grouped(
        context: &RuleContext<'_>,
        grouping: &str,
        object: &ObjectId,
        counterparts: &'c std::collections::BTreeSet<ObjectId>,
    ) -> Result<Self, Unavailable> {
        let groups = context
            .services
            .get::<std::sync::Arc<DerivedGroups>>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "no derived groups are available outside a run".to_owned(),
                )
            })?;
        let derived = groups.grouping(grouping).ok_or_else(|| {
            crate::support::invalid(format!("the run derives no grouping `{grouping}`"))
        })?;
        let exact = |cited: &[Evidence]| cited.iter().all(|evidence| evidence.exact);
        let mut reached = Self {
            counterparts: Vec::new(),
            exact: true,
        };
        let (id, group) = match derived.group(object) {
            Some(group) => (object, group),
            None => match derived.membership(object) {
                Some(Membership::Grouped(id, cited)) => {
                    reached.exact &= exact(cited);
                    let group = derived.group(id).ok_or_else(|| {
                        crate::space_distance::incomplete(format!("group {id} is not derived"))
                    })?;
                    (id, group)
                }
                Some(Membership::Undecided(reason, why)) => {
                    return Err((
                        reason.clone(),
                        format!(
                            "whether {object} belongs to a group of `{grouping}` is undecided: \
                             {why}"
                        ),
                    ));
                }
                Some(Membership::Ungrouped(_)) | None => return Ok(reached),
            },
        };
        for counterpart in counterparts {
            if group.members().binary_search(counterpart).is_ok() {
                if let Some(Membership::Grouped(_, cited)) = derived.membership(counterpart) {
                    reached.exact &= exact(cited);
                }
                reached.counterparts.push((counterpart, true));
                continue;
            }
            if group.undecided().is_none() || counterpart.source != id.source {
                continue;
            }
            let possible = match derived.membership(counterpart) {
                Some(Membership::Undecided(..)) => true,
                Some(Membership::Grouped(other, _)) => derived
                    .group(other)
                    .is_some_and(|other| other.undecided().is_some()),
                Some(Membership::Ungrouped(_)) | None => false,
            };
            if possible {
                reached.counterparts.push((counterpart, false));
            }
        }
        Ok(reached)
    }

    /// Those lying in the context zone `call` names (#360), each located
    /// as `context_zone::locate` locates it: one surely inside counts, one
    /// straddling the boundary, or which neither its footprint nor its
    /// placement locates, may or may not count, as a counterpart whose
    /// distance straddles the radius or cannot be read; why each such one
    /// cannot be located is noted.
    fn zoned(
        context: &RuleContext<'_>,
        call: &MeasuredCall,
        counterparts: &'c std::collections::BTreeSet<ObjectId>,
    ) -> Result<(Self, Vec<String>), Unavailable> {
        use crate::context_zone::{InZone, Location, locate};
        let mut reached = Self {
            counterparts: Vec::new(),
            exact: true,
        };
        let mut notes = Vec::new();
        for counterpart in counterparts {
            match locate(context, call, counterpart)? {
                Location::Located(InZone::Not, _) => {}
                Location::Located(within, exact) => {
                    reached.exact &= exact;
                    reached
                        .counterparts
                        .push((counterpart, within == InZone::Surely));
                }
                Location::Unlocated(why) => {
                    notes.push(format!("{counterpart} may lie in the zone: {why}"));
                    reached.counterparts.push((counterpart, false));
                }
            }
        }
        Ok((reached, notes))
    }

    /// How many surely and possibly count, as `count_within` cites them.
    fn count(&self) -> Measurement {
        let sure = self
            .counterparts
            .iter()
            .filter(|(_, surely)| *surely)
            .count();
        #[allow(clippy::cast_precision_loss)]
        Measurement::Cited {
            lower: sure as f64,
            upper: self.counterparts.len() as f64,
            dimension: None,
            locator: String::new(),
            exact: self.exact,
        }
    }
}

/// The walking body `measure=walking` states.
fn profile(call: &MeasuredCall) -> Result<MobilityProfile, Unavailable> {
    let needed = |key: &str| {
        length(call, key).ok_or_else(|| {
            crate::support::invalid(format!("`{}` walking needs `{key}`", call.name()))
        })
    };
    let slope = match call.argument("walking_slope") {
        None => 0.0,
        Some(MeasuredArgument::Number(slope)) => *slope,
        Some(_) => {
            return Err(crate::support::invalid(
                "`walking_slope` is not a plain number",
            ));
        }
    };
    MobilityProfile::try_new(
        needed("walking_radius")?,
        needed("walking_height")?,
        needed("walking_step")?,
        slope,
    )
    .map_err(|error| crate::support::invalid(error.to_string()))
}

/// How many counterparts surely and possibly lie within `[from, radius]`.
/// Cited as exact where every possible one measured was measured exactly,
/// as `distance` cites a shortfall: an unmeasurable counterpart widens the
/// count without evidence of its own.
fn count(all: &[&Candidate], from: f64, radius: f64) -> Measurement {
    let reached = within(all, from, radius);
    let sure = reached.iter().filter(|(_, surely)| *surely).count();
    #[allow(clippy::cast_precision_loss)]
    Measurement::Cited {
        lower: sure as f64,
        upper: reached.len() as f64,
        dimension: None,
        locator: String::new(),
        exact: measured_exactly(reached.iter().map(|(candidate, _)| *candidate)),
    }
}

/// What a counterpart within states of the value a `*_within` sums or
/// compares: a plain number, or nothing.
enum Stated {
    Number(Interval),
    Nothing,
}

/// The plain number `property` of `counterpart` states, with whether its
/// evidence is exact; `Nothing` where it states none (absent or `null`).
fn stated(
    context: &RuleContext<'_>,
    counterpart: &ObjectId,
    property: PropertyRef<'_>,
) -> Result<(Stated, bool), Unavailable> {
    let object = crate::selection::object_by_id(context, counterpart).ok_or_else(|| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("{counterpart} is not in the run"),
        )
    })?;
    let resolved = crate::support::resolve(context, object, property)
        .map_err(|(reason, why)| (reason, format!("{property} of {counterpart}: {why}")))?;
    let crate::support::Resolved::Present(found) = resolved else {
        return Ok((Stated::Nothing, true));
    };
    let exact = found
        .evidence
        .as_ref()
        .is_none_or(|evidence| evidence.exact);
    let invalid = |why: String| (NotEvaluatedReason::InvalidEvidence, why);
    match Value::from_property(&found.value) {
        Ok(Value::Null) => Ok((Stated::Nothing, exact)),
        Ok(Value::Number { value, unit }) if unit.is_plain() => Ok((Stated::Number(value), exact)),
        Ok(other) => Err(invalid(format!(
            "{property} of {counterpart} is {}, not a plain number",
            other.kind()
        ))),
        Err(why) => Err(invalid(format!("{property} of {counterpart}: {why}"))),
    }
}

/// `sum_within`, `min_within`, `max_within` or `mean_within` (`name`) of
/// `property` over the counterparts that may lie within `[from, radius]`.
///
/// A counterpart surely within contributes its value; one that only may be
/// (its distance straddles a bound, or its scope or extent is undecided)
/// contributes its value or nothing, so the sum widens to both and the
/// least or greatest moves only where it may; the mean is not evaluated
/// while one may or may not count. A counterpart surely within stating no
/// value makes the result `null`, one only possibly within leaves it not
/// evaluated, and no counterpart within is `null` (a sum of none is zero).
fn aggregate_within(
    name: &str,
    context: &RuleContext<'_>,
    reached: &Reached<'_>,
    property: PropertyRef<'_>,
) -> Result<Measurement, Unavailable> {
    let unreadable = |why: String| (NotEvaluatedReason::IncompleteEvidence, why);
    let read: Vec<_> = reached
        .counterparts
        .iter()
        .map(|(counterpart, surely)| {
            (
                *counterpart,
                *surely,
                stated(context, counterpart, property),
            )
        })
        .collect();
    // A counterpart surely within stating no value decides, whatever the
    // others are, as an aggregate's member does.
    if let Some((counterpart, ..)) = read
        .iter()
        .find(|(_, surely, value)| *surely && matches!(value, Ok((Stated::Nothing, _))))
    {
        return Ok(Measurement::Absent {
            locator: format!("{counterpart} states no {property}"),
        });
    }
    let mut sure = Vec::new();
    let mut possible = Vec::new();
    let mut exact = reached.exact;
    for (counterpart, surely, value) in read {
        let (value, value_exact) = value?;
        exact &= value_exact;
        match value {
            Stated::Number(value) if surely => sure.push(value),
            Stated::Number(value) => possible.push(value),
            Stated::Nothing => {
                return Err(unreadable(format!(
                    "{counterpart} may or may not be within and states no {property}"
                )));
            }
        }
    }
    Ok(match combine(name, &sure, &possible)? {
        Some(value) => Measurement::Cited {
            lower: value.lower,
            upper: value.upper,
            dimension: None,
            locator: String::new(),
            exact,
        },
        None => Measurement::Absent {
            locator: "no counterpart within".into(),
        },
    })
}

/// `name`'s aggregate of the values of the counterparts `sure`ly within
/// and of those `possible`ly within; `None` where it has no value.
fn combine(
    name: &str,
    sure: &[Interval],
    possible: &[Interval],
) -> Result<Option<Interval>, Unavailable> {
    let overflow = |_| {
        (
            NotEvaluatedReason::InvalidEvidence,
            "the value is not finite".to_owned(),
        )
    };
    let undecided = || {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("{} counterpart(s) may or may not be within", possible.len()),
        )
    };
    let value = match name {
        SUM_WITHIN => {
            let mut total = Interval::point(0.0);
            for value in sure {
                total = total.plus(*value).map_err(overflow)?;
            }
            // A counterpart that may be within adds its value or nothing.
            for value in possible {
                total = total
                    .plus(value.hull(Interval::point(0.0)))
                    .map_err(overflow)?;
            }
            total
        }
        MIN_WITHIN | MAX_WITHIN => {
            let least = name == MIN_WITHIN;
            let pick = |left: Interval, right: Interval| {
                if least {
                    left.min(right)
                } else {
                    left.max(right)
                }
            };
            let Some(result) = sure.iter().copied().reduce(pick) else {
                // None at all, or there may be none.
                return if possible.is_empty() {
                    Ok(None)
                } else {
                    Err(undecided())
                };
            };
            // One that may be within may lower a minimum (raise a maximum).
            possible
                .iter()
                .fold(result, |result, value| result.hull(pick(result, *value)))
        }
        _ => {
            if !possible.is_empty() {
                return Err(undecided());
            }
            if sure.is_empty() {
                return Ok(None);
            }
            let mut total = Interval::point(0.0);
            for value in sure {
                total = total.plus(*value).map_err(overflow)?;
            }
            #[allow(clippy::cast_precision_loss)]
            total
                .divided_by(Interval::point(sure.len() as f64))
                .map_err(overflow)?
        }
    };
    Ok(Some(value))
}

/// The property a `*_within` reads of each counterpart.
fn value_property(call: &MeasuredCall) -> Option<PropertyRef<'_>> {
    match call.argument("value") {
        Some(MeasuredArgument::Property { set, name }) => Some(PropertyRef {
            set: set.as_deref(),
            name,
        }),
        _ => None,
    }
}

/// A value over the counterparts in the context zone `call` names, with
/// why each counterpart that may lie in it could not be located.
fn in_zone(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Measurement, Citation), PropertyResolutionError> {
    let name = call.name();
    let Some(MeasuredArgument::Text(zone)) = call.argument("zone") else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let refused = |(reason, why): Unavailable| {
        crate::measured_kinds::resolution_error((reason, format!("`{name}` of {object}: {why}")))
    };
    let counterparts = crate::measured_kinds::objects_of_kinds(context, call, "to", object)?;
    let (reached, notes) = Reached::zoned(context, call, &counterparts).map_err(refused)?;
    let locator = format!("{name}:{object}:zone={zone}:{}", reached.counterparts.len());
    let measurement = if name == COUNT_WITHIN {
        reached.count()
    } else {
        let property = value_property(call).ok_or(PropertyResolutionError::InvalidRequest)?;
        aggregate_within(name, context, &reached, property).map_err(refused)?
    };
    Ok((
        located(measurement, locator),
        Citation {
            notes,
            ..Citation::default()
        },
    ))
}

impl MeasuredProvider for DistanceMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            COUNT_WITHIN,
            DISTANCE,
            MAX_WITHIN,
            MEAN_WITHIN,
            MIN_WITHIN,
            SUM_WITHIN,
        ]
    }

    fn measure_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        if call.argument("zone").is_some() {
            return in_zone(call, object, context);
        }
        Ok((self.measure(call, object, context)?, Citation::default()))
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        if call.argument("zone").is_some() {
            return in_zone(call, object, context).map(|(measurement, _)| measurement);
        }
        let name = call.name();
        let unavailable = |why: String| {
            PropertyResolutionError::Unavailable(format!("`{name}` of {object}: {why}"))
        };
        let counterparts = crate::measured_kinds::objects_of_kinds(context, call, "to", object)?;
        let refused = |(reason, why): Unavailable| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{name}` of {object}: {why}"),
            ))
        };
        if let Some(group) = call.argument("group") {
            let (MeasuredArgument::Text(grouping), Some(property)) = (group, value_property(call))
            else {
                return Err(PropertyResolutionError::InvalidRequest);
            };
            let reached =
                Reached::grouped(context, grouping, object, &counterparts).map_err(refused)?;
            let locator = format!("{name}:{object}:{grouping}:{}", reached.counterparts.len());
            let measurement =
                aggregate_within(name, context, &reached, property).map_err(refused)?;
            return Ok(located(measurement, locator));
        }
        let reach = if name == DISTANCE {
            length(call, "within")
        } else {
            length(call, "radius")
        }
        .ok_or(PropertyResolutionError::InvalidRequest)?;
        let walking = name != DISTANCE && call.choice("measure") == Some("walking");
        let rule = rule(call, object, counterparts, (reach, walking));
        let declared = declaration(&rule).map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{name}` of {object}: {why}"),
            ))
        })?;
        let prepared = prepare(
            context,
            &rule,
            Some(declared.margin()),
            (declared.projection, Voids::Measured),
        )
        .map_err(|refused| {
            unavailable(refused.not_evaluated_outcomes().first().map_or_else(
                || "the search could not run".into(),
                |outcome| outcome.message().to_owned(),
            ))
        })?;
        if !prepared.subjects.contains(object) || prepared.unmeasurable_subjects.contains(object) {
            return Err(unavailable("its extent could not be read".into()));
        }
        let service = prepared.service;
        let bodies = Bodies::from(prepared);
        let everything = crate::support::everything(context);
        let mut caches = Caches::default();
        let mut scope = Scope::new(&declared, None, (context, &everything), &mut caches);
        let (measured, unmeasurable) = bodies.candidates(
            service,
            declared.projection,
            &mut scope,
            (object, &std::collections::BTreeSet::new()),
        );
        let all: Vec<&Candidate> = measured.iter().chain(&unmeasurable).collect();
        let locator = format!("{name}:{object}:{}", all.len());
        let from = length(call, "from").unwrap_or(0.0);
        let measurement = match name {
            COUNT_WITHIN => count(&all, from, reach),
            DISTANCE => {
                extreme(&all, call.choice("mode") == Some("farthest")).map_err(unavailable)?
            }
            _ => {
                let property =
                    value_property(call).ok_or(PropertyResolutionError::InvalidRequest)?;
                let reached = if walking {
                    Reached::walked(context, call, object, (&all, from, reach)).map_err(refused)?
                } else {
                    Reached::measured(&all, from, reach)
                };
                aggregate_within(name, context, &reached, property).map_err(refused)?
            }
        };
        Ok(located(measurement, locator))
    }
}

/// `measurement`, its locator led by `locator`.
fn located(mut measurement: Measurement, locator: String) -> Measurement {
    let (Measurement::Value { locator: cited, .. }
    | Measurement::Rounded { locator: cited, .. }
    | Measurement::Cited { locator: cited, .. }
    | Measurement::Absent { locator: cited }) = &mut measurement;
    *cited = if cited.is_empty() {
        locator
    } else {
        format!("{locator}: {cited}")
    };
    measurement
}
