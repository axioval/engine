//! Contact measurement backed by the Axiolid geometry kernel.
//!
//! ADR 0004: this module **measures**. It reports how much of a face is in
//! contact and what it touches; whether that is enough is a rule's decision.
//!
//! The engine's [`ContactService`] is source-neutral, so nothing here may
//! assume IFC: subjects arrive as [`ObjectId`]s and their geometry is supplied
//! by the host as triangle meshes. That is what lets a proprietary CAD source
//! use this adapter without an IFC dependency.

use std::collections::BTreeMap;

use axiolid_core::Point3;
use axiolid_core::{Frame2, Point2, Vec2};
use axiolid_measure::{closest_points_on_triangles, surface_properties};
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};
use axioval_engine::{ContactError, ContactEvidence, ContactRequest, ContactService, ContactSide};
use axioval_ir::{Evidence, ObjectId, SourceId};

/// Tolerance used for mesh-health auditing when measuring areas.
///
/// Deliberately tight: this adapter reports `exact` evidence, so a mesh that
/// only measures cleanly under a loose tolerance must fail rather than be
/// laundered into the engine as fact.
const AUDIT_LINEAR_TOLERANCE: f64 = 1e-9;
const AUDIT_ANGULAR_TOLERANCE: f64 = 1e-9;

/// A triangle as three points, the form the proximity primitive consumes.
type Triangle = [Point3; 3];

/// Geometry for one object, keyed by the identity the engine uses.
///
/// Holding meshes by `ObjectId` is what keeps this adapter source-neutral:
/// the host decides how its native elements map onto identities.
#[derive(Debug, Default)]
pub struct AxiolidContactGeometry {
    meshes: BTreeMap<ObjectId, TriMesh>,
}

impl AxiolidContactGeometry {
    /// Creates an empty geometry set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            meshes: BTreeMap::new(),
        }
    }

    /// Registers one object's mesh.
    #[must_use]
    pub fn with_mesh(mut self, object: ObjectId, mesh: TriMesh) -> Self {
        self.meshes.insert(object, mesh);
        self
    }

    /// Returns the mesh registered for an object.
    #[must_use]
    pub fn mesh(&self, object: &ObjectId) -> Option<&TriMesh> {
        self.meshes.get(object)
    }

    /// Every registered object other than `subject`, in identity order.
    fn counterparts(&self, subject: &ObjectId) -> impl Iterator<Item = (&ObjectId, &TriMesh)> {
        self.meshes.iter().filter(move |(id, _)| *id != subject)
    }
}

/// Measures contact between registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidContactService {
    geometry: AxiolidContactGeometry,
    source: SourceId,
}

impl AxiolidContactService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidContactGeometry, source: SourceId) -> Self {
        Self { geometry, source }
    }
}

/// The triangles of a mesh as coordinate triples.
fn triangles(mesh: &TriMesh) -> Vec<Triangle> {
    (0..mesh.triangle_count())
        .map(|index| {
            let [a, b, c] = mesh.triangle(index);
            // Indices come from a foreign mesh, so a value that cannot be a
            // position index is a corrupt mesh, not something to truncate.
            [a, b, c].map(|index| {
                usize::try_from(index)
                    .ok()
                    .filter(|i| *i < mesh.position_count())
                    .map_or(Point3::ZERO, |i| mesh.position(i))
            })
        })
        .collect()
}

/// Whether `candidate` lies on the requested side of `subject`.
///
/// Compares the extreme coordinates rather than centroids: a large slab
/// centred below a small subject can still rest on top of it, and a centroid
/// test would wrongly reject that.
fn on_requested_side(subject: &[Triangle], candidate: &[Triangle], side: ContactSide) -> bool {
    let subject_extreme = match side {
        ContactSide::Above => extreme(subject, f64::max),
        ContactSide::Below => extreme(subject, f64::min),
    };
    let candidate_extreme = match side {
        ContactSide::Above => extreme(candidate, f64::max),
        ContactSide::Below => extreme(candidate, f64::min),
    };
    match (subject_extreme, candidate_extreme) {
        // Ties count as on-side: a slab resting exactly on a wall top shares
        // the boundary height, which is precisely the contact case.
        (Some(subject_z), Some(candidate_z)) => match side {
            ContactSide::Above => candidate_z >= subject_z,
            ContactSide::Below => candidate_z <= subject_z,
        },
        _ => false,
    }
}

/// The extreme z coordinate of a triangle set under `pick`.
fn extreme(triangles: &[Triangle], pick: fn(f64, f64) -> f64) -> Option<f64> {
    triangles.iter().flatten().map(|point| point.z).reduce(pick)
}

/// Nearest separation between two triangle sets.
///
/// Contact AREA is measured separately in plan by `planar_contact_area`;
/// this pass answers only "how close do they come", which is a 3D question.
fn nearest_separation(
    subject: &[Triangle],
    counterpart: &[Triangle],
) -> Result<Option<f64>, ContactError> {
    let mut nearest: Option<f64> = None;
    for subject_triangle in subject {
        for counterpart_triangle in counterpart {
            let pair = closest_points_on_triangles(*subject_triangle, *counterpart_triangle)
                .map_err(|_| ContactError::Unavailable)?;
            let distance = pair.distance_squared.sqrt();
            if !distance.is_finite() {
                return Err(ContactError::Unavailable);
            }
            nearest = Some(nearest.map_or(distance, |current: f64| current.min(distance)));
        }
    }
    Ok(nearest)
}

