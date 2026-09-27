//! Stair flights from the planes of a mesh.
//!
//! ADR 0004: this module measures positions; whether a riser is too high, a
//! winder too sharp or a riser open is a rule's decision.
//!
//! - **Treads** are the horizontal planes plane detection
//!   (`axiolid_inspect::detect_planes`, axiolid/kernel#131) finds facing
//!   up: planes whose certified deviation and whose corners' spread in
//!   elevation both stay within the level tolerance, which is the rounding
//!   a placement leaves in coordinates plus twice a tessellation's chord
//!   deviation. Planes at one elevation are one tread. A tread's elevation
//!   is the interval of its corners' elevations, widened by the chord
//!   deviation: the true surface lies within it of the mesh. Risers and the
//!   rest are separate planes. An upward plane flatter than 45° that is not
//!   level is refused: it is neither a tread nor measured.
//! - A flight whose treads' centres follow one line is **straight**: it is
//!   walked along the line from the lowest tread's centre to the highest's,
//!   positions are projections onto it (exact along a coordinate axis).
//!   Otherwise it **turns** (winders, a quarter turn). A first walk along
//!   the polyline through the treads' centroids finds each tread's nosing
//!   and back edge. A tread whose two edges are parallel is crossed through
//!   its centroid along its nosing; a winder is crossed along the bisector
//!   of its two edges' lines. The inner side is the one the winders turn
//!   towards,
//!   about where their nosing's and back edge's lines meet (the overall
//!   turn of the centroids when there is no winder); winders turning both
//!   ways leave no inner side. The walking line has a vertex on each
//!   chord, midway or at the requested distance from its inner end, and
//!   positions along it are arc lengths. Those are floating-point
//!   constructions, widened by a relative margin plus the chord deviation,
//!   divided by the sine of the angle at which the line crosses the
//!   tread's edge, and never exact.
//! - A tread's **sides** are stated only where its faces fill a rectangle
//!   along its walking direction (`walking_surface::rectangle`, as for a
//!   ramp's run): the flight's direction for a straight flight, the
//!   direction square to its nosing for a turning flight's tread whose
//!   nosing and back edge are parallel. A winder tapers and fills no such
//!   rectangle, so it has no sides and the flight no width; a tessellated
//!   tread's sides widen by the chord deviation. A turning flight's tread
//!   whose nosing is parallel to the one below within their uncertainty
//!   walks in that tread's frame, so a straight run of treads is one frame:
//!   a part its landings and handrails are placed in ([`TreadFrame`]).
//! - A tread's **nosing** is the boundary edge the walking line climbs onto
//!   it across, extended over the tread's boundary edges on its line.
//! - A **riser** between two treads is open where a boundary edge on the
//!   lower tread's back line (the edge the walking line leaves it across)
//!   meets a face falling away below it, and closed where every such edge
//!   meets a face rising above it. How far a rising face rises is not
//!   measured. The first riser is closed when the faces in the vertical
//!   plane of the first nosing cover the strip below it down to the base;
//!   otherwise it is not measured, since a riser set back behind a nosing
//!   leaves the same strip uncovered as an open one.

use std::collections::{BTreeMap, BTreeSet};

use axiolid_inspect::{PlaneTolerance, detect_planes};
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axioval_engine::{
    ElevationInterval, MetricDirection, PlanSegment, RiserClosure, Tread, TreadFlight,
    TreadFlightRequest, WalkingLine, WalkingLinePlacement, WalkingSurfaceError,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::Triangle;
use crate::walking_surface::{
    LEVEL_TOLERANCE, PlanFrame, Projection, STRAIGHT_TOLERANCE, Solid, plan_cross, plan_direction,
    rectangle,
};

/// The widest angle, in radians, between a triangle and the plane it joins
/// while planes grow. Treads and risers meet at 90°.
const PLANE_ANGLE: f64 = 0.05;

/// Relative numerical margin on positions a turning flight is measured at,
/// scaled by the coordinates' magnitude.
const MARGIN: f64 = 1e-9;

/// The least sine of the angle at which the walking line, or a chord
/// across a tread, may cross the tread's edge. A grazing crossing is not
/// located well enough to measure from.
const GRAZING: f64 = 0.05;

/// A mesh edge as its triangle winds it.
type Edge = (u64, u64);

/// A plan point.
type Plan = [f64; 2];

/// One tread's faces in plan.
struct Region {
    low: f64,
    high: f64,
    members: BTreeSet<usize>,
    /// Counter-clockwise plan corners and their mesh indices.
    faces: Vec<([Plan; 3], [u64; 3])>,
    /// Edges only one member has, with their plan ends.
    boundary: Vec<(Edge, [Plan; 2])>,
    centroid: Plan,
    centre: Plan,
}

/// Where a line crosses a tread's edge.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    t: f64,
    edge: Edge,
    sine: f64,
}

