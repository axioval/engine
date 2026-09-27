//! Lines of sight from an eye point to a model object, past blockers.
//!
//! ADR 0004: this module proves what is in view; how many targets must be,
//! and from where, is a rule's decision.
//!
//! The kernel's `line_of_sight` decides each question exactly. A **visible**
//! answer is one ray, checked with exact predicates, that crosses a target
//! triangle before any blocker triangle. A **hidden** answer covers every
//! ray to the target by single blocker pieces: a triangle, two coplanar
//! triangles of one mesh forming a convex quadrilateral (a wall face), or a
//! whole closed convex blocker (a column). A target covered only where two
//! separate blockers meet stays **undecided**; the kernel never guesses.
//!
//! Only exact meshes are looked at: a tessellated target or blocker could
//! be seen, or see past, where its mesh says otherwise, so the request
//! refuses once either could matter (a tessellated target's distance is
//! still measured, widened by its chord deviation). A blocker whose body was not measured may hide anything, so it
//! refuses too; a declared bodiless blocker hides nothing and is skipped. A
//! blocker whose box lies apart from the box holding the eye and the target
//! cannot meet any segment between them, so it is not handed over.
//!
//! The distance to the target is the least distance from the eye to its
//! triangles, widened by a rounding bound, so a target is left unlooked-at
//! only when it lies surely beyond the requested range.

use axiolid_core::Point3;
use axiolid_inspect::{Sight, line_of_sight};
use axiolid_measure::{closest_point_on_triangle, closest_points_on_segments};
use axiolid_mesh::TriMesh;
use axioval_engine::{SightError, SightEvidence, SightOutcome, SightRequest, SightService};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Extent, mesh_extent, triangles};

/// Relative rounding bound on a measured distance, and its absolute floor.
const DISTANCE_SLACK: f64 = 1e-9;

/// Assesses lines of sight over registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidSightService {
    geometry: AxiolidGeometry,
}

impl AxiolidSightService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self { geometry }
    }

    /// The mesh of an object asked about, or why it cannot be used.
    fn mesh(&self, object: &ObjectId, role: &str) -> Result<&TriMesh, SightError> {
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
            return Err(SightError::Unavailable(format!(
                "{role} {object} has a body that was not measured: {reason}"
            )));
        }
        self.geometry
            .mesh(object)
            .ok_or_else(|| SightError::UnknownObject(object.clone()))
    }

    /// Refuses a tessellation, whose mesh is not the object's shape.
    fn exact(&self, object: &ObjectId, role: &str) -> Result<(), SightError> {
        if self.geometry.is_tessellated(object) {
            return Err(SightError::Unavailable(format!(
                "{role} {object} is a tessellation, so its mesh is not its shape"
            )));
        }
        Ok(())
    }
}

impl SightService for AxiolidSightService {
    fn assess_sight(&self, request: &SightRequest) -> Result<SightEvidence, SightError> {
        let target = request.target();
        if self.geometry.has_no_body(target) {
            return Err(SightError::Unavailable(format!(
                "target {target} has no body to be seen"
            )));
        }
        let mesh = self.mesh(target, "target")?;
        let [x, y, z] = request.eye();
        let eye = Point3::new(x, y, z);
        let distance = nearest(eye, mesh).ok_or_else(|| {
            SightError::Unavailable(format!("target {target} has no triangles to be seen"))
        })?;
        // A tessellation lies within its chord deviation of the true surface.
        let deviation = self
            .geometry
            .fidelity(target)
            .map_err(|error| SightError::Unavailable(format!("target {target}: {error}")))?
            .deviation_metres();
        let slack = DISTANCE_SLACK * (1.0 + distance) + deviation;
        let bounds = ((distance - slack).max(0.0), distance + slack);
        let locator = |what: &str| {
            format!(
                "line-of-sight:({x},{y},{z}):{target}:{what}",
                x = request.eye()[0],
                y = request.eye()[1],
                z = request.eye()[2]
            )
        };
        let evidence = |what: &str| Evidence {
            source: target.source.clone(),
            locator: locator(what),
            exact: deviation <= 0.0,
        };
        if request
            .within_metres()
            .is_some_and(|range| bounds.0 > range)
        {
            return SightEvidence::try_new(target.clone(), bounds, None, evidence("beyond-range"));
        }
        self.exact(target, "target")?;
        let reach = mesh_extent(mesh)
            .map(|(min, max)| {
                (
                    [min[0].min(x), min[1].min(y), min[2].min(z)],
                    [max[0].max(x), max[1].max(y), max[2].max(z)],
                )
            })
            .ok_or_else(|| SightError::Unavailable(format!("target {target} has no positions")))?;
        let mut blockers: Vec<(&ObjectId, &TriMesh)> = Vec::new();
        for blocker in request.blockers() {
            if self.geometry.has_no_body(blocker) {
                continue;
            }
            let blocking = self.mesh(blocker, "blocker")?;
            if mesh_extent(blocking).is_some_and(|extent| !boxes_meet(&extent, &reach)) {
                continue;
            }
            self.exact(blocker, "blocker")?;
            blockers.push((blocker, blocking));
        }
        let meshes: Vec<&TriMesh> = blockers.iter().map(|(_, mesh)| *mesh).collect();
        let sight = line_of_sight(eye, mesh, &meshes).map_err(|error| {
            SightError::Unavailable(format!(
                "the view of {target} cannot be assessed: {error:?}"
            ))
        })?;
        let (outcome, what) = match sight {
            Sight::Visible { through, .. } => (
                SightOutcome::Visible {
                    through: [through.x, through.y, through.z],
                },
                "visible".to_owned(),
            ),
            Sight::Hidden { occluders } => {
                let occluders: Vec<ObjectId> = occluders
                    .iter()
                    .filter_map(|index| blockers.get(*index).map(|(id, _)| (*id).clone()))
                    .collect();
                let named = occluders
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                (
                    SightOutcome::Hidden { occluders },
                    format!("hidden-by:{named}"),
                )
            }
            _ => (SightOutcome::Undecided, "undecided".to_owned()),
        };
        SightEvidence::try_new(target.clone(), bounds, Some(outcome), evidence(&what))
    }
}

/// Whether two closed boxes share a point.
fn boxes_meet(a: &Extent, b: &Extent) -> bool {
    (0..3).all(|axis| a.0[axis] <= b.1[axis] && b.0[axis] <= a.1[axis])
}

/// The least distance from `point` to the mesh's triangles; `None` for a
/// mesh without triangles. A degenerate triangle is measured by its edges.
fn nearest(point: Point3, mesh: &TriMesh) -> Option<f64> {
    let mut best: Option<f64> = None;
    for triangle in triangles(mesh) {
        let distance = match closest_point_on_triangle(point, triangle) {
            Ok(closest) => Some((closest - point).length()),
            Err(_) => (0..3)
                .filter_map(|i| {
                    closest_points_on_segments([triangle[i], triangle[(i + 1) % 3]], [point, point])
                        .ok()
                        .map(|pair| pair.distance_squared.sqrt())
                })
                .reduce(f64::min),
        };
        if let Some(distance) = distance {
            best = Some(best.map_or(distance, |held| held.min(distance)));
        }
    }
    best
}
