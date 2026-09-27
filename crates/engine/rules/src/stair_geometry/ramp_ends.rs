//! The checks at the ends of a ramp or a flight: a free space of given size
//! in front of a ramp's lowest and above its highest run, or before a
//! flight's first and after its last step, and no door
//! standing on a landing at the end of a ramp's run or of a flight. Both
//! place a box and ask the free-space service whether a selected object
//! reaches into it. With `landing_door_swing`, no door may swing over a
//! landing either: its swing footprint (`door_swing::Footprint`) against
//! the landing's rectangle in plan, at the landing's level.

use axioval_engine::{
    BoxClearance, ClearanceOutcome, ClearanceRequest, ClearanceShape, ConvexPlanRegion,
    ElevationInterval, FreeSpaceServiceHandle, Landing, LandingEvidence, LandingRequest,
    MetricDirection, MetricFrame, MetricPoint, ObjectFrameServiceHandle, ParameterDescriptor,
    ParameterType, RuleContext, SlopedRun, Tread, TreadFlight, VerticalExtentServiceHandle,
    WalkingEnd, WalkingSurfaceServiceHandle, across,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use super::{Check, length};
use crate::door_swing::{self, Footprint};
use crate::level_spacing::{extent, metres};
use crate::support::{Parameters, Unavailable, invalid};

/// The free space a ramp or a flight needs at each end.
pub(super) struct EndSpaceCheck<'a> {
    pub(super) obstacles: &'a Selector,
    depth: f64,
    width: f64,
    height: f64,
}

/// Doors must not stand on the landings of a ramp or a flight, nor, with
/// `swing`, swing over them.
pub(super) struct DoorCheck<'a> {
    pub(super) doors: &'a Selector,
    height: f64,
    pub(super) swing: bool,
}

/// The end-space parameters a ramp and a flight share.
pub(super) fn end_space_descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("end_space_depth", ParameterType::Quantity),
        ParameterDescriptor::optional("end_space_width", ParameterType::Quantity),
        ParameterDescriptor::optional("end_space_height", ParameterType::Quantity),
        ParameterDescriptor::optional("end_space_obstacles", ParameterType::Selector),
    ]
}

/// The landing-door parameters a ramp and a flight share.
pub(super) fn door_descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("landing_doors", ParameterType::Selector),
        ParameterDescriptor::optional("landing_door_height", ParameterType::Quantity),
        ParameterDescriptor::optional("landing_door_swing", ParameterType::Boolean),
    ]
}

fn positive(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match length(parameters, name)? {
        Some(value) if value <= 0.0 => Err(invalid(format!("`{name}` must be positive"))),
        other => Ok(other),
    }
}

pub(super) fn parse_end_space<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<EndSpaceCheck<'a>>, Unavailable> {
    let depth = positive(parameters, "end_space_depth")?;
    let width = positive(parameters, "end_space_width")?;
    let height = positive(parameters, "end_space_height")?;
    let obstacles = parameters.selector("end_space_obstacles")?;
    match (depth, width, height, obstacles) {
        (Some(depth), Some(width), Some(height), Some(obstacles)) => Ok(Some(EndSpaceCheck {
            obstacles,
            depth,
            width,
            height,
        })),
        (None, None, None, None) => Ok(None),
        _ => Err(invalid(
            "`end_space_depth`, `end_space_width`, `end_space_height` and \
             `end_space_obstacles` are declared together",
        )),
    }
}

pub(super) fn parse_doors<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<DoorCheck<'a>>, Unavailable> {
    let doors = parameters.selector("landing_doors")?;
    let height = positive(parameters, "landing_door_height")?;
    let swing = parameters.boolean("landing_door_swing")?;
    match (doors, height) {
        (Some(doors), Some(height)) => Ok(Some(DoorCheck {
            doors,
            height,
            swing: swing.unwrap_or(false),
        })),
        (None, None) if swing.is_some() => Err(invalid(
            "`landing_door_swing` needs `landing_doors` and `landing_door_height`",
        )),
        (None, None) => Ok(None),
        _ => Err(invalid(
            "`landing_doors` and `landing_door_height` are declared together",
        )),
    }
}