/// What a flight's walk measured of one tread.
struct Walk {
    front: ElevationInterval,
    back: ElevationInterval,
    /// The direction the tread's sides lie across, when it may fill a
    /// rectangle along it: none for a winder.
    frame: Option<MetricDirection>,
    entry: Option<Crossing>,
    exit: Option<Crossing>,
}

fn unsupported(message: String) -> WalkingSurfaceError {
    WalkingSurfaceError::Unsupported(message)
}

fn interval(low: f64, high: f64) -> Result<ElevationInterval, WalkingSurfaceError> {
    ElevationInterval::try_new(low, high).map_err(|_| WalkingSurfaceError::InvalidMeasurement)
}

fn widened(value: ElevationInterval, by: f64) -> Result<ElevationInterval, WalkingSurfaceError> {
    if by == 0.0 {
        return Ok(value);
    }
    interval(value.lower_metres() - by, value.upper_metres() + by)
}

fn sub(a: Plan, b: Plan) -> Plan {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: Plan, b: Plan) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: Plan, b: Plan) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn unit(vector: Plan) -> Option<Plan> {
    let length = vector[0].hypot(vector[1]);
    (length > 0.0 && length.is_finite()).then(|| [vector[0] / length, vector[1] / length])
}

fn along(origin: Plan, direction: Plan, t: f64) -> Plan {
    [
        direction[0].mul_add(t, origin[0]),
        direction[1].mul_add(t, origin[1]),
    ]
}

/// A tread filling a rectangle along its walking direction, in plan
/// positions along that direction and across it ([`PlanFrame`]): the
/// frame a turning flight's landings and handrails are placed in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TreadFrame {
    /// The horizontal direction the tread climbs: the flight's, or square
    /// to its nosing on a turning flight.
    pub(crate) direction: MetricDirection,
    /// Its front edge (the nosing) and back edge along the direction.
    pub(crate) along: (ElevationInterval, ElevationInterval),
    /// Its sides across the direction, as the tread states them.
    pub(crate) sides: (ElevationInterval, ElevationInterval),
}

/// Measures the stair flight of a closed, outward, one-piece body.
pub(crate) fn measure(
    request: &TreadFlightRequest,
    solid: &Solid<'_>,
) -> Result<TreadFlight, WalkingSurfaceError> {
    measure_framed(request, solid).map(|(flight, _)| flight)
}

/// The stair flight of a closed, outward, one-piece body, and each tread's
/// rectangle where it fills one along its walking direction.
pub(crate) fn measure_framed(
    request: &TreadFlightRequest,
    solid: &Solid<'_>,
) -> Result<(TreadFlight, Vec<Option<TreadFrame>>), WalkingSurfaceError> {
    let object = request.object();
    let deviation = solid.deviation;
    let scale = solid.soup.iter().flatten().fold(1.0_f64, |scale, p| {
        scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    });
    let tolerance = LEVEL_TOLERANCE.mul_add(scale, 2.0 * deviation);
    let regions = treads(object, solid, tolerance)?;
    if regions.is_empty() {
        return Err(unsupported(format!(
            "{object} has no horizontal face looking up, so no tread"
        )));
    }
    if regions.len() < 2 {
        return Err(unsupported(format!(
            "{object} has a single tread, which gives no walking direction"
        )));
    }
    let straight = follows_a_line(&regions, STRAIGHT_TOLERANCE + 2.0 * deviation);
    let (line, walks, exact) = if straight {
        straight_walk(solid, &regions)?
    } else {
        turning_walk(object, request, &regions, scale, deviation)?
    };
    let edges = edge_map(solid.mesh);
    let (base, top) = solid
        .soup
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
            (low.min(point.z), high.max(point.z))
        });
    let linear = MARGIN.mul_add(scale, 2.0 * deviation);
    let mut treads = Vec::with_capacity(regions.len());
    let mut frames = Vec::with_capacity(regions.len());
    for (index, (region, walk)) in regions.iter().zip(&walks).enumerate() {
        let elevation = interval(region.low - deviation, region.high + deviation)?;
        let mut tread = Tread::try_new(elevation, walk.front, walk.back)?;
        let mut tread_frame = None;
        if let Some(direction) = walk.frame {
            let faces: Vec<Triangle> = region
                .members
                .iter()
                .map(|index| solid.soup[*index])
                .collect();
            // A tessellation's corners lie within its deviation of the true
            // surface, whose sides so lie within twice that of the mesh's
            // extremes; the true sides lie within the deviation beyond.
            let frame = PlanFrame::new(direction);
            if let Some([(front, back), (left, right)]) =
                rectangle(&faces, &frame, 2.0 * deviation)?
            {
                let sides = (widened(left, deviation)?, widened(right, deviation)?);
                tread = tread.with_sides(sides.0, sides.1)?;
                tread_frame = Some(TreadFrame {
                    direction,
                    along: (widened(front, deviation)?, widened(back, deviation)?),
                    sides,
                });
            }
        }
        frames.push(tread_frame);
        let nosing = walk
            .entry
            .and_then(|entry| on_line(region, entry.edge, linear))
            .and_then(|(_, from, to)| PlanSegment::try_new(from, to, deviation).ok());
        if let Some(nosing) = nosing {
            tread = tread.with_nosing(nosing);
        }
        let riser = if index == 0 {
            nosing.map_or(RiserClosure::NotMeasured, |nosing| {
                first_riser(solid, &nosing, base, region.low, linear)
            })
        } else {
            let below = &regions[index - 1];
            walks[index - 1]
                .exit
                .map_or(RiserClosure::NotMeasured, |exit| {
                    riser_between(solid.mesh, &edges, below, exit, tolerance, linear)
                })
        };
        treads.push(tread.with_riser_below(riser));
    }
    let evidence = Evidence {
        source: object.source.clone(),
        locator: format!("tread-flight:{object}"),
        exact: exact && deviation == 0.0,
    };
    // The riser above the last tread, should the flight end in one.
    let last = regions.len() - 1;
    let final_riser = walks[last].exit.map_or(RiserClosure::NotMeasured, |exit| {
        riser_between(solid.mesh, &edges, &regions[last], exit, tolerance, linear)
    });
    let flight = TreadFlight::try_new(
        request.clone(),
        line,
        interval(base - deviation, base + deviation)?,
        interval(top - deviation, top + deviation)?,
        treads,
        evidence,
    )?
    .with_final_riser(final_riser);
    Ok((flight, frames))
}