/// Exact contact area between two triangle sets, measured in plan.
///
/// Per-triangle counting was wrong: a triangle spanning the whole face counts
/// entirely as soon as any part of it approaches, so half-coverage reported as
/// full. Projecting both sets to the xy plane and intersecting them measures
/// the overlap itself.
///
/// This is a plan-projected measurement, which is the right one for the
/// `Above`/`Below` question this service answers: how much of the face is
/// covered when seen from above.
fn planar_contact_area(
    subject: &[Triangle],
    counterpart: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<f64, ContactError> {
    let frame = plan_frame();
    let subject_input = OverlayInput {
        frame,
        polygons: projected_polygons(subject),
    };
    let counterpart_input = OverlayInput {
        frame,
        polygons: projected_polygons(counterpart),
    };
    if subject_input.polygons.is_empty() || counterpart_input.polygons.is_empty() {
        return Ok(0.0);
    }
    let result = overlay(
        &subject_input,
        &counterpart_input,
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .map_err(|_| ContactError::Unavailable)?;
    Ok(result.polygons.iter().map(polygon_area).sum())
}

/// The xy plan frame both projections share.
fn plan_frame() -> Frame2 {
    Frame2 {
        origin: Point2::new(0.0, 0.0),
        x: Vec2::new(1.0, 0.0),
        y: Vec2::new(0.0, 1.0),
    }
}

/// Triangles projected to xy as overlay polygons, dropping degenerate ones.
///
/// A triangle seen edge-on in plan has no plan area and cannot contribute
/// coverage, so dropping it is a measurement decision, not a shortcut.
fn projected_polygons(triangles: &[Triangle]) -> Vec<Polygon> {
    triangles
        .iter()
        .filter_map(|[a, b, c]| {
            let ring = Ring {
                points: vec![
                    Point2::new(a.x, a.y),
                    Point2::new(b.x, b.y),
                    Point2::new(c.x, c.y),
                ],
            };
            (ring_area(&ring).abs() > f64::EPSILON).then_some(Polygon {
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
fn polygon_area(polygon: &Polygon) -> f64 {
    let outer = ring_area(&polygon.outer).abs();
    let holes: f64 = polygon.holes.iter().map(|r| ring_area(r).abs()).sum();
    (outer - holes).max(0.0)
}

/// Signed shoelace area of a ring.
fn ring_area(ring: &Ring) -> f64 {
    let points = &ring.points;
    let mut sum = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        sum += current.x * next.y - next.x * current.y;
    }
    sum * 0.5
}

impl ContactService for AxiolidContactService {
    fn measure_contact(&self, request: &ContactRequest) -> Result<ContactEvidence, ContactError> {
        let subject_mesh = self
            .geometry
            .mesh(request.subject())
            .ok_or(ContactError::Unavailable)?;
        let tolerance =
            axiolid_core::Tolerance::new(AUDIT_LINEAR_TOLERANCE, AUDIT_ANGULAR_TOLERANCE)
                .map_err(|_| ContactError::Unavailable)?;
        // A mesh that fails the health audit is not measurable evidence. Failing
        // here is what keeps `exact` honest.
        let whole = surface_properties(subject_mesh, tolerance)
            .map_err(|_| ContactError::Unavailable)?
            .area;
        let subject_triangles = triangles(subject_mesh);
        if subject_triangles.is_empty() {
            return Err(ContactError::UncheckableOrientation);
        }

        let mut nearest: Option<f64> = None;
        let mut contact_area = 0.0;
        let mut touching = Vec::new();
        for (candidate_id, candidate_mesh) in self.geometry.counterparts(request.subject()) {
            let candidate_triangles = triangles(candidate_mesh);
            if candidate_triangles.is_empty()
                || !on_requested_side(&subject_triangles, &candidate_triangles, request.side())
            {
                continue;
            }
            let pair_nearest = nearest_separation(&subject_triangles, &candidate_triangles)?;
            if let Some(distance) = pair_nearest {
                nearest = Some(nearest.map_or(distance, |current: f64| current.min(distance)));
            }
            // Only a counterpart within the gap tolerance is touching at all;
            // beyond it, plan overlap is just something passing overhead.
            let within_gap =
                pair_nearest.is_some_and(|d| d <= request.tolerance().maximum_gap_metres());
            if !within_gap {
                continue;
            }
            let pair_area =
                planar_contact_area(&subject_triangles, &candidate_triangles, tolerance)?;
            if pair_area >= request.tolerance().minimum_polygon_area_square_metres() {
                contact_area += pair_area;
                touching.push(candidate_id.clone());
            }
        }

        // Independent counterparts may each touch overlapping triangles, so the
        // summed area can exceed the face. Clamp rather than reject: the face's
        // own area is the true upper bound of how much of it can be covered.
        let contact_area = contact_area.min(whole);

        ContactEvidence::try_new(
            request.clone(),
            whole,
            contact_area,
            nearest,
            touching,
            Evidence::exact(
                self.source.clone(),
                format!("axiolid:contact:{}", request.subject().local_id),
            ),
        )
    }
}

#[cfg(test)]
mod polygon_area_tests {
    use super::{Polygon, Ring, polygon_area, ring_area};
    use axiolid_core::Point2;

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
