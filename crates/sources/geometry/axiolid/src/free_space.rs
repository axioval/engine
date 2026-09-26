//! Free-space measurement over application-supplied geometry.
//!
//! ADR 0004: this module measures. It reports whether a clearance volume is
//! obstructed, where one fits, and how much unobstructed floor a scope has.
//! Whether any of that satisfies a policy is the capability's decision.
//!
//! Two of the three outcomes carry a COMPLETENESS claim: `Clear` asserts
//! nothing obstructs the volume, and `NoPlacement` asserts an exhaustive
//! search found nowhere it fits. Neither may be returned on a hunch -- an
//! adapter that cannot search exhaustively must say so instead.

use axiolid_core::Point2;
use axioval_engine::{
    AreaInterval, ClearanceOutcome, ClearanceRequest, ClearanceShape, CompleteClearanceEvidence,
    FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService, ObstructionEvidence,
    PlacementOutcome, PlacementRequest,
};
use axioval_ir::{Evidence, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{plan_frame, polygon_area, projected_polygons};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};

/// The audit tolerance every measurement here shares.
fn tolerance() -> Result<axiolid_core::Tolerance, FreeSpaceError> {
    axiolid_core::Tolerance::new(1.0e-9, 1.0e-9)
        .map_err(|error| FreeSpaceError::Unavailable(format!("tolerance: {error:?}")))
}

/// Areas below this are numerical dust, not real obstruction.
const AREA_EPSILON_M2: f64 = 1.0e-9;

/// Measures free space from application-supplied meshes.
pub struct AxiolidFreeSpaceService {
    geometry: AxiolidGeometry,
    source: SourceId,
}

impl AxiolidFreeSpaceService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self { geometry, source }
    }

    fn evidence(&self) -> Evidence {
        Evidence::exact(self.source.clone(), "axiolid:free-space")
    }
}

/// The plan footprint of a clearance shape centred on `(x, y)`.
///
/// A cylinder is approximated by its inscribed square in plan: understating
/// the footprint would let a volume "fit" where it does not, so the
/// circumscribed square is used instead and the fit is conservative.
fn shape_footprint(shape: ClearanceShape, x: f64, y: f64) -> Polygon {
    let (half_width, half_depth) = match shape {
        ClearanceShape::Box(b) => (b.width_metres() / 2.0, b.depth_metres() / 2.0),
        ClearanceShape::Cylinder(c) => (c.radius_metres(), c.radius_metres()),
    };
    Polygon {
        outer: Ring {
            points: vec![
                Point2::new(x - half_width, y - half_depth),
                Point2::new(x + half_width, y - half_depth),
                Point2::new(x + half_width, y + half_depth),
                Point2::new(x - half_width, y + half_depth),
            ],
        },
        holes: Vec::new(),
    }
}

/// Vertical span of a triangle set as `(min_z, max_z)`.
fn vertical_span(triangles: &[Triangle]) -> Option<(f64, f64)> {
    let mut span: Option<(f64, f64)> = None;
    for point in triangles.iter().flatten() {
        span = Some(match span {
            None => (point.z, point.z),
            Some((lo, hi)) => (lo.min(point.z), hi.max(point.z)),
        });
    }
    span
}