/// The upward level planes of the body, grouped by elevation, lowest
/// first.
fn treads(
    object: &ObjectId,
    solid: &Solid<'_>,
    tolerance: f64,
) -> Result<Vec<Region>, WalkingSurfaceError> {
    let planes = detect_planes(
        solid.mesh,
        PlaneTolerance {
            distance: tolerance,
            angle: PLANE_ANGLE,
        },
    )
    .map_err(|_| {
        WalkingSurfaceError::Unavailable(format!("the mesh of {object} cannot be read"))
    })?;
    let mut levels: Vec<(f64, f64, Vec<usize>)> = Vec::new();
    for plane in &planes {
        let members: Vec<usize> = plane
            .triangles
            .iter()
            .filter_map(|index| usize::try_from(*index).ok())
            .collect();
        let (low, high) = members
            .iter()
            .flat_map(|index| solid.soup[*index].iter())
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), p| {
                (low.min(p.z), high.max(p.z))
            });
        let (mut up, mut down, mut edge_on) = (0_usize, 0_usize, 0_usize);
        for index in &members {
            let (cross, bound) = plan_cross(&solid.soup[*index]);
            if cross.abs() <= bound {
                edge_on += 1;
            } else if cross > 0.0 {
                up += 1;
            } else {
                down += 1;
            }
        }
        let level = high - low <= tolerance && plane.deviation <= tolerance;
        if level {
            if up == 0 {
                // An underside or the bottom: level, facing down.
                continue;
            }
            if down > 0 || edge_on > 0 {
                return Err(unsupported(format!(
                    "a horizontal face of {object} is too small to tell whether it faces up"
                )));
            }
            levels.push((low, high, members));
            continue;
        }
        let normal = plane.normal;
        if normal.z > 0.0 {
            let steepness = normal.x.hypot(normal.y) / normal.z;
            if steepness < 1.0 - 1e-9 {
                return Err(unsupported(format!(
                    "{object} has a sloped face looking up, which is no tread"
                )));
            }
            if steepness <= 1.0 + 1e-9 {
                return Err(unsupported(format!(
                    "a face of {object} slopes at 45°, neither walkable nor steep"
                )));
            }
        }
    }
    levels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut grouped: Vec<(f64, f64, Vec<usize>)> = Vec::new();
    for (low, high, members) in levels {
        match grouped.last_mut() {
            Some(level) if low <= level.1 + tolerance => {
                level.1 = level.1.max(high);
                level.2.extend(members);
            }
            _ => grouped.push((low, high, members)),
        }
    }
    Ok(grouped
        .into_iter()
        .map(|(low, high, members)| region(solid, low, high, members))
        .collect())
}

