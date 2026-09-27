//! Tactile warning surfaces at a flight's ends: a strip `tactile_depth`
//! deep, starting `tactile_offset` before the first riser and beyond the
//! last, across the flight's width, covered by a selected tactile object
//! lying on the level there.
//!
//! A tactile object's footprint is read through the plan-span service's
//! least-area rectangle, which encloses it, and the plan-area service's
//! footprint area: a footprint whose area reaches the rectangle's, within
//! the rounding of decimal coordinates, fills it. The strip is covered when one object surely on the level, surely
//! selected and filling its rectangle surely holds every corner of the
//! strip; it is not when a point surely inside the strip lies surely
//! outside every tactile object that may lie on the level. Anything else
//! is not evaluated.

use std::collections::BTreeSet;

use axioval_engine::{
    ElevationInterval, LandingRequest, ParameterDescriptor, ParameterType, PlanRectangle,
    PlanSpanServiceHandle, RuleContext, Tread, TreadFlight, WalkingEnd,
    WalkingSurfaceServiceHandle, across,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use super::{Check, length, service_error, slack};
use crate::counts::Population;
use crate::level_spacing::{extent, extents, metres};
use crate::plan_area::footprint;
use crate::support::{Parameters, Unavailable, invalid};

/// How far above or below the level at a flight's end a tactile object may
/// reach and still lie on it: a floor finish, never a step.
const ON_LEVEL: f64 = 0.05;

/// The tactile check of one rule.
pub(super) struct TactileCheck<'a> {
    objects: &'a Selector,
    offset: f64,
    depth: f64,
    /// Whether the landings between a stair's flights need strips too.
    pub(super) intermediate: bool,
}

pub(super) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("tactile_objects", ParameterType::Selector),
        ParameterDescriptor::optional("tactile_offset", ParameterType::Quantity),
        ParameterDescriptor::optional("tactile_depth", ParameterType::Quantity),
        ParameterDescriptor::optional("tactile_on_intermediate_landings", ParameterType::Boolean),
    ]
}

pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<TactileCheck<'a>>, Unavailable> {
    let objects = parameters.selector("tactile_objects")?;
    let offset = length(parameters, "tactile_offset")?;
    let depth = length(parameters, "tactile_depth")?;
    let intermediate = parameters.boolean("tactile_on_intermediate_landings")?;
    match (objects, offset, depth) {
        (Some(objects), Some(offset), Some(depth)) if depth > 0.0 => Ok(Some(TactileCheck {
            objects,
            offset,
            depth,
            intermediate: intermediate.unwrap_or(false),
        })),
        (None, None, None) if intermediate.is_none() => Ok(None),
        _ => Err(invalid(
            "`tactile_objects`, `tactile_offset` and a positive `tactile_depth` are declared \
             together, and `tactile_on_intermediate_landings` only with them",
        )),
    }
}

/// Whether something holds: surely, possibly, or surely not.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tri {
    No,
    Maybe,
    Sure,
}

/// One object that may be a tactile surface, read once per rule.
pub(super) struct Tactile {
    id: ObjectId,
    selected: Tri,
    /// Its bottom and top elevation intervals.
    extent: Result<Band, String>,
    /// Its enclosing rectangle and whether its footprint fills it.
    plan: Result<(PlanRectangle, bool, Vec<Evidence>), String>,
}

/// The objects `check` selects or may select, each measured once.
pub(super) fn read(context: &RuleContext<'_>, check: &TactileCheck<'_>) -> Vec<Tactile> {
    let population = Population::of(context, check.objects);
    let spans = context.services.get::<PlanSpanServiceHandle>();
    let heights = extents(context);
    population
        .matched
        .iter()
        .map(|id| (id, Tri::Sure))
        .chain(population.undecided.iter().map(|id| (id, Tri::Maybe)))
        .map(|(id, selected)| {
            let extent = heights
                .as_ref()
                .map_err(|(_, why)| why.clone())
                .and_then(|service| extent(service, id).map_err(|(_, why)| why))
                .map(|measured| {
                    let (bottom, top) = (measured.bottom(), measured.top());
                    (
                        (bottom.lower_metres(), bottom.upper_metres()),
                        (top.lower_metres(), top.upper_metres()),
                        measured.evidence().clone(),
                    )
                });
            let plan = spans
                .ok_or_else(|| "the plan-span service is not registered".to_owned())
                .and_then(|spans| {
                    spans
                        .measure_rectangle(id)
                        .map_err(|error| error.to_string())
                })
                .map(|rectangle| {
                    let [(_, first), (_, second)] = rectangle.half_extents_metres();
                    let most = 4.0 * first * second - slack(4.0 * first * second);
                    let mut cited = vec![rectangle.evidence().clone()];
                    let filled = match footprint(context, id) {
                        Ok(area) => {
                            cited.push(area.evidence().clone());
                            area.lower_square_metres() >= most
                        }
                        Err(_) => false,
                    };
                    (rectangle, filled, cited)
                });
            Tactile {
                id: id.clone(),
                selected,
                extent,
                plan,
            }
        })
        .collect()
}

