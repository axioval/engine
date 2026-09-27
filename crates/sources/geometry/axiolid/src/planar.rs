//! Plan-projection helpers shared by the services in this crate.
//!
//! Every geometric question this adapter answers in plan uses the same
//! frame and the same projection, so they live in one place: two services
//! disagreeing about what 'in plan' means would be a silent measurement bug.

use axiolid_core::{Frame2, Point2, Vec2};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};

use crate::geometry::Triangle;

/// The xy plan frame both projections share.
pub(crate) fn plan_frame() -> Frame2 {
    Frame2 {
        origin: Point2::new(0.0, 0.0),
        x: Vec2::new(1.0, 0.0),
        y: Vec2::new(0.0, 1.0),
    }
}

/// Triangles projected to xy as counter-clockwise overlay polygons, dropping
/// degenerate ones.
///
/// A triangle seen edge-on in plan has no plan area and cannot contribute
/// coverage, so dropping it is a measurement decision, not a shortcut.
///
/// Every triangle is wound counter-clockwise. A closed solid's top and bottom
/// faces project onto the same area with opposite windings; under the
/// non-zero fill rule every service here uses, they would cancel and the solid
/// would have no footprint at all. Orienting them first makes the fill their
/// union, which is what a footprint is.
pub(crate) fn projected_polygons(triangles: &[Triangle]) -> Vec<Polygon> {
    triangles
        .iter()
        .filter_map(|[a, b, c]| {
            let mut ring = Ring {
                points: vec![
                    Point2::new(a.x, a.y),
                    Point2::new(b.x, b.y),
                    Point2::new(c.x, c.y),
                ],
            };
            let area = ring_area(&ring);
            if area < 0.0 {
                ring.points.reverse();
            }
            (area.abs() > f64::EPSILON).then_some(Polygon {
                outer: ring,
                holes: Vec::new(),
            })
        })
        .collect()
}

/// Absolute area of a polygon, holes subtracted.
///
/// Both boundaries are honoured because a result polygon MAY carry holes in
/// general. For the triangle-soup inputs this adapter builds, the overlay was
/// observed to return hole-free polygons, so the subtraction is defensive: it
/// keeps the function correct for any polygon rather than only for the shapes
/// this call site happens to produce today.
pub(crate) fn polygon_area(polygon: &Polygon) -> f64 {
    let outer = ring_area(&polygon.outer).abs();
    let holes: f64 = polygon.holes.iter().map(|r| ring_area(r).abs()).sum();
    (outer - holes).max(0.0)
}

/// Signed shoelace area of a ring.
pub(crate) fn ring_area(ring: &Ring) -> f64 {
    let points = &ring.points;
    let mut sum = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        sum += current.x * next.y - next.x * current.y;
    }
    sum * 0.5
}