fn region(solid: &Solid<'_>, low: f64, high: f64, members: Vec<usize>) -> Region {
    let mut faces = Vec::with_capacity(members.len());
    let mut counts: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    let (mut weighted, mut total) = ([0.0, 0.0], 0.0);
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for index in &members {
        let triangle = &solid.soup[*index];
        let corners = solid.mesh.triangle(*index);
        let plan = triangle.map(|p| [p.x, p.y]);
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            *counts.entry((a.min(b), a.max(b))).or_default() += 1;
        }
        let area = cross(sub(plan[1], plan[0]), sub(plan[2], plan[0])) / 2.0;
        for axis in 0..2 {
            let mean = (plan[0][axis] + plan[1][axis] + plan[2][axis]) / 3.0;
            weighted[axis] += area * mean;
            min[axis] = plan.iter().fold(min[axis], |m, p| m.min(p[axis]));
            max[axis] = plan.iter().fold(max[axis], |m, p| m.max(p[axis]));
        }
        total += area;
        faces.push((plan, corners));
    }
    let mut boundary = Vec::new();
    for (plan, corners) in &faces {
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            if counts.get(&(a.min(b), a.max(b))) == Some(&1) {
                boundary.push(((a, b), [plan[k], plan[(k + 1) % 3]]));
            }
        }
    }
    Region {
        low,
        high,
        members: members.into_iter().collect(),
        faces,
        boundary,
        centroid: [weighted[0] / total, weighted[1] / total],
        centre: [f64::midpoint(min[0], max[0]), f64::midpoint(min[1], max[1])],
    }
}

/// Whether every tread's centre lies on the line from the lowest tread's
/// centre to the highest's, each further along it than the one below.
fn follows_a_line(regions: &[Region], tolerance: f64) -> bool {
    let (Some(first), Some(last)) = (regions.first(), regions.last()) else {
        return false;
    };
    let Some(direction) = unit(sub(last.centre, first.centre)) else {
        return false;
    };
    let mut ahead = f64::NEG_INFINITY;
    for region in regions {
        let offset = sub(region.centre, first.centre);
        let next = dot(offset, direction);
        if cross(direction, offset).abs() > tolerance || next <= ahead {
            return false;
        }
        ahead = next;
    }
    true
}

/// Where the line `origin + t·direction` (a unit direction) runs inside a
/// tread: the widest interval of `t` holding `0` over which the line stays
/// on the tread's faces, and the edges it enters and leaves through.
/// `None` when the origin lies off the tread.
fn chord(
    region: &Region,
    origin: Plan,
    direction: Plan,
    slack: f64,
) -> Option<(Crossing, Crossing)> {
    let mut spans: Vec<(Crossing, Crossing)> = Vec::new();
    for (corners, indices) in &region.faces {
        let mut low: Option<Crossing> = None;
        let mut high: Option<Crossing> = None;
        let mut empty = false;
        for k in 0..3 {
            let (p, q) = (corners[k], corners[(k + 1) % 3]);
            let edge = sub(q, p);
            // Inside the counter-clockwise triangle: left of every edge.
            let alpha = cross(edge, sub(origin, p));
            let beta = cross(edge, direction);
            let length = edge[0].hypot(edge[1]);
            let crossing = |t: f64| Crossing {
                t,
                edge: (indices[k], indices[(k + 1) % 3]),
                sine: (beta / length).abs(),
            };
            if beta > 0.0 {
                let t = -alpha / beta;
                if low.is_none_or(|low| t > low.t) {
                    low = Some(crossing(t));
                }
            } else if beta < 0.0 {
                let t = -alpha / beta;
                if high.is_none_or(|high| t < high.t) {
                    high = Some(crossing(t));
                }
            } else if alpha < 0.0 {
                empty = true;
            }
        }
        if let (false, Some(low), Some(high)) = (empty, low, high)
            && low.t < high.t
        {
            spans.push((low, high));
        }
    }
    spans.sort_by(|a, b| a.0.t.total_cmp(&b.0.t));
    let holds = |(low, high): &(Crossing, Crossing)| low.t <= slack && high.t >= -slack;
    let mut current: Option<(Crossing, Crossing)> = None;
    for span in spans {
        current = match current {
            Some((low, high)) if span.0.t <= high.t + slack => {
                Some((low, if span.1.t > high.t { span.1 } else { high }))
            }
            Some(done) if holds(&done) => return Some(done),
            _ => Some(span),
        };
    }
    current.filter(holds)
}

