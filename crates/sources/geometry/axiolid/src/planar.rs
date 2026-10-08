//! Plan-projection helpers shared by the services in this crate.
//!
//! Every geometric question this adapter answers in plan uses the same
//! frame and the same projection, so they live in one place: two services
//! disagreeing about what 'in plan' means would be a silent measurement bug.

use axiolid_core::{Frame2, Point2, Vec2};
use axiolid_overlay::{
    FillRule, OverlayError, OverlayInput, OverlayOperation, Polygon, Ring, overlay,
};

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
            let area = triangle_area([ring.points[0], ring.points[1], ring.points[2]]);
            if area < 0.0 {
                ring.points.reverse();
            }
            (area.abs() > f64::EPSILON && !collinear(&ring.points)).then_some(Polygon {
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

/// Whether a ring's vertices lie on one line, up to the rounding of their
/// coordinates: a shadow with no plan area, such as a vertical face's.
///
/// Such rings are left out before any union or region is built from plan
/// shadows. The overlay leaves exactly collinear rings out of its booleans
/// since axiolid-overlay 0.3.9 (axiolid/kernel#219, where they emptied whole
/// unions), but rounded coordinates rarely stay exactly collinear, and a
/// shadow without area covers nothing whatever the kernel does with it.
pub(crate) fn collinear(points: &[Point2]) -> bool {
    let Some((&first, rest)) = points.split_first() else {
        return true;
    };
    // The farthest vertex from the first fixes the line, so every other
    // vertex is tested against a well-conditioned direction.
    let Some(&far) = rest
        .iter()
        .max_by(|a, b| (**a - first).length().total_cmp(&(**b - first).length()))
    else {
        return true;
    };
    let along = far - first;
    let length = along.length();
    if length == 0.0 {
        return true;
    }
    // A coordinate of magnitude m is rounded by up to half an ulp of m, so
    // a vertex within a few ulps of the line is on it.
    let magnitude = points
        .iter()
        .flat_map(|point| [point.x.abs(), point.y.abs()])
        .fold(0.0, f64::max);
    let reach = 8.0 * f64::EPSILON * magnitude;
    rest.iter()
        .all(|point| along.perp_dot(*point - first).abs() <= reach * length)
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

/// Signed area of a plan triangle, taken about its first vertex.
///
/// [`ring_area`] sums the shoelace over the coordinates themselves, which
/// cancels products of the coordinates' size: at georeferenced coordinates
/// (some 10⁶ m) each term is about 10¹² m², whose rounding exceeds the area
/// of a thin triangle and can flip its sign. That wound such a triangle
/// clockwise in [`projected_polygons`], and the overlay refused the
/// footprint as self-intersecting. Edge vectors from the first vertex are
/// exact up to their own rounding, so this is accurate to the triangle's
/// size, not to where it lies. The overlay still takes a ring's orientation
/// and its `ZeroArea` check from the coordinates (axiolid/kernel#274), so
/// such a triangle may yet be refused there.
fn triangle_area(points: [Point2; 3]) -> f64 {
    let [a, b, c] = points;
    0.5 * (b - a).perp_dot(c - a)
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

/// `a + b` and the exact error of its rounding.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let back = sum - a;
    (sum, (a - (sum - back)) + (b - back))
}

/// The total boundary length of `polygons`, holes included, and a bound on
/// its rounding: zero when every segment runs along an axis and every
/// difference and sum is exact, so an axis-aligned footprint measures
/// exactly.
pub(crate) fn certified_perimeter(polygons: &[Polygon]) -> (f64, f64) {
    let mut total = 0.0_f64;
    let mut bound = 0.0_f64;
    for ring in polygons
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
    {
        for (a, b) in ring_segments(ring) {
            let (dx, ex) = two_sum(b.x, -a.x);
            let (dy, ey) = two_sum(b.y, -a.y);
            let length = if dx == 0.0 || dy == 0.0 {
                dx.abs() + dy.abs()
            } else {
                dx.hypot(dy)
            };
            // A rounded difference moves the length by at most its error; a
            // hypotenuse rounds within an ulp or two of its length.
            bound += ex.abs() + ey.abs();
            if dx != 0.0 && dy != 0.0 {
                bound += 2.0 * f64::EPSILON * length;
            }
            let (sum, error) = two_sum(total, length);
            total = sum;
            bound += error.abs();
        }
    }
    (total, bound)
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
        overlap_of(
            projected_polygons(first),
            projected_polygons(second),
            tolerance,
        )
        .ok()?
        .iter()
        .map(polygon_area)
        .sum(),
    )
}

/// The overlay's intersection of two shadow sets under the non-zero rule;
/// an empty set overlaps nothing.
fn overlap_of(
    first: Vec<Polygon>,
    second: Vec<Polygon>,
    tolerance: axiolid_core::Tolerance,
) -> Result<Vec<Polygon>, OverlayError> {
    if first.is_empty() || second.is_empty() {
        return Ok(Vec::new());
    }
    let first = OverlayInput {
        frame: plan_frame(),
        polygons: first,
    };
    let second = OverlayInput {
        frame: plan_frame(),
        polygons: second,
    };
    Ok(overlay(
        &first,
        &second,
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )?
    .polygons)
}

/// The overlap of two triangle sets' footprints, measured without the
/// shadows the overlay refuses as degenerate at `tolerance`.
///
/// The overlay refuses a whole input over one ring with two corners within
/// its linear tolerance or an area within its square (`RepeatedVertex`,
/// `ZeroArea`); the shadow of a near-vertical face of an ordinary wall is
/// such a sliver, two of its corners a rounding apart. Shadows with no area
/// within rounding are already gone (`projected_polygons`); a sliver
/// that still has area is left out of the overlay and its area kept in
/// [`BoundedOverlap::slivers`], so the true overlap lies between
/// `polygons` and `polygons` plus `slivers`. Only slivers whose box meets
/// the other set's box are counted: one apart from it cannot overlap it.
pub(crate) fn bounded_plan_overlap(
    first: &[Triangle],
    second: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<BoundedOverlap, OverlayError> {
    let first = Shadows::of(first, tolerance);
    let second = Shadows::of(second, tolerance);
    let slivers = first.slivers_meeting(second.bounds()) + second.slivers_meeting(first.bounds());
    Ok(BoundedOverlap {
        polygons: overlap_of(first.kept, second.kept, tolerance)?,
        slivers,
    })
}

/// A plan measurement taken without the slivers the overlay refuses.
#[derive(Debug)]
pub(crate) struct BoundedOverlap {
    /// The measured region over the shadows the overlay accepts. Leaving
    /// shadows out only shrinks a union, so it lies inside the true one.
    pub(crate) polygons: Vec<Polygon>,
    /// An upper bound on the area of the shadows left out that could add
    /// to it, in square metres: the true region exceeds `polygons` by at
    /// most this much. Zero when nothing was left out.
    pub(crate) slivers: f64,
}

/// A triangle set's footprint, measured without the shadows the overlay
/// refuses as degenerate at `tolerance` (see [`bounded_plan_overlap`]):
/// the true footprint's area lies between the `polygons`' and theirs plus
/// `slivers`.
pub(crate) fn bounded_footprint(
    triangles: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<BoundedOverlap, OverlayError> {
    let shadows = Shadows::of(triangles, tolerance);
    let slivers = shadows.slivers_meeting(shadows.bounds());
    if shadows.kept.is_empty() {
        return Ok(BoundedOverlap {
            polygons: Vec::new(),
            slivers,
        });
    }
    let input = OverlayInput {
        frame: plan_frame(),
        polygons: shadows.kept,
    };
    Ok(BoundedOverlap {
        polygons: overlay(
            &input,
            &input,
            OverlayOperation::Union,
            FillRule::NonZero,
            tolerance,
        )?
        .polygons,
        slivers,
    })
}

/// How far, relative to the extent, the overlay's grid may move a point it
/// snaps (axiolid/kernel#173 measures ~1.5e-8), with room to spare.
pub(crate) const OVERLAY_SNAP: f64 = 1e-7;

/// The area by which the overlay's grid snapping may already move a
/// measurement over `polygons` (axiolid/kernel#173): every boundary point
/// moved by [`OVERLAY_SNAP`] of the largest coordinate (at least a metre),
/// swept along the boundary. An area the overlay reports carries this much
/// whatever else is left out of it.
pub(crate) fn snapping_area(polygons: &[Polygon]) -> f64 {
    let rings = || {
        polygons
            .iter()
            .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
    };
    let magnitude = rings()
        .flat_map(|ring| &ring.points)
        .flat_map(|point| [point.x.abs(), point.y.abs()])
        .fold(1.0, f64::max);
    let perimeter: f64 = rings().map(ring_perimeter).sum();
    OVERLAY_SNAP * magnitude * perimeter
}

/// Why the plan overlay refused a measurement, in a report's words.
pub(crate) fn overlay_refusal(error: &OverlayError) -> &'static str {
    match error {
        OverlayError::RepeatedVertex => {
            "the plan overlay refused a footprint ring with two corners within its tolerance \
             (RepeatedVertex)"
        }
        OverlayError::ZeroArea => {
            "the plan overlay refused a footprint ring without area (ZeroArea)"
        }
        OverlayError::SelfIntersection => {
            "the plan overlay refused a footprint whose edges cross (SelfIntersection)"
        }
        OverlayError::NonFinitePoint => {
            "the plan overlay refused a footprint with a non-finite point (NonFinitePoint)"
        }
        OverlayError::RingTooShort => {
            "the plan overlay refused a footprint ring of fewer than three points (RingTooShort)"
        }
        _ => "the plan overlay refused the footprints",
    }
}

/// A triangle set's plan shadows, split into those the overlay accepts at
/// a tolerance and the slivers it would refuse.
struct Shadows {
    kept: Vec<Polygon>,
    slivers: Vec<Polygon>,
}

impl Shadows {
    fn of(triangles: &[Triangle], tolerance: axiolid_core::Tolerance) -> Self {
        let (slivers, kept) = projected_polygons(triangles)
            .into_iter()
            .partition(|polygon| refused_ring(&polygon.outer, tolerance));
        Self { kept, slivers }
    }

    /// The closed box of every shadow, kept or not; `None` when there is
    /// none.
    fn bounds(&self) -> Option<(Point2, Point2)> {
        self.kept
            .iter()
            .chain(&self.slivers)
            .flat_map(|polygon| &polygon.outer.points)
            .fold(None, |bounds, point| {
                Some(match bounds {
                    None => (*point, *point),
                    Some((low, high)) => (
                        Point2::new(low.x.min(point.x), low.y.min(point.y)),
                        Point2::new(high.x.max(point.x), high.y.max(point.y)),
                    ),
                })
            })
    }

    /// An upper bound on the area of the slivers whose closed box meets
    /// `bounds`, their rounding included: a sliver apart from that box
    /// shares no point with anything inside it.
    fn slivers_meeting(&self, bounds: Option<(Point2, Point2)>) -> f64 {
        let Some((low, high)) = bounds else {
            return 0.0;
        };
        self.slivers
            .iter()
            .filter(|polygon| {
                let points = &polygon.outer.points;
                points.iter().any(|point| point.x >= low.x)
                    && points.iter().any(|point| point.x <= high.x)
                    && points.iter().any(|point| point.y >= low.y)
                    && points.iter().any(|point| point.y <= high.y)
            })
            .map(|polygon| area_bound(&polygon.outer.points))
            .sum()
    }
}

/// Whether the overlay refuses a triangle's ring as degenerate: two corners
/// within its linear tolerance, or an area within its square. The same
/// tests `axiolid-overlay` runs (`validate_ring`), in the same arithmetic;
/// a ring this misjudges as accepted is refused by the overlay itself,
/// never measured wrongly.
fn refused_ring(ring: &Ring, tolerance: axiolid_core::Tolerance) -> bool {
    let points = &ring.points;
    let linear = tolerance.linear();
    (0..points.len())
        .any(|index| (points[index] - points[(index + 1) % points.len()]).length() <= linear)
        || ring_area(ring).abs() <= linear * linear
}

/// An upper bound on a triangle's area, its rounding included.
///
/// The edge vectors are differences of floating-point coordinates, each
/// correctly rounded, so the cross product is computed from them to within
/// a few units in the last place of `|ab| |ac|`; four of them bound it.
fn area_bound(points: &[Point2]) -> f64 {
    let [a, b, c] = [points[0], points[1], points[2]];
    let (ab, ac) = (b - a, c - a);
    0.5 * (ab.perp_dot(ac).abs() + 4.0 * f64::EPSILON * ab.length() * ac.length())
}

/// Sides of the regular polygons that stand in for a disc when a footprint
/// is grown: a disc has no exact polygon, so growth is bracketed between an
/// inscribed and a circumscribed one.
const GROWTH_SIDES: u32 = 16;

/// Which regular polygon stands in for the growth disc of radius `r`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disc {
    /// Vertices on the circle, including the four axis directions: inside the
    /// disc, and reaching exactly `r` along each axis.
    Inscribed,
    /// Edges tangent to the circle, facing the four axis directions: holding
    /// the disc, and reaching exactly `r` along each axis.
    Circumscribed,
}

impl Disc {
    fn vertices(self, radius: f64) -> Vec<(f64, f64)> {
        let step = std::f64::consts::PI / f64::from(GROWTH_SIDES / 2);
        let (offset, reach) = match self {
            Self::Inscribed => (0.0, radius),
            Self::Circumscribed => (step / 2.0, radius / (step / 2.0).cos()),
        };
        (0..GROWTH_SIDES)
            .map(|index| {
                let angle = offset + step * f64::from(index);
                (reach * angle.cos(), reach * angle.sin())
            })
            .collect()
    }
}

/// The convex hull of plan points (monotone chain), counter-clockwise, with
/// collinear points dropped; a segment's hull is its two ends.
pub(crate) fn hull_of(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let mut hull: Vec<(f64, f64)> = Vec::with_capacity(2 * points.len());
    for pass in [false, true] {
        let floor = hull.len() + 1;
        let ordered: Box<dyn Iterator<Item = &(f64, f64)>> = if pass {
            Box::new(points.iter().rev().skip(1))
        } else {
            Box::new(points.iter())
        };
        for point in ordered {
            while hull.len() >= floor.max(2)
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0
            {
                hull.pop();
            }
            hull.push(*point);
        }
    }
    hull.pop();
    hull
}

/// The smallest growth drawn as a polygon, in metres. The overlay refuses
/// vertices closer than its tolerance, so a smaller inscribed growth is
/// dropped (growing less is still inside the true growth) and a smaller
/// circumscribed one is rounded up to it (growing more still holds it).
const MINIMUM_GROWTH: f64 = 1e-6;

/// A triangle set's footprint grown by `radius` in every plan direction, as
/// counter-clockwise triangles whose non-zero union is the grown footprint,
/// with the growth disc replaced by `disc`.
///
/// Growth distributes over a union, and a triangle grown by a convex polygon
/// is the convex hull of its vertices moved to each of the polygon's
/// vertices, so each projected triangle is grown on its own and exactly. The
/// hull is handed over as a fan of triangles: the overlay's ring check
/// refuses a polygon with a vertex on the line through a non-adjacent edge,
/// which a regular polygon's hull can have, and a triangle has no
/// non-adjacent edges. A zero radius is the footprint itself.
pub(crate) fn grown_polygons(triangles: &[Triangle], radius: f64, disc: Disc) -> Vec<Polygon> {
    let footprint = projected_polygons(triangles);
    let radius = match disc {
        Disc::Inscribed if radius < MINIMUM_GROWTH => 0.0,
        Disc::Circumscribed if radius > 0.0 => radius.max(MINIMUM_GROWTH),
        _ => radius,
    };
    if radius <= 0.0 {
        return footprint;
    }
    let offsets = disc.vertices(radius);
    let mut grown = Vec::new();
    for polygon in &footprint {
        let hull = hull_of(
            polygon
                .outer
                .points
                .iter()
                .flat_map(|point| {
                    offsets
                        .iter()
                        .map(move |(dx, dy)| (point.x + dx, point.y + dy))
                })
                .collect(),
        );
        let Some(&apex) = hull.first() else {
            continue;
        };
        for pair in hull[1..].windows(2) {
            let ring = Ring {
                points: [apex, pair[0], pair[1]]
                    .into_iter()
                    .map(|(x, y)| Point2::new(x, y))
                    .collect(),
            };
            if ring_area(&ring) > f64::EPSILON {
                grown.push(Polygon {
                    outer: ring,
                    holes: Vec::new(),
                });
            }
        }
    }
    grown
}

/// Area where two polygon sets overlap, each filled as its non-zero union.
///
/// `None` when the overlay cannot be computed; an empty set overlaps nothing.
pub(crate) fn polygons_overlap_area(
    first: Vec<Polygon>,
    second: Vec<Polygon>,
    tolerance: axiolid_core::Tolerance,
) -> Option<f64> {
    if first.is_empty() || second.is_empty() {
        return Some(0.0);
    }
    let result = overlay(
        &OverlayInput {
            frame: plan_frame(),
            polygons: first,
        },
        &OverlayInput {
            frame: plan_frame(),
            polygons: second,
        },
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .ok()?;
    Some(result.polygons.iter().map(polygon_area).sum())
}

#[cfg(test)]
mod polygon_area_tests {
    use super::{
        Disc, bounded_footprint, bounded_plan_overlap, collinear, footprint_polygons,
        grown_polygons, plan_overlap_area, polygon_area, polygon_moments, projected_polygons,
        ring_area,
    };
    use axiolid_core::{Point2, Point3, Tolerance};
    use axiolid_overlay::{Polygon, Ring};

    /// A vertical face along a diagonal away from the origin: its corners
    /// project onto one line, but the shoelace sum over its rounded
    /// coordinates gives its shadow an area well above `f64::EPSILON`.
    fn vertical_face() -> [[Point3; 3]; 2] {
        let (sin, cos) = 1.1_f64.sin_cos();
        let at = |t: f64, z: f64| Point3::new(101.3 + cos * t, 202.6 + sin * t, z);
        [
            [at(0.0, 0.0), at(1.0, 0.0), at(3.0, 3.0)],
            [at(0.0, 0.0), at(3.0, 3.0), at(0.0, 3.0)],
        ]
    }

    #[test]
    fn a_vertical_faces_shadow_is_left_out() {
        let face = vertical_face();
        let shadow: Vec<Point2> = face[0].iter().map(|p| Point2::new(p.x, p.y)).collect();
        assert!(collinear(&shadow));
        assert!(ring_area(&Ring { points: shadow }).abs() > f64::EPSILON);
        assert!(projected_polygons(&face).is_empty());
        // Beside a body with a footprint, the face adds nothing to it.
        let floor = [
            Point3::new(101.0, 202.0, 0.0),
            Point3::new(102.0, 202.0, 0.0),
            Point3::new(101.0, 203.0, 0.0),
        ];
        let tolerance = Tolerance::METRE;
        let alone = footprint_polygons(&[floor], tolerance).unwrap();
        let with_face = footprint_polygons(&[floor, face[0], face[1]], tolerance).unwrap();
        assert_eq!(alone, with_face);
        assert!((polygon_area(&with_face[0]) - 0.5).abs() < 1e-6);
    }

    /// A near-vertical sliver that still has area is refused by the
    /// overlay; the bounded overlap leaves it out and keeps its area.
    #[test]
    fn a_sliver_is_left_out_with_its_area() {
        let tolerance = Tolerance::new(1e-9, 1e-9).unwrap();
        let lean = 1e-10;
        let sliver = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, lean, 3.0),
        ];
        assert_eq!(projected_polygons(&[sliver]).len(), 1, "not collinear");
        let floor = [
            [
                Point3::new(-1.0, -1.0, 0.0),
                Point3::new(5.0, -1.0, 0.0),
                Point3::new(5.0, 1.0, 0.0),
            ],
            [
                Point3::new(-1.0, -1.0, 0.0),
                Point3::new(5.0, 1.0, 0.0),
                Point3::new(-1.0, 1.0, 0.0),
            ],
        ];
        assert_eq!(plan_overlap_area(&[sliver], &floor, tolerance), None);
        let bounded = bounded_plan_overlap(&[sliver], &floor, tolerance).unwrap();
        assert!(bounded.polygons.is_empty());
        let area = 0.5 * 4.0 * lean;
        assert!(
            bounded.slivers >= area && bounded.slivers <= area + 1e-13,
            "{}",
            bounded.slivers
        );
        // Beside a footprint the overlay accepts, only the sliver is left out.
        let footprint = [sliver, floor[0]];
        let bounded = bounded_plan_overlap(&footprint, &floor, tolerance).unwrap();
        let measured: f64 = bounded.polygons.iter().map(polygon_area).sum();
        assert!((measured - 6.0).abs() < 1e-6, "{measured}");
        assert!(bounded.slivers <= area + 1e-13);
        // A sliver apart from the other set's box cannot add to the overlap.
        let away = floor.map(|triangle| triangle.map(|p| Point3::new(p.x + 10.0, p.y, p.z)));
        let bounded = bounded_plan_overlap(&[sliver], &away, tolerance).unwrap();
        assert!(bounded.polygons.is_empty());
        assert!(bounded.slivers.abs() < f64::MIN_POSITIVE);
        // Its own footprint is bounded by it.
        let footprint = bounded_footprint(&[sliver, floor[0]], tolerance).unwrap();
        let measured: f64 = footprint.polygons.iter().map(polygon_area).sum();
        assert!((measured - 6.0).abs() < 1e-6, "{measured}");
        assert!(footprint.slivers >= area && footprint.slivers <= area + 1e-13);
    }

    #[test]
    fn a_thin_triangle_is_not_collinear() {
        let points = [
            Point2::new(0.0, 0.0),
            Point2::new(10.0, 0.0),
            Point2::new(5.0, 1e-9),
        ];
        assert!(!collinear(&points));
        assert!(collinear(&[Point2::new(1.0, 1.0); 3]));
    }

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

    #[test]
    fn a_grown_triangle_is_bracketed_by_the_two_discs() {
        use axiolid_core::Point3;
        let square = [
            [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
            ],
            [
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
        ];
        let tolerance = axiolid_core::Tolerance::new(1e-9, 1e-9).unwrap();
        let area = |disc| {
            super::polygons_overlap_area(
                grown_polygons(&square, 0.1, disc),
                grown_polygons(&square, 1.0, Disc::Circumscribed),
                tolerance,
            )
            .unwrap()
        };
        // A unit square grown by a disc of 0.1 m: 1 + 4 · 0.1 + π · 0.01.
        let round = 1.0 + 0.4 + std::f64::consts::PI * 0.01;
        let (inner, outer) = (area(Disc::Inscribed), area(Disc::Circumscribed));
        assert!(inner < round && round < outer, "{inner} {round} {outer}");
        // Along the axes both reach exactly 0.1 m; only the corners differ.
        assert!(outer - inner < 0.002, "{inner} {outer}");
        assert_eq!(grown_polygons(&square, 0.0, Disc::Inscribed).len(), 2);
        // Below the smallest drawn growth, inscribed growth is dropped.
        assert_eq!(grown_polygons(&square, 1e-9, Disc::Inscribed).len(), 2);
        assert!(grown_polygons(&square, 1e-9, Disc::Circumscribed).len() > 2);
    }
}
