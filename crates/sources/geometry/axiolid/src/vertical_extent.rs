//! Bottom and top elevations of a registered mesh.
//!
//! ADR 0004: this module measures elevations; whether stacked slabs are
//! spaced correctly is a rule's decision.
//!
//! A planar mesh is the object's shape, so its lowest and highest points
//! measure exactly. A tessellated mesh lies within its declared chord
//! deviation `d` of the true surface, so each elevation is reported as the
//! interval `[z - d, z + d]` with approximate evidence, never as a point.

use axiolid_core::Tolerance;
use axiolid_mesh::audit_mesh;
use axioval_engine::{
    ElevationInterval, VerticalExtent, VerticalExtentError, VerticalExtentService,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, triangles};

/// Mesh audit tolerance, as tight as the other services'.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// Measures vertical extents of registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidVerticalExtentService {
    geometry: AxiolidGeometry,
}

impl AxiolidVerticalExtentService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self { geometry }
    }
}

impl VerticalExtentService for AxiolidVerticalExtentService {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        // A bodiless object has no bottom or top; an unmeasured one has
        // elevations nobody knows. Neither is zero.
        if self.geometry.has_no_body(object) {
            return Err(VerticalExtentError::Unavailable(format!(
                "{object} is declared to have no body"
            )));
        }
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
            return Err(VerticalExtentError::Unavailable(format!(
                "{object} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let tolerance = Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE)
            .map_err(|_| VerticalExtentError::Unavailable("invalid audit tolerance".into()))?;
        // Only positions a triangle uses belong to the body; a mesh with bad
        // indices or non-finite positions has no trustworthy extent.
        let soup = triangles(mesh);
        if soup.is_empty() || !audit_mesh(mesh, tolerance).is_surface_usable() {
            return Err(VerticalExtentError::Unavailable(format!(
                "the mesh of {object} cannot be measured"
            )));
        }
        let (bottom, top) =
            soup.iter()
                .flatten()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                    let z = point.to_array()[2];
                    (low.min(z), high.max(z))
                });
        let fidelity = self
            .geometry
            .fidelity(object)
            .map_err(|_| VerticalExtentError::InvalidMeasurement)?;
        let deviation = fidelity.deviation_metres();
        // A tessellation is never exact, even with a zero declared deviation.
        let tessellated = !fidelity.is_exact();
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!("vertical-extent:{object}"),
            exact: !tessellated,
        };
        let widen = |z: f64| {
            if tessellated {
                // Widen by at least one ulp-scale step so the interval is
                // never a point, even for a declared zero deviation.
                let margin = deviation.max(f64::EPSILON * z.abs().max(1.0));
                ElevationInterval::try_new(z - margin, z + margin)
            } else {
                ElevationInterval::exact(z)
            }
        };
        VerticalExtent::try_new(object.clone(), widen(bottom)?, widen(top)?, evidence)
    }
}