/// The tread's boundary edges on the line of its boundary edge `edge`, and
/// their farthest ends along it. `None` when `edge` is not on the
/// tread's boundary.
fn on_line(region: &Region, edge: Edge, tolerance: f64) -> Option<(Vec<Edge>, Plan, Plan)> {
    let (_, [p, q]) = region
        .boundary
        .iter()
        .find(|(candidate, _)| *candidate == edge || *candidate == (edge.1, edge.0))?;
    let direction = unit(sub(*q, *p))?;
    let mut found = Vec::new();
    let (mut first, mut last) = ((f64::INFINITY, *p), (f64::NEG_INFINITY, *p));
    for (candidate, ends) in &region.boundary {
        if ends
            .iter()
            .all(|end| cross(direction, sub(*end, *p)).abs() <= tolerance)
        {
            found.push(*candidate);
            for end in ends {
                let position = dot(direction, sub(*end, *p));
                if position < first.0 {
                    first = (position, *end);
                }
                if position > last.0 {
                    last = (position, *end);
                }
            }
        }
    }
    Some((found, first.1, last.1))
}

/// A straight flight: positions are projections along the line through
/// the treads' centres.
fn straight_walk(
    solid: &Solid<'_>,
    regions: &[Region],
) -> Result<(WalkingLine, Vec<Walk>, bool), WalkingSurfaceError> {
    let (Some(first), Some(last)) = (regions.first(), regions.last()) else {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    };
    let direction = plan_direction(first.centre, last.centre)?;
    let [ux, uy, _] = direction.components();
    let forward = Projection::new(direction);
    let deviation = solid.deviation;
    let mut walks = Vec::with_capacity(regions.len());
    for region in regions {
        let points = || {
            region
                .members
                .iter()
                .flat_map(|index| solid.soup[*index].iter())
        };
        let (front, back) = forward.span(points())?;
        let crossings = chord(region, region.centroid, [ux, uy], MARGIN);
        walks.push(Walk {
            front: widened(front, deviation)?,
            back: widened(back, deviation)?,
            frame: Some(direction),
            entry: crossings.map(|(entry, _)| entry),
            exit: crossings.map(|(_, exit)| exit),
        });
    }
    #[allow(clippy::float_cmp)]
    let level = regions.iter().all(|region| region.low == region.high);
    Ok((
        WalkingLine::Straight(direction),
        walks,
        forward.on_axis && level,
    ))
}

/// A turning flight: positions are arc lengths along a polyline with a
/// vertex on each tread's chord across it.
///
/// A first walk along the treads' centroids finds each tread's nosing and
/// back edge. A tread whose two edges are parallel is crossed along its
/// nosing, and its sides are measured across the direction square to it; a
/// winder is crossed along the bisector of its two edges and has no sides.
/// The walking line's vertex lies on that chord, midway or at the requested
/// distance from its inner end.
fn turning_walk(
    object: &ObjectId,
    request: &TreadFlightRequest,
    regions: &[Region],
    scale: f64,
    deviation: f64,
) -> Result<(WalkingLine, Vec<Walk>, bool), WalkingSurfaceError> {
    let count = regions.len();
    let centroids: Vec<Plan> = regions.iter().map(|region| region.centroid).collect();
    let slack = MARGIN * scale;
    let linear = MARGIN.mul_add(scale, 2.0 * deviation);
    let margin = |crossing: &Crossing, number: usize| -> Result<f64, WalkingSurfaceError> {
        if crossing.sine < GRAZING {
            return Err(unsupported(format!(
                "a line across tread {number} of {object} grazes its edge"
            )));
        }
        Ok(MARGIN.mul_add(scale, deviation) / crossing.sine)
    };
    let edges = crossings(object, regions, &centroids, slack)?;
    let (shapes, left, right) = shapes(object, regions, &centroids, &edges, linear, deviation)?;
    let side = match (left, right) {
        (true, true) if request.walking_line() != WalkingLinePlacement::Centre => {
            return Err(unsupported(format!(
                "{object} turns both ways, so it has no inner side to measure its walking line \
                 from"
            )));
        }
        (true, _) => 1.0,
        (false, true) => -1.0,
        (false, false) => overall_turn(&centroids),
    };
    let mut vertices = Vec::with_capacity(count);
    let mut frames = Vec::with_capacity(count);
    // The frame of the tread below and its nosing, while it has one.
    let mut previous: Option<(MetricDirection, [Plan; 2])> = None;
    for (index, (region, (mut across, parallel, walking, nosing))) in
        regions.iter().zip(shapes).enumerate()
    {
        let number = index + 1;
        // Point it at the inner side: left of the walking direction when
        // the flight turns left.
        if side * cross(walking, across) < 0.0 {
            across = [-across[0], -across[1]];
        }
        let (outer, inner) = chord(region, region.centroid, across, slack).ok_or_else(|| {
            unsupported(format!(
                "the centroid of tread {number} of {object} lies off the tread"
            ))
        })?;
        let width = inner.t - outer.t;
        let offset = match request.walking_line() {
            WalkingLinePlacement::Centre => f64::midpoint(inner.t, outer.t),
            WalkingLinePlacement::FromInnerSide(distance) => {
                if distance >= width {
                    return Err(unsupported(format!(
                        "a walking line {distance} m from the inner side runs outside tread \
                         {number} of {object}, which is {width:.3} m across"
                    )));
                }
                inner.t - distance
            }
        };
        vertices.push(along(region.centroid, across, offset));
        // A parallel tread walks square to its nosing, the way the flight
        // climbs over it; exactly along an axis when its nosing is. A tread
        // whose nosing is parallel to the one below within its uncertainty
        // shares that tread's frame, so a straight run of treads is one
        // frame however its nosings round.
        let frame = if parallel {
            let square = if cross(across, walking) > 0.0 {
                [-across[1], across[0]]
            } else {
                [across[1], -across[0]]
            };
            let own = plan_direction([0.0, 0.0], square)?;
            Some(match previous {
                Some((below, lower)) if same_frame(below, own, lower, nosing, deviation) => below,
                _ => own,
            })
        } else {
            None
        };
        previous = frame.map(|direction| (direction, nosing));
        frames.push(frame);
    }
    let (arc, _) = arc_lengths(object, &vertices)?;
    let edges = crossings(object, regions, &vertices, slack)?;
    let mut walks = Vec::with_capacity(count);
    for (index, ((entry, exit), frame)) in edges.into_iter().zip(frames).enumerate() {
        let number = index + 1;
        let (before, after) = (margin(&entry, number)?, margin(&exit, number)?);
        let front = arc[index] + entry.t;
        let back = arc[index] + exit.t;
        walks.push(Walk {
            front: interval(front - before, front + before)?,
            back: interval(back - after, back + after)?,
            frame,
            entry: Some(entry),
            exit: Some(exit),
        });
    }
    Ok((WalkingLine::Turning(vertices), walks, false))
}