fn middle(value: ElevationInterval) -> f64 {
    f64::midpoint(value.lower_metres(), value.upper_metres())
}

/// A box `width` across and `depth` along `direction`, centred at the
/// positions `along` and `across` it and standing on `elevation`.
struct Placed {
    direction: MetricDirection,
    along: f64,
    across: f64,
    elevation: f64,
    width: f64,
    depth: f64,
    height: f64,
}

impl Placed {
    fn request(
        &self,
        scope: &ObjectId,
        obstacles: Vec<ObjectId>,
    ) -> Result<ClearanceRequest, String> {
        let [dx, dy, _] = self.direction.components();
        let [ax, ay, _] = across(self.direction).components();
        let point = [
            self.along * dx + self.across * ax,
            self.along * dy + self.across * ay,
            self.elevation,
        ];
        let origin =
            MetricPoint::try_new(scope.clone(), point).map_err(|error| error.to_string())?;
        let axis = |vector: [f64; 3]| MetricDirection::try_new(vector).map_err(|e| e.to_string());
        // Right, forward and up must turn anticlockwise seen from above.
        let frame = MetricFrame::try_new(
            origin,
            axis([dy, -dx, 0.0])?,
            self.direction,
            axis([0.0, 0.0, 1.0])?,
        )
        .map_err(|error| error.to_string())?;
        let shape = BoxClearance::try_new(self.width, self.depth, self.height)
            .map_err(|error| error.to_string())?;
        Ok(ClearanceRequest::new(
            frame,
            ClearanceShape::Box(shape),
            obstacles,
        ))
    }
}

/// Asks whether a selected object reaches into the box: a blocker is a
/// finding (`found` names them), a clear box passes unless the selection
/// left an object undecided. `what` names the box in messages.
fn assess(
    free: Option<&FreeSpaceServiceHandle>,
    placed: &Placed,
    scope: &ObjectId,
    selected: &Result<(Vec<ObjectId>, bool), Unavailable>,
    what: &str,
    found: impl Fn(&str) -> String,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let (candidates, undecided) = match selected {
        Ok(selected) => selected,
        Err((_, message)) => return (Check::Undecided(message.clone()), vec![], vec![]),
    };
    let obstacles: Vec<ObjectId> = candidates
        .iter()
        .filter(|candidate| *candidate != scope)
        .cloned()
        .collect();
    let pending = || {
        Check::Undecided(format!(
            "nothing selected reaches into {what}, but an object the selection could not decide \
             may"
        ))
    };
    if obstacles.is_empty() {
        let check = if *undecided { pending() } else { Check::Pass };
        return (check, vec![], vec![]);
    }
    let Some(free) = free else {
        return (
            Check::Undecided(format!(
                "the free-space service is not registered, so {what} is not checked"
            )),
            vec![],
            vec![],
        );
    };
    let request = match placed.request(scope, obstacles) {
        Ok(request) => request,
        Err(error) => {
            return (
                Check::Undecided(format!("{what} cannot be placed: {error}")),
                vec![],
                vec![],
            );
        }
    };
    match free.assess_clearance(&request) {
        Ok(ClearanceOutcome::Clear(proof)) => {
            let check = if *undecided { pending() } else { Check::Pass };
            (check, vec![proof.evidence().clone()], vec![])
        }
        Ok(ClearanceOutcome::Obstructed(proof)) => {
            let names = proof
                .blockers()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            (
                Check::Fail(found(&names)),
                vec![proof.evidence().clone()],
                proof.blockers().to_vec(),
            )
        }
        Err(error) => (Check::Undecided(format!("{what}: {error}")), vec![], vec![]),
    }
}

