//! Whether a clearance footprint lies inside a set of scopes, in plan.
//!
//! The footprint is bounded exactly as for clearance: a box is its exact
//! rectangle, a cylinder's disc lies between an inscribed and a
//! circumscribed polygon. The part of a bound outside every scope is the
//! overlay difference of the bound less the scopes' projected triangles:
//! measured directly, not as a small difference of two large areas.

use axioval_engine::{
    ClearanceRequest, ContainmentEvidence, ContainmentOutcome, ContainmentRequest, FreeSpaceError,
};
use axioval_ir::{Evidence, SourceId};

use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, overlay};

use crate::free_space::{AREA_EPSILON_M2, shape_footprint, tolerance};
use crate::geometry::{AxiolidGeometry, triangles};
use crate::planar::{plan_frame, polygon_area, projected_polygons};

/// Answers a containment request over `geometry`, citing `source`.
///
/// A scope without a body, unmeasured or tessellated is refused: a missing
/// scope would turn "inside" into "outside", and a tessellation's footprint
/// is only near the true one.
pub(crate) fn assess(
    geometry: &AxiolidGeometry,
    source: &SourceId,
    request: &ContainmentRequest,
) -> Result<ContainmentOutcome, FreeSpaceError> {
    let volume = ClearanceRequest::new(request.frame().clone(), request.shape(), Vec::new());
    let footprint = shape_footprint(&volume)?;
    let mut scopes = Vec::new();
    for scope in request.scopes() {
        if geometry.is_tessellated(scope) {
            return Err(FreeSpaceError::Unavailable(format!(
                "scope `{scope}` is tessellated, so its footprint is only approximate"
            )));
        }
        let mesh = geometry
            .mesh(scope)
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(scope.clone())))?;
        scopes.extend(projected_polygons(&triangles(mesh)));
    }
    let tolerance = tolerance()?;
    let outside = |bound: &Polygon| -> Result<f64, FreeSpaceError> {
        if scopes.is_empty() {
            return Ok(polygon_area(bound));
        }
        let rest = overlay(
            &OverlayInput {
                frame: plan_frame(),
                polygons: vec![bound.clone()],
            },
            &OverlayInput {
                frame: plan_frame(),
                polygons: scopes.clone(),
            },
            OverlayOperation::Difference,
            FillRule::NonZero,
            tolerance,
        )
        .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {error:?}")))?;
        Ok(rest.polygons.iter().map(polygon_area).sum())
    };
    let evidence = || {
        let scopes: Vec<String> = request.scopes().iter().map(ToString::to_string).collect();
        ContainmentEvidence::try_new(
            request.clone(),
            Evidence::exact(
                source.clone(),
                format!("axiolid:containment:{}", scopes.join(",")),
            ),
        )
    };
    // The outer bound holds the footprint: nothing of it outside means
    // nothing of the footprint is. The inner bound lies in the footprint:
    // some of it outside means some of the footprint is.
    if outside(&footprint.outer)? <= AREA_EPSILON_M2 {
        return Ok(ContainmentOutcome::Inside(evidence()?));
    }
    if outside(&footprint.inner)? > AREA_EPSILON_M2 {
        return Ok(ContainmentOutcome::Outside(evidence()?));
    }
    Err(FreeSpaceError::Unavailable(
        "a scope boundary lies within the cylinder's approximation band".into(),
    ))
}
