//! `keyed-limit`'s `threshold-step`: the step a door's threshold makes above
//! the floor on each of its sides, measured from geometry.
//!
//! The step on one side is `|door bottom + threshold − floor|`: the door's
//! bottom from its vertical extent, the threshold's thickness where the rule
//! declares `threshold_thickness`, and the floor the bottom of each space
//! `floor_path` reaches. With `ramp_selector` and `ramp_reach`, a selected
//! ramp within that plan distance of the door that overlaps a space in plan
//! is that side's floor instead, measured at its top: a door at the top of
//! a ramp steps onto the ramp, not onto the space's floor below it.

use axioval_engine::{
    Deviation, NotEvaluatedReason, ProximityProjection, ProximityRequest, ProximityServiceHandle,
    RuleContext, VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId};

use super::{difference, judge_as_displayed, thickness};
use crate::counts::Population;
use crate::level_spacing::{extent, extents};
use crate::light_area::length;
use crate::plan_area::{Verdict, deviation, shown};
use crate::support::{Parameters, PropertyRef, Traversal, Unavailable, finding, invalid};

/// The threshold-step quantity's declaration.
pub(crate) struct ThresholdStep<'a> {
    floor: Traversal,
    threshold: Option<PropertyRef<'a>>,
    ramps: Option<(&'a Selector, f64)>,
}

/// Whether something holds: surely, possibly, or surely not.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tri {
    No,
    Maybe,
    Sure,
}

impl Tri {
    fn and(self, other: Self) -> Self {
        self.min(other)
    }
}

/// One floor a side may step onto: how a finding names it, its elevation
/// interval or why it is unknown, and the objects it relates.
struct Floor {
    name: String,
    elevation: Result<(f64, f64), String>,
    evidence: Vec<Evidence>,
    related: ObjectId,
}

/// A ramp near the door, and its top.
struct Ramp {
    id: ObjectId,
    near: Tri,
    top: Result<((f64, f64), Evidence), String>,
}

impl<'a> ThresholdStep<'a> {
    pub(crate) fn parse(
        parameters: &Parameters<'a>,
        floor: Traversal,
    ) -> Result<Self, Unavailable> {
        let threshold = parameters.property("threshold_thickness")?;
        let ramps = match (
            parameters.selector("ramp_selector")?,
            length(parameters, "ramp_reach")?,
        ) {
            (Some(selector), Some(reach)) => Some((selector, reach)),
            (None, None) => None,
            _ => {
                return Err(invalid(
                    "`ramp_selector` and `ramp_reach` are declared together",
                ));
            }
        };
        Ok(Self {
            floor,
            threshold,
            ramps,
        })
    }

