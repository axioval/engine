//! Vertical connectors measured for walks between levels.
//!
//! A stair or ramp is walked along its **walking line**, its centre line
//! from the foot of its lowest step (or the bottom of its sloped run) to
//! the head of its highest. A walk enters it at the lower end and leaves
//! it at the upper one; its **landings** stand just outside those ends,
//! on the floors below and above, moved out along the walking direction by
//! the body's radius and [`LANDING_GAP`]. The climb between the landings
//! is as long in plan as the straight line between them and rises from
//! the base to the top.
//!
//! Only what is measured exactly counts: a straight flight whose first and
//! last treads fill a rectangle (their sides give the line's ends and the
//! flight's width), or a ramp of one planar run filling one. A turning
//! flight, a ramp of several runs, a tessellation or a lift (which is
//! ridden, not walked) is not measured here, and a walk through it stays
//! undecided.

use axiolid_core::{Point2, Vec2};
use axioval_engine::{
    ClimbLength, ElevationInterval, HeadroomRequest, LengthInterval, VerticalConnector,
    VerticalConnectorKind, WalkingLine,
};
use axioval_ir::ObjectId;

use crate::geometry::AxiolidGeometry;
use crate::walking_surface::{headroom, sloped_runs, tread_flight};

/// How far past the walking line's end, beyond the body's radius, a
/// landing stands: clear of the connector's own body.
pub(crate) const LANDING_GAP: f64 = 0.001;

/// Relative allowance for the rounding of a plan length between landings.
const ROUNDING: f64 = 1e-12;

/// One end of a connector's walking line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct End {
    /// The walking line's end in plan, on the connector's boundary.
    pub(crate) at: Point2,
    /// The elevation of the floor this end meets.
    pub(crate) z: f64,
}

/// Whether a body can walk a connector.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Passable {
    /// It fits, with room to spare above and across.
    Proven,
    /// It surely does not fit.
    Refused(String),
    /// It may or may not.
    Undecided(String),
}

/// A stair or ramp measured for walks through it.
#[derive(Clone, Debug)]
pub(crate) struct Climb {
    pub(crate) id: ObjectId,
    pub(crate) kind: VerticalConnectorKind,
    /// The lower end, then the upper.
    pub(crate) ends: [End; 2],
    /// The walking direction, upwards, in plan.
    direction: Vec2,
    /// The narrowest width across the walking line.
    pub(crate) width: (f64, f64),
}

impl Climb {
    /// The landing in front of end `end` (0 lower, 1 upper) for a body of
    /// `radius`.
    pub(crate) fn landing(&self, end: usize, radius: f64) -> Point2 {
        let outward = if end == 0 {
            -self.direction
        } else {
            self.direction
        };
        self.ends[end].at + outward * (radius + LANDING_GAP)
    }

    /// The rise from the lower end to the upper, in metres.
    pub(crate) fn rise(&self) -> f64 {
        self.ends[1].z - self.ends[0].z
    }

    /// Bounds the length of the climb between the landings for a body of
    /// `radius`, counted by `climb`.
    pub(crate) fn length(&self, climb: ClimbLength, radius: f64) -> Result<LengthInterval, String> {
        let plan = (self.landing(1, radius) - self.landing(0, radius)).length();
        let rise = self.rise();
        let horizontal = LengthInterval::try_new(
            plan * (1.0 - ROUNDING),
            plan * (1.0 + ROUNDING) + f64::EPSILON,
        )
        .map_err(|e| e.to_string())?;
        let rise = LengthInterval::try_new(
            (rise * (1.0 - ROUNDING)).max(0.0),
            rise * (1.0 + ROUNDING) + f64::EPSILON,
        )
        .map_err(|e| e.to_string())?;
        Ok(climb.length(horizontal, rise))
    }

    /// Whether a body `body` wide and `height` high can walk the connector
    /// among `obstacles`: its narrowest width against the body, and the
    /// walking-surface headroom above it against the height.
    pub(crate) fn passable(
        &self,
        geometry: &AxiolidGeometry,
        obstacles: impl IntoIterator<Item = ObjectId>,
        body: f64,
        height: f64,
    ) -> Passable {
        let id = &self.id;
        if self.width.1 < body {
            return Passable::Refused(format!(
                "{id} is at most {:.6} m wide, narrower than the body",
                self.width.1
            ));
        }
        let mut undecided = Vec::new();
        if self.width.0 < body {
            undecided.push(format!(
                "{id} is at least {:.6} m wide, which may not admit the body",
                self.width.0
            ));
        }
        match headroom(geometry, &HeadroomRequest::new(id.clone(), obstacles)) {
            Ok(headroom) => match headroom.clearance() {
                None => {}
                Some(clearance) if clearance.lower() >= height => {}
                Some(clearance) if clearance.upper() < height => {
                    return Passable::Refused(format!(
                        "the headroom above {id} is at most {:.6} m, less than the body's height",
                        clearance.upper()
                    ));
                }
                Some(clearance) => undecided.push(format!(
                    "the headroom above {id} is at least {:.6} m, which may not clear the body",
                    clearance.lower()
                )),
            },
            Err(error) => undecided.push(format!("the headroom above {id} is unknown: {error}")),
        }
        if undecided.is_empty() {
            Passable::Proven
        } else {
            Passable::Undecided(undecided.join("; "))
        }
    }