/// The plan boundary of a triangle set, as its outer rings.
///
/// A mesh arrives as triangle soup: the shared edges between triangles are
/// interior, not boundary. Unioning first collapses them, leaving only the
/// real perimeter -- which is what an edge-based measurement must walk.
pub(crate) fn boundary_rings(
    triangles: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Option<Vec<Ring>> {
    let polygons = projected_polygons(triangles);
    if polygons.is_empty() {
        return None;
    }
    let input = OverlayInput {
        frame: plan_frame(),
        polygons,
    };
    let merged = overlay(
        &input,
        &input,
        OverlayOperation::Union,
        FillRule::NonZero,
        tolerance,
    )
    .ok()?;
    let rings: Vec<Ring> = merged.polygons.into_iter().map(|p| p.outer).collect();
    (!rings.is_empty()).then_some(rings)
}

/// Consecutive point pairs of a ring, closing back to the first point.
pub(crate) fn ring_segments(ring: &Ring) -> Vec<(Point2, Point2)> {
    let points = &ring.points;
    if points.len() < 2 {
        return Vec::new();
    }
    (0..points.len())
        .map(|i| (points[i], points[(i + 1) % points.len()]))
        .collect()
}

/// Perimeter length of a ring.
pub(crate) fn ring_perimeter(ring: &Ring) -> f64 {
    ring_segments(ring)
        .into_iter()
        .map(|(a, b)| ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt())
        .sum()
}

/// Area and perimeter of a triangle set's footprint, holes included.
///
/// `None` when the overlay cannot be computed; an empty footprint measures
/// zero with no perimeter.
pub(crate) fn footprint_measure(
    triangles: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Option<(f64, f64)> {
    let polygons = footprint_polygons(triangles, tolerance)?;
    let area = polygons.iter().map(polygon_area).sum();
    let perimeter = polygons
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .map(ring_perimeter)
        .sum();
    Some((area, perimeter))
}

/// A triangle set's footprint as the polygons of its plan union.
///
/// `None` when the overlay cannot be computed; an empty footprint has no
/// polygons.
pub(crate) fn footprint_polygons(
    triangles: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Option<Vec<Polygon>> {
    let input = OverlayInput {
        frame: plan_frame(),
        polygons: projected_polygons(triangles),
    };
    if input.polygons.is_empty() {
        return Some(Vec::new());
    }
    let merged = overlay(
        &input,
        &input,
        OverlayOperation::Union,
        FillRule::NonZero,
        tolerance,
    )
    .ok()?;
    Some(merged.polygons)
}

/// Area and first moments `(A, ∫x, ∫y)` of a polygon, holes subtracted,
/// whatever the orientation of its rings.
pub(crate) fn polygon_moments(polygon: &Polygon) -> (f64, f64, f64) {
    let ring = |ring: &Ring| {
        let points = &ring.points;
        let (mut area, mut x, mut y) = (0.0, 0.0, 0.0);
        for index in 0..points.len() {
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            let cross = current.x * next.y - next.x * current.y;
            area += cross;
            x += (current.x + next.x) * cross;
            y += (current.y + next.y) * cross;
        }
        // Shoelace sums are twice the area and six times the moments; the
        // sign of the area gives the ring's orientation.
        let sign = if area < 0.0 { -1.0 } else { 1.0 };
        (sign * area / 2.0, sign * x / 6.0, sign * y / 6.0)
    };
    let (mut area, mut x, mut y) = ring(&polygon.outer);
    for hole in &polygon.holes {
        let (a, hx, hy) = ring(hole);
        area -= a;
        x -= hx;
        y -= hy;
    }
    (area, x, y)
}

/// Area of the overlap of two triangle sets' footprints.
///
/// `None` when the overlay cannot be computed; an empty footprint overlaps
/// nothing and measures zero.
pub(crate) fn plan_overlap_area(
    first: &[Triangle],
    second: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Option<f64> {
    Some(
        plan_overlap_polygons(first, second, tolerance)?
            .iter()
            .map(polygon_area)
            .sum(),
    )
}

/// The overlap of two triangle sets' footprints, as polygons.
///
/// `None` when the overlay cannot be computed; an empty footprint overlaps
/// nothing.
pub(crate) fn plan_overlap_polygons(
    first: &[Triangle],
    second: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Option<Vec<Polygon>> {
    let first = OverlayInput {
        frame: plan_frame(),
        polygons: projected_polygons(first),
    };
    let second = OverlayInput {
        frame: plan_frame(),
        polygons: projected_polygons(second),
    };
    if first.polygons.is_empty() || second.polygons.is_empty() {
        return Some(Vec::new());
    }
    let result = overlay(
        &first,
        &second,
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .ok()?;
    Some(result.polygons)
}

#[cfg(test)]
mod polygon_area_tests {
    use super::{polygon_area, polygon_moments, ring_area};
    use axiolid_core::Point2;
    use axiolid_overlay::{Polygon, Ring};

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Ring {
        Ring {
            points: vec![
                Point2::new(x0, y0),
                Point2::new(x1, y0),
                Point2::new(x1, y1),
                Point2::new(x0, y1),
            ],
        }
    }

    #[test]
    fn a_hole_is_subtracted_from_the_area_it_removes() {
        let with_hole = Polygon {
            outer: rect(0.0, 0.0, 2.0, 2.0),
            holes: vec![rect(0.5, 0.5, 1.5, 1.5)],
        };
        // 4.0 outer minus a 1.0 void.
        assert!((polygon_area(&with_hole) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn moments_subtract_holes_whatever_the_orientation() {
        let mut hole = rect(0.0, 0.0, 1.0, 1.0);
        hole.points.reverse();
        let polygon = Polygon {
            outer: rect(0.0, 0.0, 4.0, 2.0),
            holes: vec![hole],
        };
        let (area, x, y) = polygon_moments(&polygon);
        // 8 m² centred at (2, 1) less 1 m² centred at (0.5, 0.5).
        assert!((area - 7.0).abs() < 1e-9);
        assert!((x - (16.0 - 0.5)).abs() < 1e-9);
        assert!((y - (8.0 - 0.5)).abs() < 1e-9);
    }

    #[test]
    fn ring_orientation_does_not_change_the_area() {
        let mut reversed = rect(0.0, 0.0, 2.0, 2.0);
        reversed.points.reverse();
        assert!(ring_area(&reversed) < 0.0, "reversed ring is negative");
        let polygon = Polygon {
            outer: reversed,
            holes: Vec::new(),
        };
        assert!((polygon_area(&polygon) - 4.0).abs() < 1e-9);
    }
}