/// The free space at the bottom (`top == false`) or top of a ramp, in
/// front of its lowest run's lower end or beyond its highest run's upper
/// end, centred on the run across it.
pub(super) fn end_space(
    free: Option<&FreeSpaceServiceHandle>,
    check: &EndSpaceCheck<'_>,
    obstacles: &Result<(Vec<ObjectId>, bool), Unavailable>,
    ramp: &ObjectId,
    run: &SlopedRun,
    top: bool,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let end = if top { "top" } else { "bottom" };
    let Some((left, right)) = run.sides() else {
        return (
            Check::Undecided(format!(
                "the ramp's run at its {end} fills no rectangle, so the free space there is not \
                 placed"
            )),
            vec![],
            vec![],
        );
    };
    let (along, elevation) = if top {
        (run.end().upper_metres() + check.depth / 2.0, run.top())
    } else {
        (run.start().lower_metres() - check.depth / 2.0, run.bottom())
    };
    let placed = Placed {
        direction: run.direction(),
        along,
        across: f64::midpoint(middle(left), middle(right)),
        elevation: elevation.upper_metres(),
        width: check.width,
        depth: check.depth,
        height: check.height,
    };
    let what = format!(
        "the free space at the {end} of the ramp ({} deep, {} wide)",
        metres(check.depth),
        metres(check.width)
    );
    assess(free, &placed, ramp, obstacles, &what, |names| {
        format!("{names} obstructs {what}")
    })
}

/// The free space at the bottom (`top == false`) or top of a flight, in
/// front of its first riser or beyond its last, standing on `elevation`
/// (the level at that end) and centred across the end tread. The end's
/// direction and arrival line come from the walking-surface service's
/// landing measurement, asked with no candidate; a position along it that
/// is an interval widens the box to hold every place it may start.
pub(super) fn flight_end_space(
    stairs: &WalkingSurfaceServiceHandle,
    free: Option<&FreeSpaceServiceHandle>,
    check: &EndSpaceCheck<'_>,
    obstacles: &Result<(Vec<ObjectId>, bool), Unavailable>,
    flight: &TreadFlight,
    top: bool,
    elevation: ElevationInterval,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let (end, word) = if top {
        (WalkingEnd::FlightTop, "top")
    } else {
        (WalkingEnd::FlightBottom, "bottom")
    };
    let tread = if top {
        flight.treads().last()
    } else {
        flight.treads().first()
    };
    let Some((left, right)) = tread.and_then(Tread::sides) else {
        return (
            Check::Undecided(format!(
                "the tread at the {word} of the flight fills no rectangle, so the free space \
                 there is not placed"
            )),
            vec![],
            vec![],
        );
    };
    let request = LandingRequest::new(flight.object().clone(), end, []);
    let measured = match stairs.measure_landing(&request) {
        Ok(measured) => measured,
        Err(error) => {
            return (
                Check::Undecided(format!(
                    "the free space at the {word} of the flight is not placed: {error}"
                )),
                vec![],
                vec![],
            );
        }
    };
    // A tread's sides lie across the direction it climbs; the bottom's
    // leaving direction runs the other way, so its positions turn over.
    let centre = f64::midpoint(middle(left), middle(right));
    let edge = measured.edge();
    let depth = check.depth + (edge.upper_metres() - edge.lower_metres());
    let placed = Placed {
        direction: measured.direction(),
        along: edge.lower_metres() + depth / 2.0,
        across: if top { centre } else { -centre },
        elevation: elevation.upper_metres(),
        width: check.width,
        depth,
        height: check.height,
    };
    let what = format!(
        "the free space at the {word} of the flight ({} deep, {} wide)",
        metres(check.depth),
        metres(check.width)
    );
    let (check, mut evidence, related) =
        assess(free, &placed, flight.object(), obstacles, &what, |names| {
            format!("{names} obstructs {what}")
        });
    evidence.insert(0, measured.evidence().clone());
    (check, evidence, related)
}

