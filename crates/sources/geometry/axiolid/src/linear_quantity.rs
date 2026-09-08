//! Shelf running-length measurement over supplied geometry.
//!
//! ADR 0004: this module **measures**. It reports how many running metres of
//! shelving fit an object's usable wall, in the arrangement the caller
//! describes. Whether that clears a required minimum is the capability's
//! decision, not this module's.

use axiolid_core::Point3;
use axiolid_mesh::TriangleMeshView;
use axioval_engine::{
    LinearInterval, LinearQuantityError, LinearQuantityEvidence, LinearQuantityKind,
    LinearQuantityRequest, LinearQuantityService, ShelfGeometry,
};
use axioval_ir::{Evidence, SourceId};

use crate::geometry::AxiolidGeometry;

/// Measures shelf capacity from application-supplied meshes.
pub struct AxiolidLinearQuantityService {
    geometry: AxiolidGeometry,
    source: SourceId,
}

impl AxiolidLinearQuantityService {
    /// Binds a geometry set to the source that identifies its objects.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self { geometry, source }
    }
}

/// Footprint extent of a mesh in plan, as `(min_x, max_x, min_y, max_y)`.
fn footprint(mesh: &impl TriangleMeshView) -> Option<(f64, f64, f64, f64)> {
    let count = mesh.position_count();
    if count == 0 {
        return None;
    }
    let point = |i: usize| -> Point3 { mesh.position(i) };
    let mut bounds = {
        let p = point(0);
        (p.x, p.x, p.y, p.y)
    };
    for i in 1..count {
        let p = point(i);
        bounds.0 = bounds.0.min(p.x);
        bounds.1 = bounds.1.max(p.x);
        bounds.2 = bounds.2.min(p.y);
        bounds.3 = bounds.3.max(p.y);
    }
    Some(bounds)
}

/// Vertical extent of a mesh, as `(min_z, max_z)`.
fn elevation_span(mesh: &impl TriangleMeshView) -> Option<(f64, f64)> {
    let count = mesh.position_count();
    if count == 0 {
        return None;
    }
    let mut span = {
        let z = mesh.position(0).z;
        (z, z)
    };
    for i in 1..count {
        let z = mesh.position(i).z;
        span.0 = span.0.min(z);
        span.1 = span.1.max(z);
    }
    Some(span)
}

/// Running metres of shelving that fit one room, for one arrangement.
///
/// Shelving is placed against the perimeter, stacked in tiers. The wall a
/// shelf stands against must be deep enough to hold it, and each doorway
/// consumes clearance that cannot carry a run. Returned as an interval
/// because the perimeter is measured from a bounding footprint: the true
/// usable wall of a non-rectangular room cannot exceed it.
fn shelf_running_length(
    mesh: &impl TriangleMeshView,
    shelf: ShelfGeometry,
    doorways: usize,
) -> Option<(f64, f64)> {
    let (min_x, max_x, min_y, max_y) = footprint(mesh)?;
    let (floor, ceiling) = elevation_span(mesh)?;
    let (width, depth) = (max_x - min_x, max_y - min_y);
    if width <= 0.0 || depth <= 0.0 {
        return None;
    }

    // A room narrower than twice the shelf depth cannot take shelving on
    // facing walls without the runs colliding.
    let usable_walls = |extent: f64, across: f64| -> f64 {
        if across < shelf.depth_metres() {
            0.0
        } else if across < 2.0 * shelf.depth_metres() {
            extent
        } else {
            2.0 * extent
        }
    };
    let perimeter = usable_walls(width, depth) + usable_walls(depth, width);

    // Doorways interrupt a run: each consumes its clearance from the wall.
    // A doorway count beyond f64's exact integer range is not a real
    // building; saturating keeps the cast lossless for every real input.
    let obstructed =
        f64::from(u32::try_from(doorways).unwrap_or(u32::MAX)) * shelf.door_clearance_metres();
    let usable = (perimeter - obstructed).max(0.0);

    // Tiers are whole shelves only; a partial tier holds nothing.
    let headroom = shelf.top_elevation_metres().min(ceiling - floor);
    let stack = headroom - shelf.bottom_elevation_metres();
    if stack < 0.0 {
        return Some((0.0, 0.0));
    }
    let tiers = (stack / shelf.vertical_spacing_metres()).floor().max(0.0);

    // Horizontal spacing is the pitch between uprights: a run shorter than one
    // pitch carries no shelf.
    let runs = (usable / shelf.horizontal_spacing_metres())
        .floor()
        .max(0.0);
    let upper = runs * shelf.horizontal_spacing_metres() * tiers;
    Some((0.0, upper))
}

impl LinearQuantityService for AxiolidLinearQuantityService {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        let mesh = self
            .geometry
            .mesh(request.scope())
            .ok_or(LinearQuantityError::Unavailable)?;
        // The kind enum is non-exhaustive: a kind added upstream must fail
        // closed here rather than being silently measured as shelving.
        let LinearQuantityKind::ShelfRunningLength(arrangement) = request.kind() else {
            return Err(LinearQuantityError::Unavailable);
        };

        let doorways = self.geometry.doorway_count(request.scope());
        let (lower, upper) = shelf_running_length(mesh, arrangement, doorways)
            .ok_or(LinearQuantityError::InvalidGeometry)?;

        // The bound is derived from a bounding footprint, so it is an upper
        // limit rather than a single value: reporting it as exact would assert
        // more than the geometry supports.
        let measured = LinearInterval::try_new(lower, upper)?;
        LinearQuantityEvidence::try_new(
            request.clone(),
            measured,
            Evidence::exact(
                self.source.clone(),
                format!("axiolid:shelf:{}", request.scope().local_id),
            ),
        )
    }
}