/// Whether a tread climbing along `own` over the nosing `upper` walks in
/// the frame `below` of the tread under it, whose nosing is `lower`: the
/// two nosings are parallel within their ends' uncertainty and the
/// directions point the same way.
fn same_frame(
    below: MetricDirection,
    own: MetricDirection,
    lower: [Plan; 2],
    upper: [Plan; 2],
    deviation: f64,
) -> bool {
    let [bx, by, _] = below.components();
    let [ox, oy, _] = own.components();
    let parallel = PlanSegment::try_new(lower[0], lower[1], deviation)
        .and_then(|a| PlanSegment::try_new(upper[0], upper[1], deviation).map(|b| (a, b)))
        .ok()
        .and_then(|(a, b)| a.angle_to(&b))
        .is_some_and(|angle| angle.lower() == 0.0);
    parallel && bx.mul_add(ox, by * oy) > 0.0
}

/// A tread's crossing direction, whether its nosing and back edge are
/// parallel, its walking direction and its nosing's ends.
type Shape = (Plan, bool, Plan, [Plan; 2]);

/// Each tread's [`Shape`], and whether any winder turns left or right
/// about where its nosing's line meets its back edge's.
fn shapes(
    object: &ObjectId,
    regions: &[Region],
    centroids: &[Plan],
    edges: &[(Crossing, Crossing)],
    linear: f64,
    deviation: f64,
) -> Result<(Vec<Shape>, bool, bool), WalkingSurfaceError> {
    let count = regions.len();
    let mut shapes = Vec::with_capacity(count);
    let (mut left, mut right) = (false, false);
    for (index, (region, (entry, exit))) in regions.iter().zip(edges).enumerate() {
        let (Some((_, nosing_from, nosing_to)), Some((_, back_from, back_to))) = (
            on_line(region, entry.edge, linear),
            on_line(region, exit.edge, linear),
        ) else {
            return Err(unsupported(format!(
                "the walking line of {object} crosses tread {} through its inside",
                index + 1
            )));
        };
        let (across, parallel) =
            across_direction([nosing_from, nosing_to], [back_from, back_to], deviation)?;
        let walking = sub(
            centroids[(index + 1).min(count - 1)],
            centroids[index.saturating_sub(1)],
        );
        if !parallel {
            let (a, b) = (sub(nosing_to, nosing_from), sub(back_to, back_from));
            let share = cross(sub(back_from, nosing_from), b) / cross(a, b);
            let pivot = along(nosing_from, a, share);
            let turn = cross(walking, sub(pivot, region.centroid));
            left |= turn > 0.0;
            right |= turn < 0.0;
        }
        shapes.push((across, parallel, walking, [nosing_from, nosing_to]));
    }
    Ok((shapes, left, right))
}

