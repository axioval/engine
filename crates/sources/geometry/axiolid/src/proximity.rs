//! Pairwise proximity measured with the Axiolid kernel.
//!
//! ADR 0004: this module **measures**. It reports how close two bodies come,
//! how far their footprints overlap and how deep one reaches into the other.
//! Whether that is a clash is a rule's decision.
//!
//! Three measurements, each from a published Axiolid primitive:
//!
//! - **Separation** folds `closest_points_on_triangles` over triangle pairs,
//!   skipping pairs whose boxes are already farther apart than the best found.
//! - **Plan overlap** intersects the projected triangle soups, as contact does.
//! - **Penetration** is witnessed. Zero separation means the surfaces meet;
//!   it does not say whether the bodies only touch or interpenetrate. Points of
//!   one body are sampled -- vertices, edge and face centres, and the midpoints
//!   between where each edge crosses the other surface -- and a point the
//!   other body's winding number places inside it contributes its distance to
//!   that surface. A pipe through a wall has no vertex inside the wall, but its
//!   edges cross both wall faces and the midpoint between the crossings lies
//!   half a wall deep. The deepest witness is a lower bound on the true depth.
//!
//! Two shape comparisons ride along:
//!
//! - **Overlap extents** bound the intersection's box along each axis. The
//!   lower bound spans points witnessed in both bodies: where an edge of
//!   either crosses the other's surface, and vertices of either inside the
//!   other. For polyhedra those are the intersection's vertices, so the bound
//!   is tight wherever the crossings are found. The upper bound is the
//!   overlap of the two bodies' boxes. The intersection's volume is not
//!   measured: that needs a certified mesh boolean (axiolid/kernel#183).
//! - **Hausdorff distance** between the surfaces. Its lower bound is the
//!   farthest any vertex lies from the other surface. Its upper bound holds
//!   per triangle: a distance to one convex triangle is convex, so the
//!   farthest point of a triangle from it is a vertex, and the least over
//!   the other body's triangles of that farthest vertex distance bounds the
//!   whole triangle. Triangles whose bound exceeds the lower bound are split
//!   a few times to tighten it. Identical meshes come out exactly zero.
//!
//! Winding numbers need an inside, so points are only tested against a closed
//! two-manifold mesh. A surface reaching into a solid is measured; two open
//! surfaces share no volume and report `None` rather than a depth of zero.
//!
//! Distances in a projection ([`ProximityService::measure_distance`]) reuse
//! the same pieces:
//!
//! - **Horizontal** distance folds 2D closest points over the projected
//!   triangles, edge-on ones as segments, through the same indexed search as
//!   separation. The footprint is the union of the projected triangles and the
//!   distance between two unions is the least distance between their parts,
//!   so non-convex footprints are measured exactly without a polygon
//!   boundary-distance primitive.
//! - **Vertical** distance is the gap between the two meshes' vertical
//!   extents, for bodies whose footprints are related (see
//!   [`axioval_engine::ProximityProjection::Vertical`]). A direction takes
//!   the one-sided gap and relates only a counterpart on that side
//!   ([`axioval_engine::VerticalDirection`]), compared end by end.
//! - **Plan overlap** is the footprint overlay of plan overlap measurement.
//!
//! A tessellation widens every distance by the combined chord deviation. Its
//! footprint may differ from the mesh footprint by up to the deviation, so
//! overlap is only asserted from a witness point lying deeper than the
//! deviations inside both mesh footprints, and denied only when the plan
//! distance exceeds them; anything between is reported open. A direction is
//! decided the same way: a tessellated end may move by the deviation, so ends
//! nearer each other than the combined deviation leave the side open.

use axiolid_core::{Aabb, Point3, Ray3, Tolerance};
use axiolid_measure::{
    WindingMesh, closest_point_on_triangle, closest_points_on_segments, closest_points_on_triangles,
};
use axiolid_mesh::{TriMesh, audit_mesh};
use axiolid_ray_mesh::intersect_triangle;
use axiolid_spatial::{Bvh, SpatialItem};
use axioval_engine::{
    BodyContainment, Bounds3, GeometryFidelity, LengthInterval, ObjectBounds, OverlapExtents,
    ProjectedDistanceEvidence, ProximityError, ProximityEvidence, ProximityProjection,
    ProximityRequest, ProximityService, VerticalDirection,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{plan_overlap_area, plan_overlap_polygons};

/// Linear tolerance for mesh audits, overlay and crossing tests.
///
/// Also the distance below which surfaces are taken to meet: floating-point
/// closest points of two touching faces rarely come out exactly zero.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// Plan overlap area below which exact footprints only touch.
///
/// The overlay snaps its output to a grid (axiolid/kernel#173), so footprints
/// meeting along an edge can come back as a sliver of rounding.
const OVERLAP_AREA_TOLERANCE: f64 = 1e-9;

/// A point is inside a closed body when its winding number reaches one half.
const INSIDE_WINDING: f64 = 0.5;

/// How often a triangle is split in four to tighten its Hausdorff bound.
const HAUSDORFF_REFINEMENT: u32 = 2;

/// Measures pairwise proximity between registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidProximityService {
    geometry: AxiolidGeometry,
}