/// Whether a selected door stands on the landing at one end of a run or a
/// flight: reaches into the column `check.height` high over the landing's
/// rectangle.
pub(super) fn doors(
    free: Option<&FreeSpaceServiceHandle>,
    check: &DoorCheck<'_>,
    selected: &Result<(Vec<ObjectId>, bool), Unavailable>,
    measured: &LandingEvidence,
    elevation: ElevationInterval,
    label: &str,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let subject = measured.request().subject();
    let Some(landing) = measured.landing() else {
        // No landing there, so no door on one.
        return (Check::Pass, vec![], vec![]);
    };
    let Some(extent) = landing.extent() else {
        return (
            Check::Undecided(format!(
                "the landing {} at {label} fills no rectangle along the walking direction, so \
                 doors on it are not looked for",
                Landing::carrier(landing)
            )),
            vec![],
            vec![],
        );
    };
    let (left, right) = extent.sides();
    // The sure interior of the landing's rectangle.
    let (near, far) = (measured.edge().upper_metres(), extent.far().lower_metres());
    let (low, high) = (left.upper_metres(), right.lower_metres());
    if far <= near || high <= low {
        return (
            Check::Undecided(format!(
                "the landing at {label} is too small to look for doors on"
            )),
            vec![],
            vec![],
        );
    }
    let placed = Placed {
        direction: measured.direction(),
        along: f64::midpoint(near, far),
        across: f64::midpoint(low, high),
        elevation: elevation.upper_metres(),
        width: high - low,
        depth: far - near,
        height: check.height,
    };
    let what = format!("the landing at {label}");
    let (check, mut evidence, related) = assess(free, &placed, subject, selected, &what, |names| {
        format!("door {names} stands on the landing at {label}")
    });
    evidence.insert(0, measured.evidence().clone());
    (check, evidence, related)
}

/// A landing door's swing footprint and vertical extent, each read once.
pub(super) struct DoorSwing {
    door: ObjectId,
    footprint: Result<Footprint, Unavailable>,
    /// Bottom and top elevation intervals.
    extent: Result<Band, Unavailable>,
}

/// A door's bottom and top elevation intervals and their evidence.
type Band = ((f64, f64), (f64, f64), Evidence);

/// The swing footprints and extents of the selected landing doors.
pub(super) fn door_swings(context: &RuleContext<'_>, doors: &[ObjectId]) -> Vec<DoorSwing> {
    let frames = context.services.get::<ObjectFrameServiceHandle>();
    let extents = context.services.get::<VerticalExtentServiceHandle>();
    doors
        .iter()
        .map(|door| DoorSwing {
            door: door.clone(),
            footprint: frames
                .ok_or_else(|| {
                    (
                        axioval_ir::NotEvaluatedReason::MissingService,
                        "the object-frame service is not registered, so door leaves are unknown"
                            .into(),
                    )
                })
                .and_then(|frames| door_swing::leaves(frames, door))
                .and_then(|leaves| Footprint::of(&leaves)),
            extent: extents
                .ok_or_else(|| {
                    (
                        axioval_ir::NotEvaluatedReason::MissingService,
                        "the vertical-extent service is not registered".into(),
                    )
                })
                .and_then(|extents| extent(extents, door))
                .map(|measured| {
                    let (bottom, top) = (measured.bottom(), measured.top());
                    (
                        (bottom.lower_metres(), bottom.upper_metres()),
                        (top.lower_metres(), top.upper_metres()),
                        measured.evidence().clone(),
                    )
                }),
        })
        .collect()
}

/// A convex plan region from a rectangle `along` × `beside` the walking
/// direction `direction`, or `None` for an empty one.
fn rectangle(
    direction: MetricDirection,
    (near, far): (f64, f64),
    (low, high): (f64, f64),
) -> Option<ConvexPlanRegion> {
    if far <= near || high <= low {
        return None;
    }
    let [dx, dy, _] = direction.components();
    let [ax, ay, _] = across(direction).components();
    let at = |u: f64, v: f64| [u * dx + v * ax, u * dy + v * ay];
    let mut ring = vec![at(near, low), at(far, low), at(far, high), at(near, high)];
    let area: f64 = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum();
    if area < 0.0 {
        ring.reverse();
    }
    ConvexPlanRegion::try_new(ring).ok()
}