/// The direction a tread is crossed along, and whether its nosing and back
/// edge are parallel: along the nosing if so, along the bisector of the
/// two edges' lines if not.
fn across_direction(
    nosing: [Plan; 2],
    back: [Plan; 2],
    deviation: f64,
) -> Result<(Plan, bool), WalkingSurfaceError> {
    let (Some(along_nosing), Some(along_back)) =
        (unit(sub(nosing[1], nosing[0])), unit(sub(back[1], back[0])))
    else {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    };
    let parallel = PlanSegment::try_new(nosing[0], nosing[1], deviation)
        .and_then(|a| PlanSegment::try_new(back[0], back[1], deviation).map(|b| (a, b)))
        .ok()
        .and_then(|(a, b)| a.angle_to(&b))
        .is_some_and(|angle| angle.lower() == 0.0);
    if parallel {
        return Ok((along_nosing, true));
    }
    let along_back = if dot(along_nosing, along_back) < 0.0 {
        [-along_back[0], -along_back[1]]
    } else {
        along_back
    };
    let bisector = unit([
        along_nosing[0] + along_back[0],
        along_nosing[1] + along_back[1],
    ])
    .ok_or(WalkingSurfaceError::InvalidMeasurement)?;
    Ok((bisector, false))
}

/// Where the polyline through `points`, one on each tread, enters and
/// leaves each tread, as distances from the tread's own point: the entry
/// along the segment arriving there (the first segment for the first
/// tread), the exit along the segment leaving (the last for the last).
fn crossings(
    object: &ObjectId,
    regions: &[Region],
    points: &[Plan],
    slack: f64,
) -> Result<Vec<(Crossing, Crossing)>, WalkingSurfaceError> {
    let count = points.len();
    let (_, lengths) = arc_lengths(object, points)?;
    let segment = |to: usize| -> Result<Plan, WalkingSurfaceError> {
        unit(sub(points[to], points[to - 1])).ok_or(WalkingSurfaceError::InvalidMeasurement)
    };
    let mut found = Vec::with_capacity(count);
    for (index, region) in regions.iter().enumerate() {
        let number = index + 1;
        let lost = || {
            unsupported(format!(
                "the walking line of {object} leaves tread {number} where it should cross it"
            ))
        };
        let (entry, _) =
            chord(region, points[index], segment(index.max(1))?, slack).ok_or_else(lost)?;
        let (_, exit) = chord(
            region,
            points[index],
            segment((index + 1).min(count - 1))?,
            slack,
        )
        .ok_or_else(lost)?;
        if (index > 0 && -entry.t >= lengths[index])
            || (index + 1 < count && exit.t >= lengths[index + 1])
        {
            return Err(unsupported(format!(
                "tread {number} of {object} reaches past its neighbour's walking-line vertex"
            )));
        }
        found.push((entry, exit));
    }
    Ok(found)
}

/// `1` when the polyline through `centroids` turns left overall, `-1`
/// when it turns right.
fn overall_turn(centroids: &[Plan]) -> f64 {
    let total: f64 = centroids
        .windows(3)
        .map(|w| {
            let (a, b) = (sub(w[1], w[0]), sub(w[2], w[1]));
            cross(a, b).atan2(dot(a, b))
        })
        .sum();
    if total >= 0.0 { 1.0 } else { -1.0 }
}

/// The arc length at each vertex of a polyline, and the length of the
/// segment ending there.
fn arc_lengths(
    object: &ObjectId,
    vertices: &[Plan],
) -> Result<(Vec<f64>, Vec<f64>), WalkingSurfaceError> {
    let mut arc = vec![0.0; vertices.len()];
    let mut lengths = vec![0.0; vertices.len()];
    for index in 1..vertices.len() {
        let step = sub(vertices[index], vertices[index - 1]);
        let length = step[0].hypot(step[1]);
        if length == 0.0 {
            return Err(unsupported(format!(
                "the walking line of {object} meets itself between treads {index} and {}",
                index + 1
            )));
        }
        lengths[index] = length;
        arc[index] = arc[index - 1] + length;
    }
    Ok((arc, lengths))
}

/// Mesh edges to the triangles having them.
fn edge_map(mesh: &TriMesh) -> BTreeMap<(u64, u64), Vec<usize>> {
    let mut edges: BTreeMap<(u64, u64), Vec<usize>> = BTreeMap::new();
    for index in 0..mesh.triangle_count() {
        let corners = mesh.triangle(index);
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push(index);
        }
    }
    edges
}