impl AxiolidProximityService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self { geometry }
    }

    fn body(&self, object: &ObjectId) -> Result<Body<'_>, ProximityError> {
        if self.geometry.has_no_body(object) {
            return Err(ProximityError::NoBody);
        }
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or(ProximityError::Unavailable)?;
        let triangles = triangles(mesh);
        let tolerance = tolerance()?;
        let health = audit_mesh(mesh, tolerance);
        // Every coordinate feeds a measurement reported as evidence; a mesh
        // with bad indices or non-finite positions cannot be measured at all.
        if triangles.is_empty() || !health.is_surface_usable() {
            return Err(ProximityError::Unavailable);
        }
        let boxes: Vec<Bounds3> = triangles.iter().map(triangle_box).collect();
        Ok(Body {
            mesh,
            soup: Indexed::build(triangles, boxes)?,
            solid: health.is_closed_two_manifold(),
        })
    }

    /// Measures the pair in the request's projection.
    fn projected_interval(
        &self,
        request: &ProximityRequest,
        subject: &Body<'_>,
        counterpart: &Body<'_>,
    ) -> Result<(f64, f64), ProximityError> {
        let subject_fidelity = self.geometry.fidelity(request.subject())?;
        let counterpart_fidelity = self.geometry.fidelity(request.counterpart())?;
        let deviation = subject_fidelity
            .combined(counterpart_fidelity)
            .deviation_metres();
        let widen = |distance: f64| ((distance - deviation).max(0.0), distance + deviation);
        Ok(match request.projection() {
            ProximityProjection::Minimum3d => widen(separation(&subject.soup, &counterpart.soup)?),
            ProximityProjection::Horizontal => widen(plan_separation(subject, counterpart)?),
            ProximityProjection::PlanOverlap => {
                match relation(
                    subject,
                    counterpart,
                    0.0,
                    subject_fidelity,
                    counterpart_fidelity,
                )? {
                    Relation::Related => (0.0, 0.0),
                    Relation::Unrelated => (f64::INFINITY, f64::INFINITY),
                    Relation::Open => (0.0, f64::INFINITY),
                }
            }
            ProximityProjection::Vertical {
                footprint_offset_metres,
                direction,
            } => {
                let side = side(subject, counterpart, direction, deviation);
                if matches!(side, Relation::Unrelated) {
                    return Ok((f64::INFINITY, f64::INFINITY));
                }
                let (lower, upper) = widen(vertical_gap(subject, counterpart, direction));
                match (
                    side,
                    relation(
                        subject,
                        counterpart,
                        footprint_offset_metres,
                        subject_fidelity,
                        counterpart_fidelity,
                    )?,
                ) {
                    (_, Relation::Unrelated) => (f64::INFINITY, f64::INFINITY),
                    (Relation::Related, Relation::Related) => (lower, upper),
                    _ => (lower, f64::INFINITY),
                }
            }
        })
    }
}

/// Items with their boxes and a bounding-volume hierarchy over them, so a
/// query touches only the items near it instead of scanning them all.
struct Indexed<T> {
    items: Vec<T>,
    boxes: Vec<Bounds3>,
    index: Bvh<usize>,
    bounds: Bounds3,
}

impl<T> Indexed<T> {
    fn build(items: Vec<T>, boxes: Vec<Bounds3>) -> Result<Self, ProximityError> {
        let mut all = boxes.iter();
        let first = *all.next().ok_or(ProximityError::Unavailable)?;
        let (mut min, mut max) = (first.min(), first.max());
        for bounds in all {
            for axis in 0..3 {
                min[axis] = min[axis].min(bounds.min()[axis]);
                max[axis] = max[axis].max(bounds.max()[axis]);
            }
        }
        let bounds = Bounds3::try_new(min, max)?;
        let index = Bvh::build(
            boxes
                .iter()
                .enumerate()
                .map(|(item, bounds)| SpatialItem::new(item, aabb(bounds))),
        );
        // Audited coordinates are finite, so every box is accepted; a rejected
        // one would be an item no query could ever find.
        if index.rejected_items() != 0 {
            return Err(ProximityError::Unavailable);
        }
        Ok(Self {
            items,
            boxes,
            index,
            bounds,
        })
    }

    /// Indices of the items whose boxes meet `probe`, in index order.
    fn near(&self, probe: &Bounds3) -> Vec<usize> {
        let mut hits = Vec::new();
        self.index.query_aabb(&aabb(probe), &mut hits);
        let mut items: Vec<usize> = hits
            .into_iter()
            .filter_map(|hit| self.index.item(hit).map(|item| item.key))
            .collect();
        items.sort_unstable();
        items
    }
}

struct Body<'a> {
    mesh: &'a TriMesh,
    soup: Indexed<Triangle>,
    solid: bool,
}

fn aabb(bounds: &Bounds3) -> Aabb {
    Aabb {
        min: Point3::from_array(bounds.min()),
        max: Point3::from_array(bounds.max()),
    }
}

fn tolerance() -> Result<Tolerance, ProximityError> {
    Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE).map_err(|_| ProximityError::Unavailable)
}

fn triangle_box(triangle: &Triangle) -> Bounds3 {
    let [a, b, c] = *triangle;
    let min = a.min(b).min(c);
    let max = a.max(b).max(c);
    // Coordinates were audited finite, so the box is well-formed.
    Bounds3::try_new(min.to_array(), max.to_array())
        .unwrap_or_else(|_| unreachable!("audited mesh has finite coordinates"))
}

/// Shortest distance between two indexed item sets under `distance`.
///
/// Each item of `first` is measured only against the items of `second`
/// inside its box grown by the best distance so far. Anything outside that box
/// is farther than the best already found, so the skip loses nothing.
fn nearest<T>(
    first: &Indexed<T>,
    second: &Indexed<T>,
    distance: impl Fn(&T, &T) -> Result<f64, ProximityError>,
) -> Result<f64, ProximityError> {
    let mut best = f64::INFINITY;
    for (a, a_box) in first.items.iter().zip(&first.boxes) {
        if a_box.gap(&second.bounds) >= best {
            continue;
        }
        let probe = if best.is_finite() {
            a_box.expanded(best)
        } else {
            second.bounds
        };
        for index in second.near(&probe) {
            let b_box = &second.boxes[index];
            if a_box.gap(b_box) >= best {
                continue;
            }
            let measured = distance(a, &second.items[index])?;
            if !measured.is_finite() {
                return Err(ProximityError::InvalidMeasurement);
            }
            best = best.min(measured);
        }
        if best <= LINEAR_TOLERANCE {
            return Ok(0.0);
        }
    }
    if best.is_finite() {
        Ok(best)
    } else {
        Err(ProximityError::Unavailable)
    }
}

