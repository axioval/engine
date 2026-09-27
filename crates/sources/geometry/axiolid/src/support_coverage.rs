//! Whether the tops of some bodies hold a clearance footprint, in plan.
//!
//! A support's top is every upward-facing triangle of its mesh, clipped to
//! the request's elevation band: the part of the surface that lies between
//! the two elevations and that something could stand on. The footprint is
//! bounded exactly as for clearance (a box its rectangle, a cylinder's disc
//! between two polygons), and the part of a bound outside every top is the
//! overlay difference of the bound less the tops' projections, measured
//! directly as for containment.

use axioval_engine::{
    ClearanceRequest, FreeSpaceError, SupportCoverageEvidence, SupportCoverageOutcome,
    SupportCoverageRequest,
};
use axioval_ir::{Evidence, SourceId};

use axiolid_core::{Point2, Point3};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};

use crate::free_space::{AREA_EPSILON_M2, shape_footprint, tolerance};
use crate::geometry::{AxiolidGeometry, triangles};
use crate::planar::{plan_frame, polygon_area, ring_area};

/// The part of a triangle between two elevations: a convex polygon.
fn between(triangle: &[Point3; 3], from: f64, to: f64) -> Vec<Point3> {
    let mut part: Vec<Point3> = triangle.to_vec();
    for (level, above) in [(from, true), (to, false)] {
        let inside = |point: &Point3| {
            if above {
                point.z - level
            } else {
                level - point.z
            }
        };
        let mut kept = Vec::new();
        for (index, a) in part.iter().enumerate() {
            let b = &part[(index + 1) % part.len()];
            let (fa, fb) = (inside(a), inside(b));
            if fa >= 0.0 {
                kept.push(*a);
            }
            if (fa >= 0.0) != (fb >= 0.0) {
                let t = fa / (fa - fb);
                kept.push(Point3::new(
                    a.x + t * (b.x - a.x),
                    a.y + t * (b.y - a.y),
                    a.z + t * (b.z - a.z),
                ));
            }
        }
        part = kept;
        if part.len() < 3 {
            return Vec::new();
        }
    }
    part
}

/// Whether a triangle faces up: its normal has a positive vertical part.
fn faces_up([a, b, c]: &[Point3; 3]) -> bool {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) > 0.0
}

/// Answers a support-coverage request over `geometry`, citing `source`.
///
/// A support without a mesh, unmeasured or tessellated is refused: a
/// missing top would turn "supported" into "unsupported", and a
/// tessellation's top is only near the true one.
pub(crate) fn assess(
    geometry: &AxiolidGeometry,
    source: &SourceId,
    request: &SupportCoverageRequest,
) -> Result<SupportCoverageOutcome, FreeSpaceError> {
    let volume = ClearanceRequest::new(request.frame().clone(), request.shape(), Vec::new());
    let footprint = shape_footprint(&volume)?;
    let (from, to) = (request.from_metres(), request.to_metres());
    let mut tops = Vec::new();
    for support in request.supports() {
        if geometry.is_tessellated(support) {
            return Err(FreeSpaceError::Unavailable(format!(
                "support `{support}` is tessellated, so its top is only approximate"
            )));
        }
        let mesh = geometry
            .mesh(support)
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(support.clone())))?;
        for triangle in triangles(mesh) {
            if !faces_up(&triangle) {
                continue;
            }
            let part = between(&triangle, from, to);
            if part.is_empty() {
                continue;
            }
            let mut ring = Ring {
                points: part
                    .iter()
                    .map(|point| Point2::new(point.x, point.y))
                    .collect(),
            };
            let area = ring_area(&ring);
            if area.abs() <= f64::EPSILON {
                continue;
            }
            if area < 0.0 {
                ring.points.reverse();
            }
            tops.push(Polygon {
                outer: ring,
                holes: Vec::new(),
            });
        }
    }
    let tolerance = tolerance()?;
    let uncovered = |bound: &Polygon| -> Result<f64, FreeSpaceError> {
        if tops.is_empty() {
            return Ok(polygon_area(bound));
        }
        let rest = overlay(
            &OverlayInput {
                frame: plan_frame(),
                polygons: vec![bound.clone()],
            },
            &OverlayInput {
                frame: plan_frame(),
                polygons: tops.clone(),
            },
            OverlayOperation::Difference,
            FillRule::NonZero,
            tolerance,
        )
        .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {error:?}")))?;
        Ok(rest.polygons.iter().map(polygon_area).sum())
    };
    let evidence = || {
        let supports: Vec<String> = request.supports().iter().map(ToString::to_string).collect();
        SupportCoverageEvidence::try_new(
            request.clone(),
            Evidence::exact(
                source.clone(),
                format!(
                    "axiolid:support-coverage:{}:{from}..{to}",
                    supports.join(",")
                ),
            ),
        )
    };
    // The outer bound holds the footprint: covered wholly, so is the
    // footprint. The inner bound lies in it: some of it uncovered, some of
    // the footprint is.
    if uncovered(&footprint.outer)? <= AREA_EPSILON_M2 {
        return Ok(SupportCoverageOutcome::Supported(evidence()?));
    }
    if uncovered(&footprint.inner)? > AREA_EPSILON_M2 {
        return Ok(SupportCoverageOutcome::Unsupported(evidence()?));
    }
    Err(FreeSpaceError::Unavailable(
        "the edge of a support's top lies within the cylinder's approximation band".into(),
    ))
}