/// Whether the riser from `below` to the next tread closes the step, read
/// along the back line of `below` (where the walking line leaves it).
fn riser_between(
    mesh: &TriMesh,
    edges: &BTreeMap<(u64, u64), Vec<usize>>,
    below: &Region,
    exit: Crossing,
    tolerance: f64,
    linear: f64,
) -> RiserClosure {
    let Some((line, _, _)) = on_line(below, exit.edge, linear) else {
        return RiserClosure::NotMeasured;
    };
    let (mut falling, mut rising) = (false, true);
    for (a, b) in line {
        let neighbours: Vec<usize> = edges
            .get(&(a.min(b), a.max(b)))
            .map(|faces| {
                faces
                    .iter()
                    .copied()
                    .filter(|face| !below.members.contains(face))
                    .collect()
            })
            .unwrap_or_default();
        let [neighbour] = neighbours[..] else {
            rising = false;
            continue;
        };
        let Some(third) = mesh
            .triangle(neighbour)
            .into_iter()
            .find(|corner| *corner != a && *corner != b)
            .and_then(|corner| usize::try_from(corner).ok())
            .filter(|corner| *corner < mesh.position_count())
        else {
            rising = false;
            continue;
        };
        let z = mesh.position(third).z;
        if z < below.low - tolerance {
            falling = true;
        }
        if z <= below.high + tolerance {
            rising = false;
        }
    }
    if falling {
        RiserClosure::Open
    } else if rising {
        RiserClosure::Closed
    } else {
        RiserClosure::NotMeasured
    }
}

/// Whether the body's faces in the vertical plane of the first nosing
/// cover the strip below it from the base up to the first tread.
fn first_riser(
    solid: &Solid<'_>,
    nosing: &PlanSegment,
    base: f64,
    tread: f64,
    linear: f64,
) -> RiserClosure {
    let (from, to) = (nosing.from(), nosing.to());
    let Some(direction) = unit(sub(to, from)) else {
        return RiserClosure::NotMeasured;
    };
    let length = dot(direction, sub(to, from));
    let height = tread - base;
    let mut covered = 0.0;
    for triangle in &solid.soup {
        if !triangle
            .iter()
            .all(|p| cross(direction, sub([p.x, p.y], from)).abs() <= linear)
        {
            continue;
        }
        let polygon: Vec<Plan> = triangle
            .iter()
            .map(|p| [dot(direction, sub([p.x, p.y], from)), p.z])
            .collect();
        covered += area(&clip_rectangle(polygon, [0.0, base], [length, tread]));
    }
    let lost = linear * 2.0 * (length + height) + 1e-9 * length * height;
    if covered >= length * height - lost {
        RiserClosure::Closed
    } else {
        RiserClosure::NotMeasured
    }
}

/// The unsigned area of a plan polygon.
fn area(polygon: &[Plan]) -> f64 {
    let count = polygon.len();
    (0..count)
        .map(|i| cross(polygon[i], polygon[(i + 1) % count]))
        .sum::<f64>()
        .abs()
        / 2.0
}

/// `polygon` clipped to the axis-aligned rectangle from `min` to `max`.
fn clip_rectangle(polygon: Vec<Plan>, min: Plan, max: Plan) -> Vec<Plan> {
    let mut output = polygon;
    for (axis, bound, keep_below) in [
        (0, min[0], false),
        (0, max[0], true),
        (1, min[1], false),
        (1, max[1], true),
    ] {
        let inside = |p: Plan| {
            if keep_below {
                p[axis] <= bound
            } else {
                p[axis] >= bound
            }
        };
        let input = std::mem::take(&mut output);
        for (index, from) in input.iter().enumerate() {
            let to = input[(index + 1) % input.len()];
            if inside(*from) {
                output.push(*from);
            }
            if inside(*from) != inside(to) {
                let share = (bound - from[axis]) / (to[axis] - from[axis]);
                output.push([
                    share.mul_add(to[0] - from[0], from[0]),
                    share.mul_add(to[1] - from[1], from[1]),
                ]);
            }
        }
        if output.is_empty() {
            break;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_clips_a_triangle_to_its_inside() {
        let clipped = clip_rectangle(
            vec![[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert!((area(&clipped) - 1.0).abs() < 1e-12);
        assert!(
            clip_rectangle(vec![[2.0, 2.0], [3.0, 2.0], [2.0, 3.0]], [0.0; 2], [1.0; 2]).is_empty()
        );
    }
}