/// Shortest distance between two triangle sets.
fn separation(
    first: &Indexed<Triangle>,
    second: &Indexed<Triangle>,
) -> Result<f64, ProximityError> {
    nearest(first, second, |a, b| {
        closest_points_on_triangles(*a, *b)
            .map(|pair| pair.distance_squared.sqrt())
            .map_err(|_| ProximityError::Unavailable)
    })
}

/// One projected triangle: a triangle in the plan, or a segment when the
/// triangle stands edge-on (a wall face, an open vertical sheet).
#[derive(Clone, Copy)]
enum Flat {
    Triangle(Triangle),
    Segment([Point3; 2]),
}

/// `triangle` projected onto z = 0.
fn flatten(triangle: &Triangle) -> Flat {
    let [a, b, c] = triangle.map(|point| Point3::new(point.x, point.y, 0.0));
    if (b - a).cross(c - a).length_squared() != 0.0 {
        return Flat::Triangle([a, b, c]);
    }
    // Collinear: the projection is the segment between the two points
    // farthest apart.
    [[a, b], [b, c], [c, a]]
        .into_iter()
        .max_by(|[p, q], [r, t]| {
            (*q - *p)
                .length_squared()
                .total_cmp(&(*t - *r).length_squared())
        })
        .map_or(Flat::Segment([a, a]), Flat::Segment)
}

fn flat_box(flat: &Flat) -> Bounds3 {
    match flat {
        Flat::Triangle(triangle) => triangle_box(triangle),
        Flat::Segment([a, b]) => Bounds3::try_new(a.min(*b).to_array(), a.max(*b).to_array())
            .unwrap_or_else(|_| unreachable!("audited mesh has finite coordinates")),
    }
}

fn segment_distance(a: [Point3; 2], b: [Point3; 2]) -> Result<f64, ProximityError> {
    closest_points_on_segments(a, b)
        .map(|pair| pair.distance_squared.sqrt())
        .map_err(|_| ProximityError::Unavailable)
}

/// Plan distance from a segment to a triangle in the same plane: zero when an
/// end lies inside, otherwise the least distance to one of its edges.
fn segment_triangle_distance(
    segment: [Point3; 2],
    triangle: Triangle,
) -> Result<f64, ProximityError> {
    let mut best = f64::INFINITY;
    for point in segment {
        let closest =
            closest_point_on_triangle(point, triangle).map_err(|_| ProximityError::Unavailable)?;
        best = best.min(closest.distance(point));
    }
    let [a, b, c] = triangle;
    for edge in [[a, b], [b, c], [c, a]] {
        best = best.min(segment_distance(segment, edge)?);
    }
    Ok(best)
}

fn flat_distance(first: &Flat, second: &Flat) -> Result<f64, ProximityError> {
    match (first, second) {
        (Flat::Triangle(a), Flat::Triangle(b)) => closest_points_on_triangles(*a, *b)
            .map(|pair| pair.distance_squared.sqrt())
            .map_err(|_| ProximityError::Unavailable),
        (Flat::Segment(a), Flat::Segment(b)) => segment_distance(*a, *b),
        (Flat::Segment(segment), Flat::Triangle(triangle))
        | (Flat::Triangle(triangle), Flat::Segment(segment)) => {
            segment_triangle_distance(*segment, *triangle)
        }
    }
}

/// A body's footprint as indexed projected triangles.
fn footprint(body: &Body<'_>) -> Result<Indexed<Flat>, ProximityError> {
    let flats: Vec<Flat> = body.soup.items.iter().map(flatten).collect();
    let boxes = flats.iter().map(flat_box).collect();
    Indexed::build(flats, boxes)
}

/// Plan distance between two bodies' footprints; zero when they meet.
fn plan_separation(first: &Body<'_>, second: &Body<'_>) -> Result<f64, ProximityError> {
    nearest(&footprint(first)?, &footprint(second)?, flat_distance)
}

/// Gap between the two meshes' vertical extents in `direction`: from the
/// subject's top up to the counterpart's bottom (`Above`), from its bottom
/// down to the counterpart's top (`Below`), or either; zero when they overlap.
fn vertical_gap(subject: &Body<'_>, counterpart: &Body<'_>, direction: VerticalDirection) -> f64 {
    let (a, b) = (subject.soup.bounds, counterpart.soup.bounds);
    let rise = b.min()[2] - a.max()[2];
    let drop = a.min()[2] - b.max()[2];
    match direction {
        VerticalDirection::Either => rise.max(drop),
        VerticalDirection::Above => rise,
        VerticalDirection::Below => drop,
    }
    .max(0.0)
}

/// Whether the counterpart lies in `direction` from the subject: above
/// unless lower at both ends, below unless higher at both ends.
///
/// Each true end may lie up to its body's chord deviation from the mesh end,
/// so an end difference within the combined `deviation` of zero decides
/// nothing: the side is asserted only beyond it and denied only when both
/// ends are beyond it on the other side.
fn side(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    direction: VerticalDirection,
    deviation: f64,
) -> Relation {
    let (a, b) = (subject.soup.bounds, counterpart.soup.bounds);
    // How far the counterpart's top and bottom lie in `direction` past the
    // subject's.
    let ends = match direction {
        VerticalDirection::Either => return Relation::Related,
        VerticalDirection::Above => [b.max()[2] - a.max()[2], b.min()[2] - a.min()[2]],
        VerticalDirection::Below => [a.max()[2] - b.max()[2], a.min()[2] - b.min()[2]],
    };
    if ends.iter().any(|past| past - deviation >= 0.0) {
        Relation::Related
    } else if ends.iter().all(|past| past + deviation < 0.0) {
        Relation::Unrelated
    } else {
        Relation::Open
    }
}

