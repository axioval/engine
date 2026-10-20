//! Whether a clearance footprint lies inside a set of scopes, in plan.
//!
//! The footprint is bounded exactly as for clearance: a box is its exact
//! rectangle, a cylinder's disc lies between an inscribed and a
//! circumscribed polygon. The part of a bound outside every scope is the
//! overlay difference of the bound less the scopes' projected triangles:
//! measured directly, not as a small difference of two large areas. A
//! shadow the overlay refuses (an upright face whose corners lie some ulps
//! apart in plan, #377) is left out of the scopes a bound must lie inside
//! and taken as a box around it into those it must leave
//! ([`crate::placement::bracketed_footprint`]).

use axioval_engine::{
    ClearanceRequest, ContainmentEvidence, ContainmentOutcome, ContainmentRequest, FreeSpaceError,
};
use axioval_ir::{Evidence, SourceId};

use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon};

use crate::free_space::{AREA_EPSILON_M2, shape_footprint, tolerance};
use crate::geometry::{AxiolidGeometry, triangles};
use crate::placement::sliver_box;
use crate::planar::{plan_frame, plan_overlay, polygon_area, projected_polygons, refused_ring};

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
    let tolerance = tolerance()?;
    // `sure` lies inside the scopes' footprint and `possible` holds it.
    let (mut sure, mut possible) = (Vec::new(), Vec::new());
    for scope in request.scopes() {
        if geometry.is_tessellated(scope) {
            return Err(FreeSpaceError::Unavailable(format!(
                "scope `{scope}` is tessellated, so its footprint is only approximate"
            )));
        }
        let mesh = geometry
            .mesh(scope)
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(scope.clone())))?;
        for polygon in projected_polygons(&triangles(mesh)) {
            if refused_ring(&polygon.outer, tolerance) {
                possible.push(Polygon {
                    outer: sliver_box(&polygon.outer),
                    holes: Vec::new(),
                });
            } else {
                sure.push(polygon.clone());
                possible.push(polygon);
            }
        }
    }
    let outside = |bound: &Polygon, scopes: &[Polygon]| outside(bound, scopes, tolerance);
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
    if outside(&footprint.outer, &sure)? <= AREA_EPSILON_M2 {
        return Ok(ContainmentOutcome::Inside(evidence()?));
    }
    if outside(&footprint.inner, &possible)? > AREA_EPSILON_M2 {
        return Ok(ContainmentOutcome::Outside(evidence()?));
    }
    Err(FreeSpaceError::Unavailable(
        "a scope boundary lies within the cylinder's approximation band".into(),
    ))
}

/// The area of `bound` outside the non-zero fill of `scopes`, measured as
/// the overlay's difference, which is held to the overlay budget (#432):
/// the scopes of a containment, the tops of a support coverage.
pub(crate) fn outside(
    bound: &Polygon,
    scopes: &[Polygon],
    tolerance: axiolid_core::Tolerance,
) -> Result<f64, FreeSpaceError> {
    if scopes.is_empty() {
        return Ok(polygon_area(bound));
    }
    let rest = plan_overlay(
        &OverlayInput {
            frame: plan_frame(),
            polygons: vec![bound.clone()],
        },
        &OverlayInput {
            frame: plan_frame(),
            polygons: scopes.to_vec(),
        },
        OverlayOperation::Difference,
        FillRule::NonZero,
        tolerance,
    )
    .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {}", error.described())))?;
    Ok(rest.polygons.iter().map(polygon_area).sum())
}

#[cfg(test)]
mod budget_tests {
    use axiolid_core::{Point2, Tolerance};
    use axiolid_overlay::{Polygon, Ring};
    use axioval_engine::FreeSpaceError;

    use crate::planar::budget::OVERLAY_PAIRS;
    use crate::planar::sliver_polygons;

    /// A footprint bound against scopes whose shadows are past the overlay
    /// budget is refused by name, never measured (#432).
    #[test]
    fn scopes_past_the_overlay_budget_are_refused_by_name() {
        let p = Point2::new;
        let bound = Polygon {
            outer: Ring {
                points: vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)],
            },
            holes: Vec::new(),
        };
        let scopes = sliver_polygons();
        let tolerance = Tolerance::new(1e-9, 1e-9).unwrap();
        let Err(FreeSpaceError::Unavailable(message)) = super::outside(&bound, &scopes, tolerance)
        else {
            panic!("measured past the budget");
        };
        assert!(
            message.ends_with(&format!("past the overlay budget of {OVERLAY_PAIRS} pairs")),
            "{message}"
        );
    }
}
