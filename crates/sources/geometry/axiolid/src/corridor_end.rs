//! Corridor ends of a space's footprint, from the region skeleton of
//! `axiolid-route`, and how subjects sit against the wall each end runs into.
//!
//! ADR 0004: this module measures; whether a window may sit in a corridor's
//! end wall is a rule's decision.
//!
//! The skeleton approximates the footprint's medial axis from boundary
//! samples at most `spacing` apart, pruned of the spurs into corners
//! (`PRUNE`, which drops spurs into right-angled and sharper corners). Its
//! nodes lie inside the footprint and their clearances are certified; where
//! the axis lies, and so where a path ends and which way it runs, is not.
//! The kernel asks for a spacing below an eighth of the narrowest width, so
//! the spacing starts at a tenth of the footprint's narrower bounding side
//! and is refined to a tenth of the narrowest width the skeleton finds
//! until it lies below an eighth of it.
//!
//! An end's wall is the straight run of the boundary the path, continued
//! straight on, runs into. It is decided only when the approximation cannot
//! change it: the path must stop at the wall (the end lies no farther from it
//! than its clearance and two sample spacings) and every ray from the end
//! within `SPREAD` of the path's direction (taken one clearance back along
//! it) must first meet the same run. Otherwise the wall is undecided, never
//! guessed.
//!
//! Only an exact footprint has walls: a tessellated one's edges are chords
//! of a curved wall, so a tessellated space refuses. A subject is measured
//! on its own footprint: its plan gap to the wall segment and the length of
//! the segment it faces (its projection onto the wall's line, clipped to
//! the segment), exact for a planar mesh; a tessellated subject widens the
//! gap by its chord deviation `d` and the facing length by `2d`, since each
//! end of its projection moves by at most `d`.

use axiolid_core::Point2;
use axiolid_overlay::{Polygon, Ring};
use axiolid_route::{NodeKind, Skeleton, skeleton};
use axioval_engine::{
    CorridorEnd, CorridorEndRequest, CorridorEnds, EndWall, PlanLength, PlanSpanError, WallContact,
};
use axioval_ir::{Evidence, ObjectId};

use crate::plan_area::{Footprint, tolerance};
use crate::planar::{footprint_polygons, ring_segments};

/// The skeleton's prune factor: keeps corridors, junctions and dead ends,
/// drops the spurs into right-angled and sharper corners.
pub(crate) const PRUNE: f64 = 1.5;

/// How far either side of a path's direction a ray from its end must still
/// meet the same wall (20°): the direction taken one clearance back along
/// an approximate axis is off by less.
const SPREAD: f64 = 0.35;

/// A point in plan.
type Point = (f64, f64);

/// Most boundary samples a skeleton may take.
const MOST_SAMPLES: f64 = 50_000.0;

/// How many times the spacing is refined before refusing.
const REFINEMENTS: usize = 4;

/// Relative distance within which a vertex counts as lying on the line
/// through its neighbours; well below the overlay's own grid snapping.
const COLLINEAR: f64 = 1e-9;

/// The corridor ends of `space`'s footprint, with every subject measured
/// against each decided end wall.
pub(crate) fn corridor_ends(
    request: &CorridorEndRequest,
    space: &Footprint,
    subjects: &[(ObjectId, Footprint)],
) -> Result<CorridorEnds, PlanSpanError> {
    let named = request.space();
    if space.deviation > 0.0 {
        return Err(PlanSpanError::Unavailable(format!(
            "{named} is tessellated: its footprint's edges are chords of a curved wall, \
             so no end wall can be named"
        )));
    }
    let region: Vec<Polygon> = polygons(named, space)?
        .iter()
        .map(simplified)
        .filter(|polygon| polygon.outer.points.len() >= 3)
        .collect();
    if region.is_empty() {
        return Err(PlanSpanError::Unavailable(format!(
            "{named} has no footprint, so it has no corridor"
        )));
    }
    let skeleton = refined(named, &region)?;
    let walls = walls(&region);
    let mut measured_subjects = Vec::with_capacity(subjects.len());
    for (subject, footprint) in subjects {
        measured_subjects.push((subject, footprint.deviation, polygons(subject, footprint)?));
    }
    let mut ends = Vec::new();
    for index in skeleton.ends() {
        let node = &skeleton.nodes[index];
        let point = (node.point.x, node.point.y);
        let wall = match end_wall(&skeleton, index, &walls) {
            Err(why) => EndWall::Undecided(why),
            Ok((start, end)) => {
                let mut contacts = Vec::with_capacity(measured_subjects.len());
                for (subject, deviation, footprint) in &measured_subjects {
                    contacts.push(contact(subject, *deviation, footprint, named, start, end)?);
                }
                EndWall::Decided {
                    start: [start.0, start.1],
                    end: [end.0, end.1],
                    contacts,
                }
            }
        };
        ends.push(CorridorEnd::try_new(
            [point.0, point.1],
            node.clearance,
            wall,
        )?);
    }
    ends.sort_by(|a, b| {
        let (a, b) = (a.point(), b.point());
        a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
    });
    let locator = format!(
        "corridor-ends:{named}:spacing={:.6}:prune={PRUNE}:ends={}",
        skeleton.spacing,
        ends.len()
    );
    CorridorEnds::try_new(
        named.clone(),
        ends,
        Evidence {
            source: named.source.clone(),
            locator,
            exact: false,
        },
    )
}