/// Whether two bodies are related in plan, or whether the geometry's fidelity
/// leaves it open.
enum Relation {
    Related,
    Unrelated,
    Open,
}

/// Whether the footprints overlap with positive area (`offset` zero) or come
/// closer than `offset` (positive).
///
/// Exact footprints are decided. A tessellated footprint may lie anywhere
/// within its chord deviation of the mesh footprint, so a relation is only
/// asserted or denied when the deviations cannot change it.
fn relation(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    offset: f64,
    subject_fidelity: GeometryFidelity,
    counterpart_fidelity: GeometryFidelity,
) -> Result<Relation, ProximityError> {
    let exact = subject_fidelity.is_exact() && counterpart_fidelity.is_exact();
    let deviation = subject_fidelity.deviation_metres() + counterpart_fidelity.deviation_metres();
    if offset > 0.0 {
        let distance = plan_separation(subject, counterpart)?;
        return Ok(if distance + deviation < offset {
            Relation::Related
        } else if distance - deviation >= offset {
            Relation::Unrelated
        } else {
            Relation::Open
        });
    }
    if exact {
        let area = plan_overlap_area(&subject.soup.items, &counterpart.soup.items, tolerance()?)
            .ok_or(ProximityError::Unavailable)?;
        return Ok(if area > OVERLAP_AREA_TOLERANCE {
            Relation::Related
        } else {
            Relation::Unrelated
        });
    }
    if plan_separation(subject, counterpart)? > deviation {
        return Ok(Relation::Unrelated);
    }
    let overlap = plan_overlap_polygons(&subject.soup.items, &counterpart.soup.items, tolerance()?)
        .ok_or(ProximityError::Unavailable)?;
    let depth = subject_fidelity
        .deviation_metres()
        .max(counterpart_fidelity.deviation_metres());
    Ok(
        if overlap.iter().any(|polygon| deep_point(polygon, depth)) {
            Relation::Related
        } else {
            Relation::Open
        },
    )
}

/// Whether `polygon` holds a point farther than `depth` from its boundary.
///
/// A witness only: the candidates are the ring's area centroid and the
/// centroids of its fan triangles. Finding none leaves the question open; it
/// never denies depth.
fn deep_point(polygon: &axiolid_overlay::Polygon, depth: f64) -> bool {
    use axiolid_core::Point2;
    let outer = &polygon.outer.points;
    if outer.len() < 3 {
        return false;
    }
    let mut candidates = Vec::new();
    let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for index in 1..outer.len() - 1 {
        let (a, b, c) = (outer[0], outer[index], outer[index + 1]);
        let signed = ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)) / 2.0;
        let centre = Point2::new((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
        area += signed;
        cx += signed * centre.x;
        cy += signed * centre.y;
        candidates.push(centre);
    }
    if area != 0.0 {
        candidates.insert(0, Point2::new(cx / area, cy / area));
    }
    let rings: Vec<&Vec<Point2>> = std::iter::once(outer)
        .chain(polygon.holes.iter().map(|hole| &hole.points))
        .collect();
    candidates.into_iter().any(|point| {
        let inside = contains(outer, point)
            && !polygon
                .holes
                .iter()
                .any(|hole| contains(&hole.points, point));
        inside
            && rings
                .iter()
                .all(|ring| ring_distance(ring, point) > depth + LINEAR_TOLERANCE)
    })
}