    /// Why this connector is described as it is, for evidence.
    pub(crate) fn text(&self, climb: ClimbLength, radius: f64) -> String {
        let length = self.length(climb, radius).map_or_else(
            |_| "unbounded".to_owned(),
            |length| format!("{:.6}..{:.6}", length.lower_metres(), length.upper_metres()),
        );
        format!(
            "{}:{}:rise={:.6}:length={length}:{}*{:.6}",
            self.kind.as_str(),
            self.id,
            self.rise(),
            climb.measure().as_str(),
            climb.vertical_factor()
        )
    }
}

/// A plan point `along` the walking direction and `sideways` across it.
fn at(direction: Vec2, along: f64, sideways: f64) -> Point2 {
    let across = Vec2::new(-direction.y, direction.x);
    Point2::new(
        direction.x * along + across.x * sideways,
        direction.y * along + across.y * sideways,
    )
}

/// A single exactly known value.
fn point(lower: f64, upper: f64, what: &str, id: &ObjectId) -> Result<f64, String> {
    #[allow(clippy::float_cmp)]
    if lower == upper {
        Ok(lower)
    } else {
        Err(format!("{id}'s {what} is not known exactly"))
    }
}

/// Measures a stair or ramp's walking line, rise and width.
pub(crate) fn measure(
    geometry: &AxiolidGeometry,
    connector: &VerticalConnector,
) -> Result<Climb, String> {
    let id = connector.object();
    match connector.kind() {
        VerticalConnectorKind::Lift => Err(format!(
            "{id} is a lift, which is ridden rather than walked: no walking length is measured \
             for it"
        )),
        VerticalConnectorKind::Stair => stair(geometry, id),
        VerticalConnectorKind::Ramp => ramp(geometry, id),
    }
}

fn stair(geometry: &AxiolidGeometry, id: &ObjectId) -> Result<Climb, String> {
    let flight = tread_flight(geometry, id).map_err(|e| format!("stair {id}: {e}"))?;
    if !flight.is_exact() {
        return Err(format!(
            "stair {id} is not measured exactly, so its walking line's ends are not known"
        ));
    }
    let WalkingLine::Straight(direction) = flight.walking_line() else {
        return Err(format!(
            "stair {id} turns; a turning flight's walking length is not measured"
        ));
    };
    let [dx, dy, _] = direction.components();
    let direction = Vec2::new(dx, dy);
    let treads = flight.treads();
    let (Some(first), Some(last)) = (treads.first(), treads.last()) else {
        return Err(format!("stair {id} has no tread"));
    };
    let (Some(first_sides), Some(last_sides)) = (first.sides(), last.sides()) else {
        return Err(format!(
            "stair {id}'s first or last tread fills no rectangle, so where its walking line \
             ends is not known"
        ));
    };
    let width = flight
        .width()
        .ok_or_else(|| format!("stair {id} has a tread without sides, so no width"))?;
    let centre = |(left, right): (ElevationInterval, ElevationInterval)| {
        f64::midpoint(left.lower_metres(), right.lower_metres())
    };
    let start = point(
        first.front().lower_metres(),
        first.front().upper_metres(),
        "first nosing",
        id,
    )?;
    let end = point(
        last.back().lower_metres(),
        last.back().upper_metres(),
        "last tread's back",
        id,
    )?;
    let base = point(
        flight.base().lower_metres(),
        flight.base().upper_metres(),
        "base",
        id,
    )?;
    let rise = flight.rise();
    let top = base + point(rise.lower(), rise.upper(), "rise", id)?;
    let ends = [
        End {
            at: at(direction, start, centre(first_sides)),
            z: base,
        },
        End {
            at: at(direction, end, centre(last_sides)),
            z: top,
        },
    ];
    Ok(Climb {
        id: id.clone(),
        kind: VerticalConnectorKind::Stair,
        ends,
        direction,
        width: (width.lower(), width.upper()),
    })
}

fn ramp(geometry: &AxiolidGeometry, id: &ObjectId) -> Result<Climb, String> {
    let surface = sloped_runs(geometry, id).map_err(|e| format!("ramp {id}: {e}"))?;
    let [run] = surface.runs() else {
        return Err(format!(
            "ramp {id} has {} sloped runs; only a ramp of one run is walked",
            surface.runs().len()
        ));
    };
    if !run.is_exact() {
        return Err(format!("ramp {id} is not measured exactly"));
    }
    let (Some((left, right)), Some(width)) = (run.sides(), run.width()) else {
        return Err(format!(
            "ramp {id}'s run fills no rectangle, so where its walking line ends is not known"
        ));
    };
    let [dx, dy, _] = run.direction().components();
    let direction = Vec2::new(dx, dy);
    let sideways = f64::midpoint(left.lower_metres(), right.lower_metres());
    let (start, end) = (run.start().lower_metres(), run.end().lower_metres());
    let ends = [
        End {
            at: at(direction, start, sideways),
            z: run.bottom().lower_metres(),
        },
        End {
            at: at(direction, end, sideways),
            z: run.top().lower_metres(),
        },
    ];
    Ok(Climb {
        id: id.clone(),
        kind: VerticalConnectorKind::Ramp,
        ends,
        direction,
        width: (width.lower(), width.upper()),
    })
}

/// The across direction the engine measures sides along agrees with ours.
#[cfg(test)]
mod tests {
    use super::*;
    use axioval_engine::{MetricDirection, across};

    #[test]
    fn sides_run_along_the_engines_across_direction() {
        let direction = MetricDirection::try_new([0.6, 0.8, 0.0]).unwrap();
        let [x, y, _] = across(direction).components();
        let ours = at(Vec2::new(0.6, 0.8), 0.0, 1.0);
        assert!((ours.x - x).abs() < 1e-15 && (ours.y - y).abs() < 1e-15);
    }
}