fn polygons(object: &ObjectId, footprint: &Footprint) -> Result<Vec<Polygon>, PlanSpanError> {
    let tolerance =
        tolerance().map_err(|_| PlanSpanError::Unavailable("invalid overlay tolerance".into()))?;
    let polygons = footprint_polygons(&footprint.soup, tolerance).ok_or_else(|| {
        PlanSpanError::Unavailable(format!("the footprint of {object} cannot be computed"))
    })?;
    if polygons.is_empty() {
        return Err(PlanSpanError::Unavailable(format!(
            "{object} has no footprint (no body)"
        )));
    }
    Ok(polygons)
}

/// The polygon with every vertex on the line through its neighbours
/// dropped, so a straight wall is one edge.
fn simplified(polygon: &Polygon) -> Polygon {
    Polygon {
        outer: simplified_ring(&polygon.outer),
        holes: polygon
            .holes
            .iter()
            .map(simplified_ring)
            .filter(|ring| ring.points.len() >= 3)
            .collect(),
    }
}

fn simplified_ring(ring: &Ring) -> Ring {
    let mut points: Vec<Point2> = ring.points.clone();
    points.dedup();
    while points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    loop {
        let count = points.len();
        if count < 3 {
            break;
        }
        let redundant = (0..count).find(|&i| {
            let (a, p, b) = (
                points[(i + count - 1) % count],
                points[i],
                points[(i + 1) % count],
            );
            let (ex, ey) = (b.x - a.x, b.y - a.y);
            let length = ex.hypot(ey);
            let (wx, wy) = (p.x - a.x, p.y - a.y);
            // On the line, and between its neighbours.
            length > 0.0
                && (ex * wy - ey * wx).abs() <= COLLINEAR * length * length.max(1.0)
                && ex * wx + ey * wy > 0.0
                && ex * wx + ey * wy < length * length
        });
        match redundant {
            Some(i) => {
                points.remove(i);
            }
            None => break,
        }
    }
    Ring { points }
}

/// A skeleton whose spacing lies below an eighth of the narrowest width it
/// finds.
fn refined(space: &ObjectId, region: &[Polygon]) -> Result<Skeleton, PlanSpanError> {
    let (mut low, mut high) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    let mut perimeter = 0.0;
    for polygon in region {
        for ring in std::iter::once(&polygon.outer).chain(&polygon.holes) {
            for (a, b) in ring_segments(ring) {
                low = (low.0.min(a.x), low.1.min(a.y));
                high = (high.0.max(a.x), high.1.max(a.y));
                perimeter += (b.x - a.x).hypot(b.y - a.y);
            }
        }
    }
    let mut spacing = (high.0 - low.0).min(high.1 - low.1) / 10.0;
    for _ in 0..REFINEMENTS {
        if !(spacing.is_finite() && spacing > 0.0) || perimeter / spacing > MOST_SAMPLES {
            break;
        }
        let built = skeleton(region, spacing, PRUNE).map_err(|error| {
            PlanSpanError::Unavailable(format!(
                "the skeleton of {space} cannot be built: {error:?}"
            ))
        })?;
        let narrowest = 2.0
            * built
                .nodes
                .iter()
                .filter(|node| node.kind != NodeKind::Isolated)
                .map(|node| node.clearance.0)
                .fold(f64::INFINITY, f64::min);
        if !narrowest.is_finite() {
            // No path at all: a footprint too small or round to hold one.
            return Ok(built);
        }
        if spacing < narrowest / 8.0 {
            return Ok(built);
        }
        spacing = narrowest / 10.0;
    }
    Err(PlanSpanError::Unavailable(format!(
        "{space}'s footprint is too finely detailed for its skeleton to be sampled \
         below an eighth of its narrowest width"
    )))
}