/// Even-odd point-in-ring test.
fn contains(ring: &[axiolid_core::Point2], point: axiolid_core::Point2) -> bool {
    let mut inside = false;
    for index in 0..ring.len() {
        let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
        if (a.y > point.y) != (b.y > point.y) {
            let x = a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if point.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Distance from `point` to the nearest edge of a ring.
fn ring_distance(ring: &[axiolid_core::Point2], point: axiolid_core::Point2) -> f64 {
    let lift = |p: axiolid_core::Point2| Point3::new(p.x, p.y, 0.0);
    (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            closest_points_on_segments([lift(a), lift(b)], [lift(point), lift(point)])
                .map_or(0.0, |pair| pair.distance_squared.sqrt())
        })
        .fold(f64::INFINITY, f64::min)
}

/// Distance from a point to a triangle set's surface.
///
/// The triangle whose box is nearest bounds the answer from above; only
/// triangles within that bound can improve on it.
fn surface_distance(point: Point3, body: &Body<'_>) -> Result<f64, ProximityError> {
    surface_distance_above(point, body, 0.0)
}

/// [`surface_distance`] when it exceeds `floor`; otherwise some distance no
/// greater than `floor`, found without the full search.
fn surface_distance_above(
    point: Point3,
    body: &Body<'_>,
    floor: f64,
) -> Result<f64, ProximityError> {
    let to = |index: usize| -> Result<f64, ProximityError> {
        closest_point_on_triangle(point, body.soup.items[index])
            .map(|closest| closest.distance(point))
            .map_err(|_| ProximityError::Unavailable)
    };
    let nearest = body
        .soup
        .index
        .nearest_to(&Aabb::from_point(point), |_| true)
        .ok_or(ProximityError::Unavailable)?;
    let mut best = to(nearest.key)?;
    if best <= floor {
        return Ok(best);
    }
    let probe = Bounds3::try_new(point.to_array(), point.to_array())?.expanded(best);
    for index in body.soup.near(&probe) {
        best = best.min(to(index)?);
    }
    Ok(best)
}

/// Points of `body` at which to test whether it reaches into `other`.
fn sample_points(body: &Body<'_>, other: &Body<'_>) -> Result<Vec<Point3>, ProximityError> {
    let tolerance = tolerance()?;
    let mut points = Vec::new();
    let mut centroid_sum = Point3::ZERO;
    let mut corners = 0.0;
    for [a, b, c] in &body.soup.items {
        points.extend([*a, *b, *c, (*a + *b + *c) / 3.0]);
        centroid_sum += *a + *b + *c;
        corners += 3.0;
        for (start, end) in [(*a, *b), (*b, *c), (*c, *a)] {
            points.push((start + end) / 2.0);
            points.extend(crossing_midpoints(start, end, other, tolerance)?);
        }
    }
    // A body lying exactly on another's faces, a duplicate for instance, has
    // every surface point on the other's surface; its centre does not.
    points.push(centroid_sum / corners);
    Ok(points)
}

/// Midpoints between successive crossings of segment `start..end` with the
/// other body's surface, where a stretch of the edge may lie inside it.
fn crossing_midpoints(
    start: Point3,
    end: Point3,
    other: &Body<'_>,
    tolerance: Tolerance,
) -> Result<Vec<Point3>, ProximityError> {
    let direction = end - start;
    if direction.length_squared() == 0.0 {
        return Ok(Vec::new());
    }
    let segment = Bounds3::try_new(start.min(end).to_array(), start.max(end).to_array())?;
    if segment.gap(&other.soup.bounds) > 0.0 {
        return Ok(Vec::new());
    }
    let ray = Ray3 {
        origin: start,
        direction,
    };
    let mut crossings = vec![0.0, 1.0];
    for index in other.soup.near(&segment) {
        let hit = intersect_triangle(&ray, other.soup.items[index], tolerance, index)
            .map_err(|_| ProximityError::Unavailable)?;
        if let Some(hit) = hit.filter(|hit| (0.0..=1.0).contains(&hit.t)) {
            crossings.push(hit.t);
        }
    }
    if crossings.len() == 2 {
        return Ok(Vec::new());
    }
    crossings.sort_by(f64::total_cmp);
    Ok(crossings
        .windows(2)
        .map(|pair| start + direction * f64::midpoint(pair[0], pair[1]))
        .collect())
}

/// Deepest witnessed point of `body` inside `other`, zero when none is.
fn deepest_inside(body: &Body<'_>, other: &Body<'_>) -> Result<f64, ProximityError> {
    // A point's depth is its distance to the other surface, which the index
    // answers cheaply; whether it is inside at all costs a winding number over
    // every triangle. So rank the candidates by depth and test them deepest
    // first: the first one inside is the deepest witness, and the rest need
    // no winding test. Points outside the other body's box cannot be inside,
    // and points within tolerance of its surface are contact, not depth.
    let mut candidates = Vec::new();
    for point in sample_points(body, other)? {
        let within = (0..3).all(|axis| {
            (other.soup.bounds.min()[axis]..=other.soup.bounds.max()[axis]).contains(&point[axis])
        });
        if !within {
            continue;
        }
        let depth = surface_distance(point, other)?;
        if depth > LINEAR_TOLERANCE {
            candidates.push((depth, point));
        }
    }
    candidates.sort_by(|(a_depth, a), (b_depth, b)| {
        b_depth.total_cmp(a_depth).then_with(|| {
            a.to_array()
                .partial_cmp(&b.to_array())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let winding =
        WindingMesh::prepare(other.mesh, tolerance()?).map_err(|_| ProximityError::Unavailable)?;
    for (depth, point) in candidates {
        if inside(&winding, point)? {
            return Ok(depth);
        }
    }
    // No sample lies inside by more than rounding: the bodies touch.
    Ok(0.0)
}

fn inside(winding: &WindingMesh<'_, TriMesh>, point: Point3) -> Result<bool, ProximityError> {
    let number = winding
        .winding_number(point)
        .map_err(|_| ProximityError::Unavailable)?;
    Ok(number.value.abs() >= INSIDE_WINDING)
}

/// Whether separated body `inner` lies inside `outer`.
///
/// With the surfaces apart, a body is wholly inside or wholly outside the
/// other, so one vertex decides.
fn contained(inner: &Body<'_>, outer: &Body<'_>) -> Result<bool, ProximityError> {
    let winding =
        WindingMesh::prepare(outer.mesh, tolerance()?).map_err(|_| ProximityError::Unavailable)?;
    inside(&winding, inner.soup.items[0][0])
}

/// Witnessed penetration and containment between two bodies.
///
/// Only a closed body has an inside, so only points reaching into a closed
/// body are tested. That is still complete when the other body is an open
/// surface: a surface has no volume for anything to reach into, so the
/// surface entering the solid is the whole of their overlap. Two open
/// surfaces share no volume to measure, and report `None`.
fn penetration(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    separation: f64,
) -> Result<(Option<f64>, Option<BodyContainment>), ProximityError> {
    if !subject.solid && !counterpart.solid {
        return Ok((None, None));
    }
    if separation > 0.0 {
        // Apart at the surface: one body is wholly inside the other or they
        // share nothing. Only a closed body can hold the other.
        if counterpart.solid && contained(subject, counterpart)? {
            return Ok((
                Some(deepest_inside(subject, counterpart)?),
                Some(BodyContainment::SubjectInsideCounterpart),
            ));
        }
        if subject.solid && contained(counterpart, subject)? {
            return Ok((
                Some(deepest_inside(counterpart, subject)?),
                Some(BodyContainment::CounterpartInsideSubject),
            ));
        }
        return Ok((Some(0.0), None));
    }
    let into_counterpart = if counterpart.solid {
        deepest_inside(subject, counterpart)?
    } else {
        0.0
    };
    let into_subject = if subject.solid {
        deepest_inside(counterpart, subject)?
    } else {
        0.0
    };
    Ok((Some(into_counterpart.max(into_subject)), None))
}

/// Every distinct vertex the body's triangles use.
fn vertices(body: &Body<'_>) -> Vec<Point3> {
    let mut points: Vec<Point3> = body.soup.items.iter().flatten().copied().collect();
    points.sort_by(|a, b| {
        a.to_array()
            .partial_cmp(&b.to_array())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    points.dedup_by(|a, b| a.to_array().map(f64::to_bits) == b.to_array().map(f64::to_bits));
    points
}

/// Points where the edges of `body` cross the surface of `other`.
fn crossings(body: &Body<'_>, other: &Body<'_>) -> Result<Vec<Point3>, ProximityError> {
    let tolerance = tolerance()?;
    let mut points = Vec::new();
    for [a, b, c] in &body.soup.items {
        for (start, end) in [(*a, *b), (*b, *c), (*c, *a)] {
            let direction = end - start;
            if direction.length_squared() == 0.0 {
                continue;
            }
            let segment = Bounds3::try_new(start.min(end).to_array(), start.max(end).to_array())?;
            if segment.gap(&other.soup.bounds) > 0.0 {
                continue;
            }
            let ray = Ray3 {
                origin: start,
                direction,
            };
            for index in other.soup.near(&segment) {
                let hit = intersect_triangle(&ray, other.soup.items[index], tolerance, index)
                    .map_err(|_| ProximityError::Unavailable)?;
                if let Some(hit) = hit.filter(|hit| (0.0..=1.0).contains(&hit.t)) {
                    points.push(start + direction * hit.t);
                }
            }
        }
    }
    Ok(points)
}

/// A box grown to hold witnessed points of the intersection.
#[derive(Default)]
struct Witnessed(Option<([f64; 3], [f64; 3])>);

impl Witnessed {
    fn add(&mut self, point: Point3) {
        let point = point.to_array();
        let (min, max) = self.0.get_or_insert((point, point));
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    /// Whether `value` would widen the box on the given side of `axis`.
    fn extends(&self, axis: usize, upward: bool, value: f64) -> bool {
        match self.0 {
            None => true,
            Some((min, max)) => {
                if upward {
                    value > max[axis]
                } else {
                    value < min[axis]
                }
            }
        }
    }
}

/// Widens `witnessed` by the vertices of `body` that lie inside `other`.
///
/// Only a vertex that would widen the box matters, and on each side of each
/// axis the outermost inside vertex is the only one that does. So vertices
/// are tried outermost first and each side stops at its first inside vertex
/// or at the first that would not widen the box: the winding test, linear in
/// the other body's size, runs only where it can change the answer.
fn add_inside_vertices(
    body: &Body<'_>,
    other: &Body<'_>,
    witnessed: &mut Witnessed,
) -> Result<(), ProximityError> {
    let bounds = other.soup.bounds;
    let points: Vec<Point3> = vertices(body)
        .into_iter()
        .filter(|point| {
            (0..3).all(|axis| (bounds.min()[axis]..=bounds.max()[axis]).contains(&point[axis]))
        })
        .collect();
    if points.is_empty() {
        return Ok(());
    }
    let winding =
        WindingMesh::prepare(other.mesh, tolerance()?).map_err(|_| ProximityError::Unavailable)?;
    let mut known: Vec<Option<bool>> = vec![None; points.len()];
    for axis in 0..3 {
        for upward in [false, true] {
            let mut order: Vec<usize> = (0..points.len()).collect();
            order.sort_by(|&a, &b| {
                let ordering = points[a][axis].total_cmp(&points[b][axis]);
                if upward { ordering.reverse() } else { ordering }
            });
            for index in order {
                let point = points[index];
                if !witnessed.extends(axis, upward, point[axis]) {
                    break;
                }
                let inside_other = if let Some(answer) = known[index] {
                    answer
                } else {
                    let answer = inside(&winding, point)?;
                    known[index] = Some(answer);
                    answer
                };
                if inside_other {
                    witnessed.add(point);
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Extents of the bodies' intersection along each axis, widened for the
/// geometry's fidelity.
///
/// The lower bound spans witnessed points of the intersection; the upper
/// bound is the overlap of the true bodies' enclosing boxes.
fn overlap_extents(
    subject: &Body<'_>,
    counterpart: &Body<'_>,
    separation: f64,
    containment: Option<BodyContainment>,
    subject_fidelity: GeometryFidelity,
    counterpart_fidelity: GeometryFidelity,
) -> Result<OverlapExtents, ProximityError> {
    let mut witnessed = Witnessed::default();
    // Bodies apart at the surface share nothing unless one holds the other.
    let shared = separation <= 0.0 || containment.is_some();
    if shared {
        for point in crossings(subject, counterpart)?
            .into_iter()
            .chain(crossings(counterpart, subject)?)
        {
            witnessed.add(point);
        }
        if counterpart.solid {
            add_inside_vertices(subject, counterpart, &mut witnessed)?;
        }
        if subject.solid {
            add_inside_vertices(counterpart, subject, &mut witnessed)?;
        }
    }
    let deviation = subject_fidelity
        .combined(counterpart_fidelity)
        .deviation_metres();
    let (a, b) = (
        subject
            .soup
            .bounds
            .expanded(subject_fidelity.deviation_metres()),
        counterpart
            .soup
            .bounds
            .expanded(counterpart_fidelity.deviation_metres()),
    );
    let axis = |axis: usize| -> Result<LengthInterval, ProximityError> {
        if !shared {
            return LengthInterval::exact(0.0).map_err(|_| ProximityError::InvalidMeasurement);
        }
        let upper = (a.max()[axis].min(b.max()[axis]) - a.min()[axis].max(b.min()[axis])).max(0.0);
        let lower = witnessed.0.map_or(0.0, |(min, max)| {
            // Each end of a tessellated extent may move by the deviation.
            (max[axis] - min[axis] - 2.0 * deviation).max(0.0)
        });
        LengthInterval::try_new(lower.min(upper), upper)
            .map_err(|_| ProximityError::InvalidMeasurement)
    };
    Ok(OverlapExtents::new(axis(0)?, axis(1)?, axis(2)?))
}

/// Distance from `point` to one triangle.
fn triangle_distance(point: Point3, triangle: Triangle) -> Result<f64, ProximityError> {
    closest_point_on_triangle(point, triangle)
        .map(|closest| closest.distance(point))
        .map_err(|_| ProximityError::Unavailable)
}

/// An upper bound on how far any point of `triangle` lies from `other`'s
/// surface: the least, over `other`'s triangles, of the farthest vertex
/// distance to that one triangle.
///
/// A bound already no greater than `floor` is returned as soon as it is
/// found: it cannot raise a maximum that has reached `floor`.
fn triangle_bound(triangle: Triangle, other: &Body<'_>, floor: f64) -> Result<f64, ProximityError> {
    let farthest = |target: Triangle| -> Result<f64, ProximityError> {
        let mut worst = 0.0_f64;
        for vertex in triangle {
            worst = worst.max(triangle_distance(vertex, target)?);
        }
        Ok(worst)
    };
    let [a, b, c] = triangle;
    let centre = (a + b + c) / 3.0;
    let first = other
        .soup
        .index
        .nearest_to(&Aabb::from_point(centre), |_| true)
        .ok_or(ProximityError::Unavailable)?;
    let mut best = farthest(other.soup.items[first.key])?;
    if best <= floor {
        return Ok(best);
    }
    // A triangle doing better comes within `best` of every vertex of this
    // one, so its box meets the box around each vertex grown by `best`:
    // it reaches up to the highest vertex less `best` and down to the lowest
    // plus `best` on every axis. Only those are measured.
    let (highest, lowest) = (a.max(b).max(c).to_array(), a.min(b).min(c).to_array());
    for index in other.soup.near(&triangle_box(&triangle).expanded(best)) {
        let candidate = &other.soup.boxes[index];
        let reaches = (0..3).all(|axis| {
            candidate.max()[axis] >= highest[axis] - best
                && candidate.min()[axis] <= lowest[axis] + best
        });
        if reaches {
            best = best.min(farthest(other.soup.items[index])?);
        }
    }
    Ok(best)
}

/// [`triangle_bound`], tightened by splitting the triangle while its bound
/// exceeds `floor`, a distance the answer is already known to reach.
fn refined_bound(
    triangle: Triangle,
    other: &Body<'_>,
    floor: f64,
    depth: u32,
) -> Result<f64, ProximityError> {
    let bound = triangle_bound(triangle, other, floor)?;
    if bound <= floor || depth == 0 {
        return Ok(bound);
    }
    let [a, b, c] = triangle;
    let (ab, bc, ca) = ((a + b) / 2.0, (b + c) / 2.0, (c + a) / 2.0);
    let mut split = 0.0_f64;
    for child in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
        split = split.max(refined_bound(child, other, floor, depth - 1)?);
        if split >= bound {
            return Ok(bound);
        }
    }
    Ok(split)
}

/// The Hausdorff distance between the two meshes' surfaces, as `(lower,
/// upper)`.
fn hausdorff(first: &Body<'_>, second: &Body<'_>) -> Result<(f64, f64), ProximityError> {
    let mut lower = 0.0_f64;
    for (from, to) in [(first, second), (second, first)] {
        for vertex in vertices(from) {
            lower = lower.max(surface_distance_above(vertex, to, lower)?);
        }
    }
    let mut upper = lower;
    for (from, to) in [(first, second), (second, first)] {
        for triangle in &from.soup.items {
            upper = upper.max(refined_bound(*triangle, to, upper, HAUSDORFF_REFINEMENT)?);
        }
    }
    let snap = |value: f64| {
        if value <= LINEAR_TOLERANCE {
            0.0
        } else {
            value
        }
    };
    Ok((snap(lower), snap(upper)))
}

impl ProximityService for AxiolidProximityService {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let body = self.body(object)?;
        ObjectBounds::try_new(
            object.clone(),
            body.soup.bounds,
            self.geometry.fidelity(object)?,
        )
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        if request.projection() != ProximityProjection::Minimum3d {
            return Err(ProximityError::UnsupportedProjection);
        }
        let subject = self.body(request.subject())?;
        let counterpart = self.body(request.counterpart())?;
        let fidelity = self
            .geometry
            .fidelity(request.subject())?
            .combined(self.geometry.fidelity(request.counterpart())?);

        let separation = separation(&subject.soup, &counterpart.soup)?;
        let plan_overlap =
            plan_overlap_area(&subject.soup.items, &counterpart.soup.items, tolerance()?)
                .ok_or(ProximityError::Unavailable)?;

        let (penetration, containment) = penetration(&subject, &counterpart, separation)?;
        let subject_fidelity = self.geometry.fidelity(request.subject())?;
        let counterpart_fidelity = self.geometry.fidelity(request.counterpart())?;
        let extents = match penetration {
            Some(_) => Some(overlap_extents(
                &subject,
                &counterpart,
                separation,
                containment,
                subject_fidelity,
                counterpart_fidelity,
            )?),
            None => None,
        };
        let deviation = fidelity.deviation_metres();
        let (lower, upper) = hausdorff(&subject, &counterpart)?;
        let hausdorff = LengthInterval::try_new((lower - deviation).max(0.0), upper + deviation)
            .map_err(|_| ProximityError::InvalidMeasurement)?;

        let measured = ProximityEvidence::try_new(
            request.clone(),
            separation,
            penetration,
            plan_overlap,
            containment,
            fidelity,
            Evidence {
                source: request.subject().source.clone(),
                locator: format!(
                    "axiolid:proximity:{}:{}",
                    request.subject(),
                    request.counterpart()
                ),
                exact: fidelity.is_exact(),
            },
        )?
        .with_hausdorff(hausdorff)?;
        match extents {
            Some(extents) => measured.with_overlap_extents(extents),
            None => Ok(measured),
        }
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let subject = self.body(request.subject())?;
        let counterpart = self.body(request.counterpart())?;
        let fidelity = self
            .geometry
            .fidelity(request.subject())?
            .combined(self.geometry.fidelity(request.counterpart())?);
        let (lower, upper) = self.projected_interval(request, &subject, &counterpart)?;
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            lower,
            upper,
            fidelity,
            Evidence {
                source: request.subject().source.clone(),
                locator: match request.projection() {
                    ProximityProjection::Vertical {
                        direction: direction @ (VerticalDirection::Above | VerticalDirection::Below),
                        ..
                    } => format!(
                        "axiolid:distance:vertical-{}:{}:{}",
                        direction.name(),
                        request.subject(),
                        request.counterpart()
                    ),
                    projection => format!(
                        "axiolid:distance:{}:{}:{}",
                        projection.name(),
                        request.subject(),
                        request.counterpart()
                    ),
                },
                exact: fidelity.is_exact(),
            },
        )
    }
}

#[cfg(test)]
mod tests {

    use axioval_ir::SourceId;

    use super::*;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    /// A closed prism around `axis` with `sides` chords and `rings` bands.
    fn column(centre: [f64; 3], radius: f64, height: f64, sides: u32, rings: u32) -> TriMesh {
        let mut positions = Vec::new();
        for ring in 0..=rings {
            let z = centre[2] + height * f64::from(ring) / f64::from(rings);
            for side in 0..sides {
                let angle = std::f64::consts::TAU * f64::from(side) / f64::from(sides);
                positions.push(Point3::new(
                    centre[0] + radius * angle.cos(),
                    centre[1] + radius * angle.sin(),
                    z,
                ));
            }
        }
        let bottom = u32::try_from(positions.len()).unwrap();
        positions.push(Point3::new(centre[0], centre[1], centre[2]));
        positions.push(Point3::new(centre[0], centre[1], centre[2] + height));
        let mut indices = Vec::new();
        for ring in 0..rings {
            for side in 0..sides {
                let next = (side + 1) % sides;
                let (a, b) = (ring * sides + side, ring * sides + next);
                let (c, d) = (a + sides, b + sides);
                indices.extend([a, b, d, a, d, c]);
            }
        }
        for side in 0..sides {
            let next = (side + 1) % sides;
            indices.extend([bottom, next, side]);
            let top = rings * sides;
            indices.extend([bottom + 1, top + side, top + next]);
        }
        TriMesh::new(positions, indices)
    }

    fn brute_separation(first: &Body<'_>, second: &Body<'_>) -> f64 {
        let mut best = f64::INFINITY;
        for a in &first.soup.items {
            for b in &second.soup.items {
                let pair = closest_points_on_triangles(*a, *b).unwrap();
                best = best.min(pair.distance_squared.sqrt());
            }
        }
        best
    }

    /// The index may only skip work, never change the answer.
    #[test]
    fn indexed_separation_matches_the_exhaustive_scan() {
        for (offset, height) in [(0.55, 3.0), (1.3, 2.0), (0.9, 0.5)] {
            let geometry = AxiolidGeometry::new()
                .with_mesh(id("a"), column([0.0, 0.0, 0.0], 0.3, 3.0, 24, 6))
                .with_mesh(id("b"), column([offset, 0.2, 1.0], 0.25, height, 20, 5));
            let service = AxiolidProximityService::new(geometry);
            let (a, b) = (
                service.body(&id("a")).unwrap(),
                service.body(&id("b")).unwrap(),
            );
            let indexed = separation(&a.soup, &b.soup).unwrap();
            let exhaustive = brute_separation(&a, &b);
            let exhaustive = if exhaustive <= LINEAR_TOLERANCE {
                0.0
            } else {
                exhaustive
            };
            assert!(
                (indexed - exhaustive).abs() < 1e-12,
                "offset {offset}: {indexed} vs {exhaustive}"
            );
        }
    }

    #[test]
    fn indexed_surface_distance_matches_the_exhaustive_scan() {
        let geometry =
            AxiolidGeometry::new().with_mesh(id("a"), column([0.0, 0.0, 0.0], 0.3, 3.0, 24, 6));
        let service = AxiolidProximityService::new(geometry);
        let body = service.body(&id("a")).unwrap();
        for point in [
            Point3::new(0.0, 0.0, 1.5),
            Point3::new(0.1, -0.05, 0.2),
            Point3::new(2.0, 1.0, 4.0),
        ] {
            let exhaustive = body
                .soup
                .items
                .iter()
                .map(|t| {
                    closest_point_on_triangle(point, *t)
                        .unwrap()
                        .distance(point)
                })
                .fold(f64::INFINITY, f64::min);
            let indexed = surface_distance(point, &body).unwrap();
            assert!(
                (indexed - exhaustive).abs() < 1e-12,
                "{point}: {indexed} vs {exhaustive}"
            );
        }
    }
}
