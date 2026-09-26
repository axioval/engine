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
//! Winding numbers need an inside, so points are only tested against a closed
//! two-manifold mesh. A surface reaching into a solid is measured; two open
//! surfaces share no volume and report `None` rather than a depth of zero.

use axiolid_core::{Aabb, Point3, Ray3, Tolerance};
use axiolid_measure::{WindingMesh, closest_point_on_triangle, closest_points_on_triangles};
use axiolid_mesh::{TriMesh, audit_mesh};
use axiolid_ray_mesh::intersect_triangle;
use axiolid_spatial::{Bvh, SpatialItem};
use axioval_engine::{
    BodyContainment, Bounds3, ObjectBounds, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::plan_overlap_area;

/// Linear tolerance for mesh audits, overlay and crossing tests.
///
/// Also the distance below which surfaces are taken to meet: floating-point
/// closest points of two touching faces rarely come out exactly zero.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// A point is inside a closed body when its winding number reaches one half.
const INSIDE_WINDING: f64 = 0.5;

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
        let bounds = extent(&triangles)?;
        let boxes: Vec<Bounds3> = triangles.iter().map(triangle_box).collect();
        let index = Bvh::build(
            boxes
                .iter()
                .enumerate()
                .map(|(triangle, bounds)| SpatialItem::new(triangle, aabb(bounds))),
        );
        // Audited coordinates are finite, so every box is accepted; a rejected
        // one would be a triangle no query could ever find.
        if index.rejected_items() != 0 {
            return Err(ProximityError::Unavailable);
        }
        Ok(Body {
            mesh,
            boxes,
            index,
            triangles,
            bounds,
            solid: health.is_closed_two_manifold(),
        })
    }
}

struct Body<'a> {
    mesh: &'a TriMesh,
    triangles: Vec<Triangle>,
    boxes: Vec<Bounds3>,
    /// Triangle hierarchy, so a query touches only the triangles near it
    /// instead of scanning the whole mesh.
    index: Bvh<usize>,
    bounds: Bounds3,
    solid: bool,
}

impl Body<'_> {
    /// Indices of the triangles whose boxes meet `probe`, in index order.
    fn near(&self, probe: &Bounds3) -> Vec<usize> {
        let mut hits = Vec::new();
        self.index.query_aabb(&aabb(probe), &mut hits);
        let mut triangles: Vec<usize> = hits
            .into_iter()
            .filter_map(|hit| self.index.item(hit).map(|item| item.key))
            .collect();
        triangles.sort_unstable();
        triangles
    }
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

fn extent(triangles: &[Triangle]) -> Result<Bounds3, ProximityError> {
    let mut points = triangles.iter().flatten();
    let first = *points.next().ok_or(ProximityError::Unavailable)?;
    let (min, max) = points.fold((first, first), |(min, max), p| (min.min(*p), max.max(*p)));
    Bounds3::try_new(min.to_array(), max.to_array())
}

/// Shortest distance between two triangle sets.
///
/// Each triangle of `first` is measured only against the triangles of
/// `second` inside its box grown by the best distance so far. Anything outside
/// that box is farther than the best already found, so the skip loses nothing.
fn separation(first: &Body<'_>, second: &Body<'_>) -> Result<f64, ProximityError> {
    let mut best = f64::INFINITY;
    for (a, a_box) in first.triangles.iter().zip(&first.boxes) {
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
            let pair = closest_points_on_triangles(*a, second.triangles[index])
                .map_err(|_| ProximityError::Unavailable)?;
            let distance = pair.distance_squared.sqrt();
            if !distance.is_finite() {
                return Err(ProximityError::InvalidMeasurement);
            }
            best = best.min(distance);
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

/// Distance from a point to a triangle set's surface.
///
/// The triangle whose box is nearest bounds the answer from above; only
/// triangles within that bound can improve on it.
fn surface_distance(point: Point3, body: &Body<'_>) -> Result<f64, ProximityError> {
    let to = |index: usize| -> Result<f64, ProximityError> {
        closest_point_on_triangle(point, body.triangles[index])
            .map(|closest| closest.distance(point))
            .map_err(|_| ProximityError::Unavailable)
    };
    let nearest = body
        .index
        .nearest_to(&Aabb::from_point(point), |_| true)
        .ok_or(ProximityError::Unavailable)?;
    let mut best = to(nearest.key)?;
    let probe = Bounds3::try_new(point.to_array(), point.to_array())?.expanded(best);
    for index in body.near(&probe) {
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
    for [a, b, c] in &body.triangles {
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
    if segment.gap(&other.bounds) > 0.0 {
        return Ok(Vec::new());
    }
    let ray = Ray3 {
        origin: start,
        direction,
    };
    let mut crossings = vec![0.0, 1.0];
    for index in other.near(&segment) {
        let hit = intersect_triangle(&ray, other.triangles[index], tolerance, index)
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
            (other.bounds.min()[axis]..=other.bounds.max()[axis]).contains(&point[axis])
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
    inside(&winding, inner.triangles[0][0])
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

impl ProximityService for AxiolidProximityService {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let body = self.body(object)?;
        ObjectBounds::try_new(object.clone(), body.bounds, self.geometry.fidelity(object)?)
    }

    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        let subject = self.body(request.subject())?;
        let counterpart = self.body(request.counterpart())?;
        let fidelity = self
            .geometry
            .fidelity(request.subject())?
            .combined(self.geometry.fidelity(request.counterpart())?);

        let separation = separation(&subject, &counterpart)?;
        let plan_overlap =
            plan_overlap_area(&subject.triangles, &counterpart.triangles, tolerance()?)
                .ok_or(ProximityError::Unavailable)?;

        let (penetration, containment) = penetration(&subject, &counterpart, separation)?;

        ProximityEvidence::try_new(
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
        for a in &first.triangles {
            for b in &second.triangles {
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
            let indexed = separation(&a, &b).unwrap();
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
                .triangles
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
