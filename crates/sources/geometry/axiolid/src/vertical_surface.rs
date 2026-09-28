//! Vertical distances between chosen surfaces.
//!
//! A subject's surface is a level: its top or its bottom. A counterpart's
//! top or bottom is a level too, and the distance is the difference of the
//! two levels in the requested direction, for bodies whose footprints are
//! related as for the extent gap.
//!
//! The counterpart's nearest surface is found over the subject's footprint:
//! each counterpart triangle is clipped in plan to each projected subject
//! triangle, a piece with positive plan area lies directly over or under the
//! footprint, and the nearest of its points to the subject's level in the
//! direction is at a vertex of the piece (the piece is planar and convex, so
//! height is linear over it). A sloped slab's underside is measured where it
//! runs over the subject, not at its lowest point elsewhere.
//!
//! A tessellation moves every surface by up to the combined chord deviation
//! `d`. The lower bound then comes from pieces over the footprint grown by
//! `d` (circumscribed), cut at the level less `d`, less `d`; the upper bound
//! only from a witness: a piece's plan centroid deeper than `d` inside the
//! measured footprint, surely on the direction's side, plus `d`. Without a
//! witness the upper bound is unbounded.

use axiolid_core::Point3;
use axioval_engine::{
    CounterpartSurface, GeometryFidelity, ProximityError, SubjectSurface, VerticalDirection,
};

use crate::geometry::Triangle;
use crate::planar::{Disc, boundary_rings, grown_polygons, projected_polygons, ring_segments};
use crate::proximity::{Body, Relation, relation, tolerance};

/// Plan area below which a clipped piece only touches the footprint.
const TOUCH_AREA: f64 = 1e-12;

/// The two bodies and their fidelities.
pub(crate) struct Pair<'p, 'a> {
    pub(crate) subject: &'p Body<'a>,
    pub(crate) counterpart: &'p Body<'a>,
    pub(crate) subject_fidelity: GeometryFidelity,
    pub(crate) counterpart_fidelity: GeometryFidelity,
}

/// The distance from `from` of the subject to `to` of the counterpart in
/// `direction`, as `(lower, upper)`; infinite when unrelated.
pub(crate) fn interval(
    pair: &Pair<'_, '_>,
    offset: f64,
    direction: VerticalDirection,
    from: SubjectSurface,
    to: CounterpartSurface,
) -> Result<(f64, f64), ProximityError> {
    let deviation = pair
        .subject_fidelity
        .combined(pair.counterpart_fidelity)
        .deviation_metres();
    let bounds = pair.subject.soup.bounds;
    let level = match from {
        SubjectSurface::Top => bounds.max()[2],
        SubjectSurface::Bottom => bounds.min()[2],
    };
    let sides: &[VerticalDirection] = match direction {
        VerticalDirection::Either => &[VerticalDirection::Above, VerticalDirection::Below],
        VerticalDirection::Above => &[VerticalDirection::Above],
        VerticalDirection::Below => &[VerticalDirection::Below],
    };
    let (mut lower, mut upper) = (f64::INFINITY, f64::INFINITY);
    for &side in sides {
        let (low, high) = match to {
            CounterpartSurface::Nearest => {
                if offset > 0.0 {
                    return Err(ProximityError::UnsupportedProjection);
                }
                nearest(pair, level, side, deviation)?
            }
            CounterpartSurface::Top | CounterpartSurface::Bottom => {
                let counterpart = pair.counterpart.soup.bounds;
                let other = if to == CounterpartSurface::Top {
                    counterpart.max()[2]
                } else {
                    counterpart.min()[2]
                };
                let gap = if side == VerticalDirection::Above {
                    other - level
                } else {
                    level - other
                };
                if gap + deviation < 0.0 {
                    (f64::INFINITY, f64::INFINITY)
                } else if gap - deviation >= 0.0 {
                    ((gap - deviation).max(0.0), gap + deviation)
                } else {
                    (0.0, f64::INFINITY)
                }
            }
        };
        lower = lower.min(low);
        upper = upper.min(high);
    }
    if to == CounterpartSurface::Nearest || lower.is_infinite() {
        return Ok((lower, upper));
    }
    Ok(
        match relation(
            pair.subject,
            pair.counterpart,
            offset,
            pair.subject_fidelity,
            pair.counterpart_fidelity,
        )? {
            Relation::Unrelated => (f64::INFINITY, f64::INFINITY),
            Relation::Related => (lower, upper),
            Relation::Open => (lower, f64::INFINITY),
        },
    )
}

/// The part of a convex polygon where `side` is not negative.
fn clip(polygon: &[Point3], side: impl Fn(&Point3) -> f64) -> Vec<Point3> {
    let mut kept = Vec::with_capacity(polygon.len() + 1);
    for (index, a) in polygon.iter().enumerate() {
        let b = &polygon[(index + 1) % polygon.len()];
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            kept.push(*a);
        }
        if (sa < 0.0) != (sb < 0.0) {
            let t = sa / (sa - sb);
            kept.push(*a + (*b - *a) * t);
        }
    }
    kept
}

/// A convex plan polygon, counter-clockwise, as `(x, y)` points.
type Plan = Vec<(f64, f64)>;

/// The part of a triangle over a convex counter-clockwise plan polygon.
fn over(triangle: &Triangle, region: &Plan) -> Vec<Point3> {
    let mut piece = triangle.to_vec();
    for index in 0..region.len() {
        if piece.is_empty() {
            break;
        }
        let (px, py) = region[index];
        let (qx, qy) = region[(index + 1) % region.len()];
        piece = clip(&piece, |point| {
            (qx - px).mul_add(point.y - py, -((qy - py) * (point.x - px)))
        });
    }
    piece
}