    /// Judges the step on every side of `subject` against `(minimum,
    /// maximum)`: one side failing is a finding naming its floor;
    /// otherwise a side that cannot be decided leaves the door not
    /// evaluated.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn judge(
        &self,
        context: &RuleContext<'_>,
        rule: &axioval_engine::CompiledRule,
        subject: &Object,
        (minimum, maximum): (Option<f64>, Option<f64>),
        described: &str,
        mut evidence: Vec<Evidence>,
        mut related: Vec<ObjectId>,
    ) -> Result<Option<(Finding, Option<Deviation>)>, Unavailable> {
        let service = extents(context)?;
        let door = extent(service, &subject.id)?;
        let threshold = match self.threshold {
            None => Threshold::None,
            Some(property) => match thickness(context, subject, property, &mut evidence)? {
                Some(value) => Threshold::Stated(value),
                None => Threshold::Unknown(property.to_string()),
            },
        };
        let everything: Vec<&Object> = context.project.objects().collect();
        let (spaces, cited) = self.floor.related(context, &subject.id, &everything)?;
        if spaces.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} reaches no floor to measure from",
                    self.floor.relationship
                ),
            ));
        }
        evidence.extend(cited);
        let ramps = self.ramps(context, service, &subject.id, &spaces);
        let proximity = context.services.get::<ProximityServiceHandle>();
        let bottom = (door.bottom().lower_metres(), door.bottom().upper_metres());
        let mut failed = Vec::new();
        let mut worst: Option<Deviation> = None;
        let mut open = Vec::new();
        for space in &spaces {
            let (sure, possible) = sides(proximity, service, space, &ramps);
            let mut fails = Vec::new();
            let mut passes = true;
            let mut undecided = Vec::new();
            let judged: Vec<(bool, &Floor, Judged)> = sure
                .iter()
                .map(|floor| {
                    (
                        true,
                        floor,
                        step(bottom, &threshold, floor, minimum, maximum),
                    )
                })
                .chain(possible.iter().map(|floor| {
                    (
                        false,
                        floor,
                        step(bottom, &threshold, floor, minimum, maximum),
                    )
                }))
                .collect();
            for (is_sure, floor, judged) in &judged {
                match judged {
                    Judged::Pass => {}
                    Judged::Fail(message, missed) => {
                        passes = false;
                        fails.push((*is_sure, *floor, message.clone(), *missed));
                    }
                    Judged::Undecided(message) => {
                        passes = false;
                        undecided.push(message.clone());
                    }
                }
            }
            let sure_fails: Vec<_> = fails.iter().filter(|(is_sure, ..)| *is_sure).collect();
            let all_fail = sure.is_empty()
                && !possible.is_empty()
                && fails.len() == possible.len()
                && undecided.is_empty();
            if !sure_fails.is_empty() || all_fail {
                let chosen: Vec<_> = if sure_fails.is_empty() {
                    fails.iter().collect()
                } else {
                    sure_fails
                };
                for (_, floor, message, missed) in chosen {
                    failed.push(message.clone());
                    worst = match (worst, *missed) {
                        (Some(worst), Some(missed)) => Some(worst.worst(missed)),
                        (worst, missed) => worst.or(missed),
                    };
                    evidence.extend(floor.evidence.iter().cloned());
                    related.push(floor.related.clone());
                }
            } else if !passes {
                if undecided.is_empty() {
                    undecided.push(format!(
                        "the floor beside {space} may be any of several, and not every one fails"
                    ));
                }
                open.extend(undecided);
            }
        }
        if failed.is_empty() {
            return if open.is_empty() {
                Ok(None)
            } else {
                Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("{} ({described})", open.join("; ")),
                ))
            };
        }
        evidence.push(door.evidence().clone());
        related.sort();
        related.dedup();
        Ok(Some((
            finding(
                rule,
                &subject.id,
                format!("{} ({described})", failed.join("; ")),
                evidence,
                related,
            ),
            worst,
        )))
    }

    /// The selected ramps near the door and their tops; none without
    /// `ramp_selector`.
    fn ramps(
        &self,
        context: &RuleContext<'_>,
        service: &VerticalExtentServiceHandle,
        door: &ObjectId,
        spaces: &[ObjectId],
    ) -> Vec<Ramp> {
        let Some((selector, reach)) = self.ramps else {
            return Vec::new();
        };
        let population = Population::of(context, selector);
        let proximity = context.services.get::<ProximityServiceHandle>();
        let door_box = proximity.and_then(|proximity| proximity.bounds(door).ok());
        let mut ramps = Vec::new();
        for ramp in population.matched.iter().chain(&population.undecided) {
            if ramp == door || spaces.contains(ramp) {
                continue;
            }
            let selected = if population.matched.contains(ramp) {
                Tri::Sure
            } else {
                Tri::Maybe
            };
            let near = match proximity {
                None => Tri::Maybe,
                Some(proximity) => near(proximity, door_box.as_ref(), door, ramp, reach),
            };
            let near = selected.and(near);
            if near == Tri::No {
                continue;
            }
            let top = extent(service, ramp)
                .map(|measured| {
                    let top = measured.top();
                    (
                        (top.lower_metres(), top.upper_metres()),
                        measured.evidence().clone(),
                    )
                })
                .map_err(|(_, why)| why);
            ramps.push(Ramp {
                id: ramp.clone(),
                near,
                top,
            });
        }
        ramps
    }
}

/// Whether a ramp lies within `reach` of the door in plan.
fn near(
    proximity: &ProximityServiceHandle,
    door_box: Option<&axioval_engine::ObjectBounds>,
    door: &ObjectId,
    ramp: &ObjectId,
    reach: f64,
) -> Tri {
    if let (Some(door_box), Ok(ramp_box)) = (door_box, proximity.bounds(ramp)) {
        let (a, b) = (door_box.enclosing(), ramp_box.enclosing());
        let gap = |axis: usize| (a.min()[axis] - b.max()[axis]).max(b.min()[axis] - a.max()[axis]);
        if gap(0).max(0.0).hypot(gap(1).max(0.0)) > reach {
            return Tri::No;
        }
    }
    let Ok(request) =
        ProximityRequest::projected(door.clone(), ramp.clone(), ProximityProjection::Horizontal)
    else {
        return Tri::Maybe;
    };
    match proximity.measure_distance(&request) {
        Ok(measured) => {
            let (lower, upper) = measured.interval_metres();
            if upper <= reach {
                Tri::Sure
            } else if lower > reach {
                Tri::No
            } else {
                Tri::Maybe
            }
        }
        Err(_) => Tri::Maybe,
    }
}