/// A rectangle in the frame leaving a flight's end: along and across.
type Region = ((f64, f64), (f64, f64));

/// Bottom and top elevation intervals of a body, with their evidence.
type Band = ((f64, f64), (f64, f64), Evidence);

/// The strip at one end of a flight: the region it may take, the region
/// it surely takes, and the frame leaving the end.
struct Placed {
    outer: Region,
    inner: Region,
    direction: [f64; 3],
    side: [f64; 3],
    evidence: Evidence,
}

impl Placed {
    /// The plan point at `along` and `across` the leaving direction.
    fn at(&self, (along, beside): (f64, f64)) -> [f64; 2] {
        [
            along.mul_add(self.direction[0], beside * self.side[0]),
            along.mul_add(self.direction[1], beside * self.side[1]),
        ]
    }
}

/// Places the strip at the bottom (`top == false`) or top of `flight`,
/// along the direction and from the arrival line the landing measurement
/// gives, across the end tread.
fn place(
    stairs: &WalkingSurfaceServiceHandle,
    check: &TactileCheck<'_>,
    flight: &TreadFlight,
    top: bool,
) -> Result<Placed, String> {
    let (end, tread) = if top {
        (WalkingEnd::FlightTop, flight.treads().last())
    } else {
        (WalkingEnd::FlightBottom, flight.treads().first())
    };
    let (left, right) = tread
        .and_then(Tread::sides)
        .ok_or_else(|| "the tread there fills no rectangle".to_owned())?;
    let request = LandingRequest::new(flight.object().clone(), end, []);
    let measured = stairs
        .measure_landing(&request)
        .map_err(|error| service_error(&error).1)?;
    let edge = measured.edge();
    // A tread's sides lie across the direction it climbs; the bottom's
    // leaving direction runs the other way, so its positions turn over.
    let (outer_across, inner_across) = if top {
        (
            (left.lower_metres(), right.upper_metres()),
            (left.upper_metres(), right.lower_metres()),
        )
    } else {
        (
            (-right.upper_metres(), -left.lower_metres()),
            (-right.lower_metres(), -left.upper_metres()),
        )
    };
    Ok(Placed {
        outer: (
            (
                edge.lower_metres() + check.offset,
                edge.upper_metres() + check.offset + check.depth,
            ),
            outer_across,
        ),
        inner: (
            (
                edge.upper_metres() + check.offset,
                edge.lower_metres() + check.offset + check.depth,
            ),
            inner_across,
        ),
        direction: measured.direction().components(),
        side: across(measured.direction()).components(),
        evidence: measured.evidence().clone(),
    })
}

/// Whether the tactile strip at the bottom (`top == false`) or top of a
/// flight is covered, the level there `level`.
pub(super) fn strip(
    stairs: &WalkingSurfaceServiceHandle,
    check: &TactileCheck<'_>,
    tactiles: &[Tactile],
    flight: &TreadFlight,
    top: bool,
    level: ElevationInterval,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let (word, riser, place_word) = if top {
        ("top", "last", "beyond")
    } else {
        ("bottom", "first", "before")
    };
    let what = format!(
        "the tactile strip at the {word} of the flight ({} deep, {} {place_word} the {riser} \
         riser, across the flight)",
        metres(check.depth),
        metres(check.offset)
    );
    match place(stairs, check, flight, top) {
        Ok(placed) => judge(&placed, tactiles, level, &what),
        Err(why) => (
            Check::Undecided(format!("{what} is not placed: {why}")),
            vec![],
            vec![],
        ),
    }
}

