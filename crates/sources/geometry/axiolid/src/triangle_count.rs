//! Triangle counts of registered meshes.
//!
//! ADR 0004: this module counts; whether an element has too many polygons is
//! a rule's decision.
//!
//! The count is of the mesh the host registered, which is the host's
//! tessellation, not a polygon count the source states. A planar mesh is the
//! object's exact shape, so its count is reported with exact evidence; a
//! tessellation of curved faces is counted just as exactly, but its count
//! follows the host's chord budget, so the evidence says it is approximate.

use axiolid_core::Tolerance;
use axiolid_mesh::audit_mesh;
use axioval_engine::{TriangleCount, TriangleCountError, TriangleCountService};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::AxiolidGeometry;

/// Mesh audit tolerance, as tight as the other services'.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// Counts the triangles of registered meshes.
#[derive(Debug)]
pub struct AxiolidTriangleCountService {
    geometry: AxiolidGeometry,
}

impl AxiolidTriangleCountService {
    /// Creates a service over the supplied geometry, which may hold several
    /// sources: each count cites the counted object's own source.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self { geometry }
    }
}

impl TriangleCountService for AxiolidTriangleCountService {
    fn count_triangles(&self, object: &ObjectId) -> Result<TriangleCount, TriangleCountError> {
        let evidence = |exact: bool| Evidence {
            source: object.source.clone(),
            locator: format!(
                "triangle-count:{object}{}",
                self.geometry.body_note(&[object])
            ),
            exact,
        };
        // A bodiless object has no mesh, so none of its triangles; an
        // unmeasured one has a mesh nobody produced, whose count is unknown.
        if self.geometry.has_no_body(object) {
            return TriangleCount::try_new(object.clone(), 0, evidence(true));
        }
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
            return Err(TriangleCountError::Unavailable(format!(
                "{object} has a body that was not meshed: {reason}"
            )));
        }
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or_else(|| TriangleCountError::UnknownObject(object.clone()))?;
        let tolerance = Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE)
            .map_err(|_| TriangleCountError::Unavailable("invalid audit tolerance".into()))?;
        // Bad indices or non-finite positions: the triangles are not all
        // triangles of a body, so their number is not its count.
        if !audit_mesh(mesh, tolerance).is_surface_usable() {
            return Err(TriangleCountError::Unavailable(format!(
                "the mesh of {object} cannot be read"
            )));
        }
        let fidelity = self
            .geometry
            .fidelity(object)
            .map_err(|_| TriangleCountError::InvalidMeasurement)?;
        let triangles = u64::try_from(mesh.triangle_count())
            .map_err(|_| TriangleCountError::InvalidMeasurement)?;
        TriangleCount::try_new(object.clone(), triangles, evidence(fidelity.is_exact()))
    }
}