/// Plan area shared by a footprint polygon and a triangle set.
fn overlap_area(
    footprint: &Polygon,
    other: &[Triangle],
    tolerance: axiolid_core::Tolerance,
) -> Result<f64, FreeSpaceError> {
    let other_polygons = projected_polygons(other);
    if other_polygons.is_empty() {
        return Ok(0.0);
    }
    let frame = plan_frame();
    let result = overlay(
        &OverlayInput {
            frame,
            polygons: vec![footprint.clone()],
        },
        &OverlayInput {
            frame,
            polygons: other_polygons,
        },
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {error:?}")))?;
    Ok(result.polygons.iter().map(polygon_area).sum())
}

/// Height of the vertical overlap between two spans, zero when disjoint.
fn overlapping_height(first: (f64, f64), second: (f64, f64)) -> f64 {
    (first.1.min(second.1) - first.0.max(second.0)).max(0.0)
}

impl FreeSpaceService for AxiolidFreeSpaceService {
    fn assess_clearance(
        &self,
        request: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError> {
        let tolerance = tolerance()?;
        let [x, y, z] = request.frame().origin().coordinates_metres();
        let footprint = shape_footprint(request.shape(), x, y);
        let height = match request.shape() {
            ClearanceShape::Box(b) => b.height_metres(),
            ClearanceShape::Cylinder(c) => c.height_metres(),
        };
        let volume_span = (z, z + height);

        // The volume's box. A tessellated obstacle whose true body could reach
        // it makes the verdict an estimate; this evidence is exact.
        let volume = footprint.outer.points.iter().fold(
            (
                [f64::INFINITY, f64::INFINITY, volume_span.0],
                [f64::NEG_INFINITY, f64::NEG_INFINITY, volume_span.1],
            ),
            |(min, max), p| {
                (
                    [min[0].min(p.x), min[1].min(p.y), min[2]],
                    [max[0].max(p.x), max[1].max(p.y), max[2]],
                )
            },
        );
        if self
            .geometry
            .tessellated_near(&volume, 0.0, false, |object| {
                !request.obstacles().contains(object)
            })
            .is_some()
        {
            return Err(FreeSpaceError::InexactObstructionEvidence);
        }

        let mut blockers = Vec::new();
        for obstacle in request.obstacles() {
            // A named obstacle without geometry cannot be shown to be clear of
            // the volume, and `Clear` asserts that nothing obstructs it.
            let mesh = self
                .geometry
                .mesh(obstacle)
                .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(obstacle.clone())))?;
            let body = triangles(mesh);
            let Some(span) = vertical_span(&body) else {
                return Err(FreeSpaceError::MissingGeometry(Box::new(obstacle.clone())));
            };
            if overlapping_height(volume_span, span) <= 0.0 {
                continue;
            }
            if overlap_area(&footprint, &body, tolerance)? > AREA_EPSILON_M2 {
                blockers.push(obstacle.clone());
            }
        }

        if blockers.is_empty() {
            // Every named obstacle was measured and none intersects, so the
            // completeness claim is earned rather than assumed.
            return Ok(ClearanceOutcome::Clear(CompleteClearanceEvidence::try_new(
                request.clone(),
                self.evidence(),
            )?));
        }
        Ok(ClearanceOutcome::Obstructed(ObstructionEvidence::try_new(
            request.clone(),
            blockers,
            self.evidence(),
        )?))
    }

    fn find_placement(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementOutcome, FreeSpaceError> {
        // `NoPlacement` asserts an EXHAUSTIVE search: that the shape fits
        // nowhere in the scope. A sampled sweep cannot establish that -- it can
        // only fail to find a witness, which is a different statement. Rather
        // than launder "did not find" into "does not exist", this adapter
        // refuses the request it cannot answer completely.
        //
        // A finding witness would be sound to return, but returning `Found`
        // while being unable to ever return `NoPlacement` gives a capability a
        // one-sided answer that reads as a pass. Both arms need the same
        // search, so both wait for it.
        let _ = self.geometry.mesh(request.scope());
        Err(FreeSpaceError::Unavailable(
            "exhaustive placement search is not implemented; \
             a sampled sweep cannot establish that no placement exists"
                .to_string(),
        ))
    }

    fn measure_free_area(
        &self,
        request: &FreeAreaRequest,
    ) -> Result<FreeAreaEvidence, FreeSpaceError> {
        let tolerance = tolerance()?;
        let scope_mesh = self
            .geometry
            .mesh(request.scope())
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(request.scope().clone())))?;
        // Free area is exact evidence. A tessellated scope, or a tessellated
        // obstacle whose true footprint could reach it, makes it an estimate.
        let scope_extent = self
            .geometry
            .enclosing_extent(request.scope())
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(request.scope().clone())))?;
        if self.geometry.is_tessellated(request.scope())
            || self
                .geometry
                .tessellated_near(&scope_extent, 0.0, true, |object| {
                    !request.obstacles().contains(object)
                })
                .is_some()
        {
            return Err(FreeSpaceError::InexactAreaEvidence);
        }
        let scope = triangles(scope_mesh);
        let scope_polygons = projected_polygons(&scope);
        if scope_polygons.is_empty() {
            return Err(FreeSpaceError::MissingGeometry(Box::new(
                request.scope().clone(),
            )));
        }
        let frame = plan_frame();

        // Union the scope first: overlapping triangles of one body must not
        // count their shared area twice.
        let scope_input = OverlayInput {
            frame,
            polygons: scope_polygons,
        };
        let scope_union = overlay(
            &scope_input,
            &scope_input,
            OverlayOperation::Union,
            FillRule::NonZero,
            tolerance,
        )
        .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {error:?}")))?;
        let total: f64 = scope_union.polygons.iter().map(polygon_area).sum();

        // Obstacles are unioned too, so two overlapping obstacles do not
        // subtract their shared area twice and understate what is free.
        let mut obstacle_polygons = Vec::new();
        for obstacle in request.obstacles() {
            let mesh = self
                .geometry
                .mesh(obstacle)
                .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(obstacle.clone())))?;
            obstacle_polygons.extend(projected_polygons(&triangles(mesh)));
        }

        let obstructed = if obstacle_polygons.is_empty() {
            0.0
        } else {
            let obstacle_input = OverlayInput {
                frame,
                polygons: obstacle_polygons,
            };
            let merged = overlay(
                &obstacle_input,
                &obstacle_input,
                OverlayOperation::Union,
                FillRule::NonZero,
                tolerance,
            )
            .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {error:?}")))?;
            // Clip to the scope: an obstacle extending past the room does not
            // consume floor the room never had.
            overlay(
                &OverlayInput {
                    frame,
                    polygons: scope_union.polygons.clone(),
                },
                &OverlayInput {
                    frame,
                    polygons: merged.polygons,
                },
                OverlayOperation::Intersection,
                FillRule::NonZero,
                tolerance,
            )
            .map_err(|error| FreeSpaceError::Unavailable(format!("overlay: {error:?}")))?
            .polygons
            .iter()
            .map(polygon_area)
            .sum()
        };

        // Plan area is an UPPER bound on usable floor: it counts area a
        // mobility profile cannot actually occupy, such as a strip too narrow
        // to turn in. The lower bound stays 0 because this adapter does not
        // yet erode by the profile's footprint.
        let free = (total - obstructed).max(0.0);
        FreeAreaEvidence::try_new(
            request.clone(),
            AreaInterval::try_new(0.0, free)?,
            self.evidence(),
        )
    }
}