/// A wall: the index of its run, and its ends.
type WallSegment = (usize, Point, Point);

/// Every boundary edge, indexed as the skeleton names walls.
fn walls(region: &[Polygon]) -> Vec<(axiolid_route::Wall, WallSegment)> {
    let mut walls = Vec::new();
    for (polygon_index, polygon) in region.iter().enumerate() {
        for (ring_index, ring) in std::iter::once(&polygon.outer)
            .chain(&polygon.holes)
            .enumerate()
        {
            for (edge_index, (a, b)) in ring_segments(ring).into_iter().enumerate() {
                let key = walls.len();
                walls.push((
                    axiolid_route::Wall {
                        polygon: polygon_index,
                        ring: ring_index,
                        edge: edge_index,
                    },
                    (key, (a.x, a.y), (b.x, b.y)),
                ));
            }
        }
    }
    walls
}

/// The segment of the wall the path ending at `index` runs into, or why it
/// is undecided.
fn end_wall(
    skeleton: &Skeleton,
    index: usize,
    walls: &[(axiolid_route::Wall, WallSegment)],
) -> Result<(Point, Point), String> {
    let node = &skeleton.nodes[index];
    let here = (node.point.x, node.point.y);
    let ahead = node
        .ahead
        .ok_or_else(|| "the path's direction into its end is undecided".to_owned())?;
    let &(_, (key, start, end)) = walls
        .iter()
        .find(|(wall, _)| *wall == ahead)
        .ok_or_else(|| "the skeleton names a wall the footprint does not have".to_owned())?;
    let short = segment_distance(here, start, end) - node.clearance.1;
    if short > 2.0 * skeleton.spacing {
        return Err(format!(
            "the path ends {short:.3} m short of the wall ahead, so it does not stop at a wall"
        ));
    }
    let from = behind(skeleton, index)
        .ok_or_else(|| "the path is too short to give its direction".to_owned())?;
    let direction = (here.0 - from.0, here.1 - from.1);
    for angle in [-SPREAD, 0.0, SPREAD] {
        let (sin, cos) = f64::sin_cos(angle);
        let ray = (
            direction.0 * cos - direction.1 * sin,
            direction.0 * sin + direction.1 * cos,
        );
        if first_hit(here, ray, walls) != Some(key) {
            return Err(format!(
                "a path direction within {:.0}° of the skeleton's meets another wall first",
                SPREAD.to_degrees()
            ));
        }
    }
    Ok((start, end))
}

/// The first node at least one clearance back along the path from the end
/// at `index`.
fn behind(skeleton: &Skeleton, index: usize) -> Option<(f64, f64)> {
    let node = &skeleton.nodes[index];
    let here = (node.point.x, node.point.y);
    let reach = node.clearance.0.max(skeleton.spacing);
    let (mut previous, mut at) = (usize::MAX, index);
    for _ in 0..skeleton.nodes.len() {
        let next = skeleton.edges.iter().find_map(|&(a, b)| {
            let other = if a == at {
                b
            } else if b == at {
                a
            } else {
                return None;
            };
            (other != previous).then_some(other)
        })?;
        let point = skeleton.nodes[next].point;
        if distance(here, (point.x, point.y)) >= reach {
            return Some((point.x, point.y));
        }
        previous = at;
        at = next;
    }
    None
}

/// The run first met by the ray from `origin` along `direction`.
fn first_hit(
    origin: (f64, f64),
    direction: (f64, f64),
    walls: &[(axiolid_route::Wall, WallSegment)],
) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for &(_, (key, a, b)) in walls {
        let edge = (b.0 - a.0, b.1 - a.1);
        let denominator = cross(direction, edge);
        if denominator == 0.0 {
            continue;
        }
        let offset = (a.0 - origin.0, a.1 - origin.1);
        let along = cross(offset, edge) / denominator;
        let on_edge = cross(offset, direction) / denominator;
        if along > 0.0 && (0.0..=1.0).contains(&on_edge) && best.is_none_or(|(t, _)| along < t) {
            best = Some((along, key));
        }
    }
    best.map(|(_, key)| key)
}