/// Whether the tactile objects cover a placed strip on `level`.
fn judge(
    placed: &Placed,
    tactiles: &[Tactile],
    level: ElevationInterval,
    what: &str,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let strip = corners(placed.outer).map(|point| placed.at(point));
    let mut evidence = vec![placed.evidence.clone()];
    let mut related = BTreeSet::new();
    let mut unknown = Vec::new();
    let mut covered = false;
    let mut candidates = Vec::new();
    for tactile in tactiles {
        let on = match &tactile.extent {
            Ok((bottom, top, _)) => on_level(*bottom, *top, level),
            Err(_) => Tri::Maybe,
        };
        if on == Tri::No {
            continue;
        }
        let may = on.min(tactile.selected);
        match &tactile.plan {
            Err(why) => unknown.push(format!("{} cannot be placed in plan: {why}", tactile.id)),
            Ok((rectangle, filled, cited)) => {
                if may == Tri::Sure && !apart(rectangle, &strip, [placed.direction, placed.side]) {
                    related.insert(tactile.id.clone());
                    evidence.extend(cited.iter().cloned());
                    if let Ok((_, _, cited)) = &tactile.extent {
                        evidence.push(cited.clone());
                    }
                }
                covered |= *filled
                    && may == Tri::Sure
                    && strip
                        .iter()
                        .all(|point| inside(rectangle, *point) == Tri::Sure);
                candidates.push(rectangle);
            }
        }
    }
    if covered {
        return (Check::Pass, vec![], vec![]);
    }
    let related: Vec<ObjectId> = related.into_iter().collect();
    let (along, beside) = placed.inner;
    let witness = along.0 <= along.1
        && beside.0 <= beside.1
        && unknown.is_empty()
        && samples(placed.inner).into_iter().any(|point| {
            candidates
                .iter()
                .all(|rectangle| inside(rectangle, placed.at(point)) == Tri::No)
        });
    if !witness {
        let why = if unknown.is_empty() {
            "no selected tactile surface surely covers it, and none surely leaves part of it bare"
                .to_owned()
        } else {
            unknown.join("; ")
        };
        return (
            Check::Undecided(format!("whether {what} is covered is not decided: {why}")),
            evidence,
            related,
        );
    }
    let message = if related.is_empty() {
        format!("no selected tactile surface lies in {what}")
    } else {
        format!(
            "{what} is not covered: {} leave part of it bare",
            related
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    (Check::Fail(message), evidence, related)
}

/// Whether `rectangle` surely lies apart from the convex quadrilateral
/// `strip` in plan: separated along one of the rectangle's axes or of
/// `axes`, the strip's, allowing for the rectangle's centre and axis error.
fn apart(rectangle: &PlanRectangle, strip: &[[f64; 2]; 4], axes: [[f64; 3]; 2]) -> bool {
    let centre = rectangle.centre();
    let [(_, first), (_, second)] = rectangle.half_extents_metres();
    let [u, v] = rectangle.axes();
    let reach = strip
        .iter()
        .map(|point| (point[0] - centre[0]).hypot(point[1] - centre[1]))
        .fold(0.0_f64, f64::max);
    let margin = reach.mul_add(
        rectangle.axis_error_radians(),
        rectangle.centre_radius_metres(),
    ) + 2.0 * slack(reach + centre[0].abs() + centre[1].abs());
    let own = [
        [
            centre[0] + first * u[0] + second * v[0],
            centre[1] + first * u[1] + second * v[1],
        ],
        [
            centre[0] + first * u[0] - second * v[0],
            centre[1] + first * u[1] - second * v[1],
        ],
        [
            centre[0] - first * u[0] - second * v[0],
            centre[1] - first * u[1] - second * v[1],
        ],
        [
            centre[0] - first * u[0] + second * v[0],
            centre[1] - first * u[1] + second * v[1],
        ],
    ];
    [u, v, [axes[0][0], axes[0][1]], [axes[1][0], axes[1][1]]]
        .iter()
        .any(|axis| {
            let project = |points: &[[f64; 2]]| {
                points
                    .iter()
                    .map(|point| point[0].mul_add(axis[0], point[1] * axis[1]))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                        (low.min(value), high.max(value))
                    })
            };
            let (a, b) = (project(&own), project(strip));
            a.0 - margin > b.1 || b.0 - margin > a.1
        })
}

/// Whether an object reaching from `bottom` to `top` lies on `level`.
fn on_level(bottom: (f64, f64), top: (f64, f64), level: ElevationInterval) -> Tri {
    let (low, high) = (level.lower_metres(), level.upper_metres());
    if bottom.0 > high + ON_LEVEL || top.1 < low - ON_LEVEL {
        Tri::No
    } else if bottom.1 <= low + ON_LEVEL && top.0 >= high - ON_LEVEL {
        Tri::Sure
    } else {
        Tri::Maybe
    }
}

fn corners(((a, b), (c, d)): Region) -> [(f64, f64); 4] {
    [(a, c), (a, d), (b, c), (b, d)]
}

/// The corners, the middles of the sides and the centre of a region.
fn samples(region: Region) -> Vec<(f64, f64)> {
    let ((near, far), (low, high)) = region;
    let (along, beside) = (f64::midpoint(near, far), f64::midpoint(low, high));
    let mut points = corners(region).to_vec();
    points.extend([
        (along, low),
        (along, high),
        (near, beside),
        (far, beside),
        (along, beside),
    ]);
    points
}

/// Whether `point` lies in the closed true rectangle `rectangle` bounds:
/// surely within the least half extents, surely beyond the greatest along
/// an axis, or undecided, allowing for the centre's and the axes' error and
/// a few units in the last place for the binary rounding of decimal
/// coordinates, so a strip exactly as large as required covers it.
fn inside(rectangle: &PlanRectangle, point: [f64; 2]) -> Tri {
    let centre = rectangle.centre();
    let offset = [point[0] - centre[0], point[1] - centre[1]];
    let reach = offset[0].hypot(offset[1]);
    let margin = reach.mul_add(
        rectangle.axis_error_radians(),
        rectangle.centre_radius_metres(),
    );
    let rounding = 2.0 * slack(point[0].abs().max(point[1].abs()).max(reach));
    let mut sure = true;
    for (axis, (lower, upper)) in rectangle.axes().iter().zip(rectangle.half_extents_metres()) {
        let along = offset[0].mul_add(axis[0], offset[1] * axis[1]).abs();
        if along - margin > upper + rounding {
            return Tri::No;
        }
        if along + margin > lower + rounding {
            sure = false;
        }
    }
    if sure { Tri::Sure } else { Tri::Maybe }
}
