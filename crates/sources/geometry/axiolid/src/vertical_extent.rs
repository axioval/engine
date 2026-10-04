//! Bottom and top elevations of a registered mesh.
//!
//! ADR 0004: this module measures elevations; whether stacked slabs are
//! spaced correctly is a rule's decision.
//!
//! A planar mesh is the object's shape, so its lowest and highest points
//! measure exactly. A tessellated mesh lies within its declared chord
//! deviation `d` of the true surface, so each elevation is reported as the
//! interval `[z - d, z + d]` with approximate evidence, never as a point.
//!
//! The extent along any other direction projects the same positions onto
//! it; see [`VerticalExtentService::measure_directional_extent`] below.

use axiolid_core::Tolerance;
use axiolid_mesh::audit_mesh;
use axioval_engine::{
    DirectionalExtent, ElevationInterval, FaceNormals, MetricDirection, SurfaceFace,
    VerticalExtent, VerticalExtentError, VerticalExtentService,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};

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

/// The triangles of a measurable mesh, and its chord deviation when it is
/// a tessellation.
struct Body {
    soup: Vec<Triangle>,
    tessellation: Option<f64>,
    /// Whether the mesh closes a volume, so its faces have an outside.
    closed: bool,
}

impl AxiolidVerticalExtentService {
    fn body(&self, object: &ObjectId) -> Result<Body, VerticalExtentError> {
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
        let health = audit_mesh(mesh, tolerance);
        if soup.is_empty() || !health.is_surface_usable() {
            return Err(VerticalExtentError::Unavailable(format!(
                "the mesh of {object} cannot be measured"
            )));
        }
        let fidelity = self
            .geometry
            .fidelity(object)
            .map_err(|_| VerticalExtentError::InvalidMeasurement)?;
        // A tessellation is never exact, even with a zero declared deviation.
        let tessellation = (!fidelity.is_exact()).then(|| fidelity.deviation_metres());
        Ok(Body {
            soup,
            tessellation,
            closed: health.is_closed_two_manifold(),
        })
    }
}

/// `[value - margin, value + margin]`, or the point `value` without a
/// margin. A margin is widened to at least one ulp-scale step so a widened
/// value is never a point, even for a declared zero deviation.
fn widened(value: f64, margin: Option<f64>) -> Result<ElevationInterval, VerticalExtentError> {
    match margin {
        None => ElevationInterval::exact(value),
        Some(margin) => {
            let margin = margin.max(f64::EPSILON * value.abs().max(1.0));
            ElevationInterval::try_new(value - margin, value + margin)
        }
    }
}

impl VerticalExtentService for AxiolidVerticalExtentService {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let Body {
            soup, tessellation, ..
        } = self.body(object)?;
        let (bottom, top) =
            soup.iter()
                .flatten()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                    let z = point.to_array()[2];
                    (low.min(z), high.max(z))
                });
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!(
                "vertical-extent:{object}{}",
                self.geometry.union_note(&[object])
            ),
            exact: tessellation.is_none(),
        };
        VerticalExtent::try_new(
            object.clone(),
            widened(bottom, tessellation)?,
            widened(top, tessellation)?,
            evidence,
        )
    }

    /// Positions are the used mesh positions projected onto `direction`.
    ///
    /// Along a coordinate axis the projection reads one coordinate, so a
    /// planar mesh measures exactly. Along any other direction each
    /// projection is a rounded dot product: it is widened by a bound on
    /// that rounding and reported as approximate, never as a point it
    /// cannot vouch for. A tessellation widens further by its chord
    /// deviation, since the true surface lies within it of the mesh.
    fn measure_directional_extent(
        &self,
        object: &ObjectId,
        direction: MetricDirection,
    ) -> Result<DirectionalExtent, VerticalExtentError> {
        let Body {
            soup, tessellation, ..
        } = self.body(object)?;
        let axis = direction.components();
        // A unit direction with one non-zero component is exactly a
        // coordinate axis: its products and sums are exact.
        let on_axis = axis.iter().filter(|component| **component != 0.0).count() == 1;
        // Each position's projection lies in `[p - r, p + r]`, so the lowest
        // lies in `[min(p - r), min(p + r)]` and the highest likewise.
        let mut lowest = (f64::INFINITY, f64::INFINITY);
        let mut highest = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for point in soup.iter().flatten() {
            let p = point.to_array();
            let terms = [p[0] * axis[0], p[1] * axis[1], p[2] * axis[2]];
            let projected = terms[0] + terms[1] + terms[2];
            // Three products and two sums: the result lies within
            // 3u·Σ|terms| of the exact dot product (u = ε/2); 2ε covers it
            // and the rounding of the bound itself.
            let rounding = if on_axis {
                0.0
            } else {
                2.0 * f64::EPSILON * terms.iter().map(|term| term.abs()).sum::<f64>()
            };
            let (low, high) = (projected - rounding, projected + rounding);
            lowest = (lowest.0.min(low), lowest.1.min(high));
            highest = (highest.0.max(low), highest.1.max(high));
        }
        let margin = match (tessellation, on_axis) {
            (None, true) => None,
            (deviation, _) => Some(deviation.unwrap_or(0.0)),
        };
        let widen = |(low, high): (f64, f64)| match margin {
            None => ElevationInterval::exact(low),
            Some(margin) => {
                // Never a point, even with no deviation and no rounding.
                let margin = margin.max(f64::EPSILON * low.abs().max(high.abs()).max(1.0));
                ElevationInterval::try_new(low - margin, high + margin)
            }
        };
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!(
                "directional-extent:{object}:[{},{},{}]",
                axis[0], axis[1], axis[2]
            ),
            exact: margin.is_none(),
        };
        DirectionalExtent::try_new(
            object.clone(),
            direction,
            widen(lowest)?,
            widen(highest)?,
            evidence,
        )
    }

    /// The normals of the face's triangles; see [`crate::face_normals`].
    fn measure_face_normals(
        &self,
        object: &ObjectId,
        face: SurfaceFace,
    ) -> Result<FaceNormals, VerticalExtentError> {
        let Body {
            soup,
            tessellation,
            closed,
        } = self.body(object)?;
        crate::face_normals::measure(object, face, &soup, tessellation, closed)
    }
}