/// How `subject`'s footprint sits against the wall from `start` to `end`.
fn contact(
    subject: &ObjectId,
    deviation: f64,
    footprint: &[Polygon],
    space: &ObjectId,
    start: (f64, f64),
    end: (f64, f64),
) -> Result<WallContact, PlanSpanError> {
    let segments: Vec<(Point, Point)> = footprint
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .flat_map(ring_segments)
        .map(|(a, b)| ((a.x, a.y), (b.x, b.y)))
        .collect();
    let gap = if segments
        .iter()
        .any(|&(a, b)| segments_meet(a, b, start, end))
        || inside(footprint, start)
    {
        0.0
    } else {
        segments
            .iter()
            .map(|&(a, b)| segments_distance(a, b, start, end))
            .fold(f64::INFINITY, f64::min)
    };
    let length = distance(start, end);
    let unit = ((end.0 - start.0) / length, (end.1 - start.1) / length);
    let (first, last) = segments
        .iter()
        .map(|&(a, _)| (a.0 - start.0) * unit.0 + (a.1 - start.1) * unit.1)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), along| {
            (low.min(along), high.max(along))
        });
    let facing = (last.min(length) - first.max(0.0)).max(0.0);
    let wall = format!(
        "({:.6},{:.6})-({:.6},{:.6})",
        start.0, start.1, end.0, end.1
    );
    let measured = |value: f64, slack: f64, cap: f64, what: &str| {
        let locator = format!("corridor-end-{what}:{space}:{subject}:wall={wall}");
        if slack == 0.0 {
            return PlanLength::try_new(
                value,
                value,
                Evidence::exact(subject.source.clone(), locator),
            );
        }
        PlanLength::try_new(
            (value - slack).max(0.0),
            (value + slack).min(cap),
            Evidence {
                source: subject.source.clone(),
                locator,
                exact: false,
            },
        )
    };
    Ok(WallContact::new(
        subject.clone(),
        measured(gap, deviation, f64::INFINITY, "gap")?,
        measured(facing, 2.0 * deviation, length, "facing")?,
    ))
}

/// Whether a point lies in the polygons (crossing count; a point on the
/// boundary also meets a boundary segment, which the caller tests first).
fn inside(polygons: &[Polygon], (x, y): (f64, f64)) -> bool {
    let encloses = |ring: &Ring| {
        let mut inside = false;
        for (a, b) in ring_segments(ring) {
            if (a.y > y) != (b.y > y) && x < a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y) {
                inside = !inside;
            }
        }
        inside
    };
    polygons
        .iter()
        .any(|polygon| encloses(&polygon.outer) && !polygon.holes.iter().any(encloses))
}

fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The distance from `point` to the segment from `a` to `b`.
fn segment_distance(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(point, (a.0 + t * dx, a.1 + t * dy))
}

/// Whether two closed segments share a point.
fn segments_meet(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        cross((q.0 - p.0, q.1 - p.1), (r.0 - p.0, r.1 - p.1))
    };
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    segment_distance(a, c, d) == 0.0
        || segment_distance(b, c, d) == 0.0
        || segment_distance(c, a, b) == 0.0
        || segment_distance(d, a, b) == 0.0
}

/// The distance between two segments that do not meet.
fn segments_distance(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> f64 {
    segment_distance(a, c, d)
        .min(segment_distance(b, c, d))
        .min(segment_distance(c, a, b))
        .min(segment_distance(d, a, b))
}

#[cfg(test)]
mod tests {
    use axiolid_core::Point2;
    use axiolid_overlay::{Polygon, Ring};

    use super::{first_hit, simplified_ring, walls};

    fn ring(points: &[(f64, f64)]) -> Ring {
        Ring {
            points: points.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        }
    }

    #[test]
    fn collinear_vertices_are_dropped_so_a_wall_is_one_edge() {
        let simplified = simplified_ring(&ring(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (10.0, 0.0),
            (10.0, 2.0),
            (0.0, 2.0),
            (0.0, 1.0),
        ]));
        assert_eq!(
            simplified.points,
            ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 2.0), (0.0, 2.0)]).points
        );
    }

    #[test]
    fn a_ray_meets_the_nearest_wall_it_crosses() {
        let region = [Polygon {
            outer: ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 2.0), (0.0, 2.0)]),
            holes: Vec::new(),
        }];
        let walls = walls(&region);
        // Leftwards from (1, 1): the west wall, edge 3.
        assert_eq!(first_hit((1.0, 1.0), (-1.0, 0.0), &walls), Some(3));
        assert_eq!(first_hit((1.0, 1.0), (1.0, 0.0), &walls), Some(1));
        assert_eq!(first_hit((1.0, 1.0), (0.0, 1.0), &walls), Some(2));
    }
}