/// Whether a selected door swings over the landing at one end: a hinged
/// leaf's sector overlaps the landing's rectangle in plan, and the door
/// reaches into the column `check.height` high over it.
#[allow(clippy::too_many_lines)]
pub(super) fn door_swings_over(
    check: &DoorCheck<'_>,
    swings: &[DoorSwing],
    undecided: bool,
    measured: &LandingEvidence,
    elevation: ElevationInterval,
    label: &str,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let Some(landing) = measured.landing() else {
        return (Check::Pass, vec![], vec![]);
    };
    let Some(extent) = landing.extent() else {
        return (
            Check::Undecided(format!(
                "the landing {} at {label} fills no rectangle along the walking direction, so \
                 door swings over it are not looked for",
                Landing::carrier(landing)
            )),
            vec![],
            vec![],
        );
    };
    let (left, right) = extent.sides();
    let direction = measured.direction();
    // The sure rectangle and the one holding every position it may have.
    let inner = rectangle(
        direction,
        (measured.edge().upper_metres(), extent.far().lower_metres()),
        (left.upper_metres(), right.lower_metres()),
    );
    let outer = rectangle(
        direction,
        (measured.edge().lower_metres(), extent.far().upper_metres()),
        (left.lower_metres(), right.upper_metres()),
    );
    let Some(outer) = outer else {
        return (
            Check::Undecided(format!("the landing at {label} is too small to look at")),
            vec![],
            vec![],
        );
    };
    let (floor, ceiling) = (
        elevation.lower_metres(),
        elevation.upper_metres() + check.height,
    );
    let mut failing = Vec::new();
    let mut evidence = vec![measured.evidence().clone()];
    let mut open = None;
    for swing in swings {
        // A door wholly above or below the column cannot swing over it.
        let level = match &swing.extent {
            Ok((bottom, top, _)) if bottom.0 >= ceiling || top.1 <= floor => continue,
            Ok((bottom, top, cited)) => Ok((
                bottom.1 < ceiling && top.0 > elevation.upper_metres(),
                cited,
            )),
            Err(unavailable) => Err(unavailable),
        };
        let footprint = match &swing.footprint {
            Ok(footprint) => footprint,
            Err((_, message)) => {
                open.get_or_insert_with(|| message.clone());
                continue;
            }
        };
        let apart = footprint
            .parts
            .iter()
            .all(|(_, part)| part.separation(&outer) >= 0.0);
        if apart {
            continue;
        }
        let over = inner.as_ref().is_some_and(|inner| {
            footprint
                .parts
                .iter()
                .any(|(part, _)| part.separation(inner) < 0.0)
        });
        match level {
            Ok((true, cited)) if over => {
                failing.push(swing.door.clone());
                evidence.push(footprint.evidence.clone());
                evidence.push(cited.clone());
            }
            Ok(_) => {
                open.get_or_insert_with(|| {
                    format!(
                        "the swing of {} may reach over the landing at {label}",
                        swing.door
                    )
                });
            }
            Err((_, message)) => {
                open.get_or_insert_with(|| message.clone());
            }
        }
    }
    if !failing.is_empty() {
        let names: Vec<String> = failing.iter().map(ToString::to_string).collect();
        return (
            Check::Fail(format!(
                "door {} swings over the landing at {label}",
                names.join(", ")
            )),
            evidence,
            failing,
        );
    }
    match (open, undecided) {
        (Some(message), _) => (Check::Undecided(message), evidence, vec![]),
        (None, true) => (
            Check::Undecided(format!(
                "no selected door swings over the landing at {label}, but one the selection \
                 could not decide may"
            )),
            evidence,
            vec![],
        ),
        (None, false) => (Check::Pass, evidence, vec![]),
    }
}