/// Whether a ramp overlaps a space in plan.
fn overlaps(proximity: Option<&ProximityServiceHandle>, ramp: &ObjectId, space: &ObjectId) -> Tri {
    let Some(proximity) = proximity else {
        return Tri::Maybe;
    };
    let Ok(request) = ProximityRequest::projected(
        ramp.clone(),
        space.clone(),
        ProximityProjection::PlanOverlap,
    ) else {
        return Tri::Maybe;
    };
    match proximity.measure_distance(&request) {
        Ok(measured) => match measured.interval_metres() {
            (_, upper) if upper <= 0.0 => Tri::Sure,
            (lower, _) if lower == f64::INFINITY => Tri::No,
            _ => Tri::Maybe,
        },
        Err(_) => Tri::Maybe,
    }
}

/// The floors beside `space`: those it surely steps onto, and those it may.
/// A ramp surely near and surely over the space replaces its floor; one
/// that only may leaves both possible.
fn sides(
    proximity: Option<&ProximityServiceHandle>,
    service: &VerticalExtentServiceHandle,
    space: &ObjectId,
    ramps: &[Ramp],
) -> (Vec<Floor>, Vec<Floor>) {
    let mut sure = Vec::new();
    let mut possible = Vec::new();
    for ramp in ramps {
        let beside = ramp.near.and(overlaps(proximity, &ramp.id, space));
        if beside == Tri::No {
            continue;
        }
        let (elevation, evidence) = match &ramp.top {
            Ok((top, cited)) => (Ok(*top), vec![cited.clone()]),
            Err(why) => (
                Err(format!(
                    "the top of ramp {} cannot be measured: {why}",
                    ramp.id
                )),
                vec![],
            ),
        };
        let floor = Floor {
            name: format!("the top of ramp {} beside {space}", ramp.id),
            elevation,
            evidence,
            related: ramp.id.clone(),
        };
        if beside == Tri::Sure {
            sure.push(floor);
        } else {
            possible.push(floor);
        }
    }
    if sure.is_empty() {
        let (elevation, evidence) = match extent(service, space) {
            Ok(measured) => {
                let bottom = measured.bottom();
                (
                    Ok((bottom.lower_metres(), bottom.upper_metres())),
                    vec![measured.evidence().clone()],
                )
            }
            Err((_, why)) => (
                Err(format!("the floor of {space} cannot be measured: {why}")),
                vec![],
            ),
        };
        let floor = Floor {
            name: format!("the floor of {space}"),
            elevation,
            evidence,
            related: space.clone(),
        };
        if possible.is_empty() {
            sure.push(floor);
        } else {
            possible.push(floor);
        }
    }
    (sure, possible)
}

/// The door's threshold as the rule and the door state it.
enum Threshold {
    /// The rule declares none: the door's bottom is its threshold.
    None,
    Stated(f64),
    /// Declared, and the door does not state it: at least zero.
    Unknown(String),
}

enum Judged {
    Pass,
    /// A failing step and how far it misses its bound.
    Fail(String, Option<Deviation>),
    Undecided(String),
}

/// The step from `floor` to the door's `bottom` and threshold against the
/// bounds.
fn step(
    (low, high): (f64, f64),
    threshold: &Threshold,
    floor: &Floor,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Judged {
    let (floor_low, floor_high) = match &floor.elevation {
        Ok(elevation) => *elevation,
        Err(why) => return Judged::Undecided(why.clone()),
    };
    let lower = difference(low, floor_high).0;
    let upper = difference(high, floor_low).1;
    let (lower, upper, note) = match threshold {
        Threshold::None => (lower, upper, String::new()),
        Threshold::Stated(value) => (
            difference(lower, -value).0,
            difference(upper, -value).1,
            format!(" with its {} m threshold", shown(*value, *value)),
        ),
        Threshold::Unknown(property) => (
            lower,
            f64::INFINITY,
            format!(" and a threshold {property} does not state"),
        ),
    };
    let (step_low, step_high) = if lower >= 0.0 {
        (lower, upper)
    } else if upper <= 0.0 {
        (-upper, -lower)
    } else {
        (0.0, (-lower).max(upper))
    };
    let measured = if step_high.is_finite() {
        format!("{} m", shown(step_low, step_high))
    } else {
        format!("at least {} m", shown(step_low, step_low))
    };
    let named = format!(
        "the step from {} to the door's bottom{note} is {measured}",
        floor.name
    );
    match judge_as_displayed(step_low, step_high, minimum, maximum) {
        Verdict::Pass => Judged::Pass,
        Verdict::Fail(bound) => Judged::Fail(
            format!("{named}; required {bound} m"),
            deviation(step_low, step_high, minimum, maximum),
        ),
        Verdict::Undecided(bound) => {
            Judged::Undecided(format!("{named}, which straddles the bound {bound} m"))
        }
    }
}
