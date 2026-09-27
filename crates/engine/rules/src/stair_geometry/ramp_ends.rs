//! The checks at a ramp's ends: a free space of given size in front of the
//! lowest and above the highest run, and no door standing on a landing.
//! Both place a box and ask the free-space service whether a selected
//! object reaches into it.

use axioval_engine::{
    BoxClearance, ClearanceOutcome, ClearanceRequest, ClearanceShape, ElevationInterval,
    FreeSpaceServiceHandle, Landing, LandingEvidence, MetricDirection, MetricFrame, MetricPoint,
    ParameterDescriptor, ParameterType, SlopedRun, across,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use super::{Check, length};
use crate::level_spacing::metres;
use crate::support::{Parameters, Unavailable, invalid};

/// The free space a ramp needs at each end.
pub(super) struct EndSpaceCheck<'a> {
    pub(super) obstacles: &'a Selector,
    depth: f64,
    width: f64,
    height: f64,
}

/// Doors must not stand on a ramp's landings.
pub(super) struct DoorCheck<'a> {
    pub(super) doors: &'a Selector,
    height: f64,
}

pub(super) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("end_space_depth", ParameterType::Quantity),
        ParameterDescriptor::optional("end_space_width", ParameterType::Quantity),
        ParameterDescriptor::optional("end_space_height", ParameterType::Quantity),
        ParameterDescriptor::optional("end_space_obstacles", ParameterType::Selector),
        ParameterDescriptor::optional("landing_doors", ParameterType::Selector),
        ParameterDescriptor::optional("landing_door_height", ParameterType::Quantity),
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
    match (doors, height) {
        (Some(doors), Some(height)) => Ok(Some(DoorCheck { doors, height })),
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

/// Whether a selected door stands on the landing at one end: reaches into
/// the column `check.height` high over the landing's rectangle.
pub(super) fn doors(
    free: Option<&FreeSpaceServiceHandle>,
    check: &DoorCheck<'_>,
    selected: &Result<(Vec<ObjectId>, bool), Unavailable>,
    measured: &LandingEvidence,
    elevation: ElevationInterval,
    label: &str,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let ramp = measured.request().subject();
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
    let (check, mut evidence, related) = assess(free, &placed, ramp, selected, &what, |names| {
        format!("door {names} stands on the landing at {label}")
    });
    evidence.insert(0, measured.evidence().clone());
    (check, evidence, related)
}