fn plan_area(piece: &[Point3]) -> f64 {
    let mut sum = 0.0;
    for (index, a) in piece.iter().enumerate() {
        let b = piece[(index + 1) % piece.len()];
        sum += a.x.mul_add(b.y, -(b.x * a.y));
    }
    (sum / 2.0).abs()
}

fn box_of(points: &[(f64, f64)]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[x0, y0, x1, y1], &(x, y)| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
    )
}

fn triangle_box(triangle: &Triangle) -> [f64; 4] {
    box_of(&triangle.map(|point| (point.x, point.y)))
}

fn apart(a: [f64; 4], b: [f64; 4]) -> bool {
    a[2] < b[0] || b[2] < a[0] || a[3] < b[1] || b[3] < a[1]
}

/// The plan regions, as convex counter-clockwise polygons.
fn regions(polygons: Vec<axiolid_overlay::Polygon>) -> Vec<(Plan, [f64; 4])> {
    polygons
        .into_iter()
        .map(|polygon| {
            let points: Plan = polygon
                .outer
                .points
                .iter()
                .map(|point| (point.x, point.y))
                .collect();
            let bounds = box_of(&points);
            (points, bounds)
        })
        .collect()
}

/// The pieces of the counterpart's surface over the regions.
fn pieces(counterpart: &Body<'_>, regions: &[(Plan, [f64; 4])]) -> Vec<(Vec<Point3>, Triangle)> {
    let mut found = Vec::new();
    for triangle in &counterpart.soup.items {
        let bounds = triangle_box(triangle);
        for (region, region_bounds) in regions {
            if apart(bounds, *region_bounds) {
                continue;
            }
            let piece = over(triangle, region);
            if piece.len() >= 3 {
                found.push((piece, *triangle));
            }
        }
    }
    found
}

/// How far `z` lies from `level` in `side`, negative on the other side.
fn past(z: f64, level: f64, side: VerticalDirection) -> f64 {
    if side == VerticalDirection::Below {
        level - z
    } else {
        z - level
    }
}

/// The nearest counterpart surface over the subject's footprint on `side`.
fn nearest(
    pair: &Pair<'_, '_>,
    level: f64,
    side: VerticalDirection,
    deviation: f64,
) -> Result<(f64, f64), ProximityError> {
    let subject = &pair.subject.soup.items;
    let footprint = regions(projected_polygons(subject));
    // The least distance over pieces cut to the side, the level moved back
    // by `slack`.
    let least = |pieces: &[(Vec<Point3>, Triangle)], slack: f64, touching: bool| {
        pieces
            .iter()
            .filter(|(piece, _)| touching || plan_area(piece) > TOUCH_AREA)
            .flat_map(|(piece, _)| clip(piece, |point| past(point.z, level, side) + slack))
            .map(|point| past(point.z, level, side).max(0.0))
            .fold(f64::INFINITY, f64::min)
    };
    if deviation == 0.0 {
        let distance = least(&pieces(pair.counterpart, &footprint), 0.0, false);
        return Ok((distance, distance));
    }
    let grown = regions(grown_polygons(subject, deviation, Disc::Circumscribed));
    let lower = (least(&pieces(pair.counterpart, &grown), deviation, true) - deviation).max(0.0);
    if lower.is_infinite() {
        return Ok((f64::INFINITY, f64::INFINITY));
    }
    // A witness: a piece's plan centroid deeper than the deviation inside
    // the footprint, surely on the side.
    let outline: Vec<_> = boundary_rings(subject, tolerance()?)
        .unwrap_or_default()
        .iter()
        .flat_map(ring_segments)
        .collect();
    let depth = |x: f64, y: f64| {
        outline
            .iter()
            .map(|(a, b)| {
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let length = dx.mul_add(dx, dy * dy);
                let t = if length > 0.0 {
                    ((x - a.x).mul_add(dx, (y - a.y) * dy) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (x - dx.mul_add(t, a.x)).hypot(y - dy.mul_add(t, a.y))
            })
            .fold(f64::INFINITY, f64::min)
    };
    let mut upper = f64::INFINITY;
    for (piece, triangle) in pieces(pair.counterpart, &footprint) {
        if plan_area(&piece) <= TOUCH_AREA {
            continue;
        }
        let count = f64::from(u32::try_from(piece.len()).unwrap_or(u32::MAX));
        let (x, y) = piece
            .iter()
            .fold((0.0, 0.0), |(x, y), point| (x + point.x, y + point.y));
        let (x, y) = (x / count, y / count);
        if outline.is_empty() || depth(x, y) <= deviation {
            continue;
        }
        let Some(z) = height_at(&triangle, x, y) else {
            continue;
        };
        let distance = past(z, level, side);
        if distance >= deviation {
            upper = upper.min(distance + deviation);
        }
    }
    Ok((lower, upper.max(lower)))
}

/// The height of a triangle's plane over `(x, y)`; `None` when it stands
/// edge-on in plan.
fn height_at(triangle: &Triangle, px: f64, py: f64) -> Option<f64> {
    let [first, second, third] = triangle;
    let normal = (*second - *first).cross(*third - *first);
    if normal.z.abs() <= f64::EPSILON {
        return None;
    }
    Some(first.z - normal.x.mul_add(px - first.x, normal.y * (py - first.y)) / normal.z)
}
