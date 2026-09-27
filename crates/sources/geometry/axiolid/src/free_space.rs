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

use axiolid_core::{Point2, Point3};
use axiolid_measure::WindingMesh;
use axiolid_mesh::{TriMesh, audit_mesh};
use axioval_engine::{
    AreaInterval, ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest, ClearanceShape,
    CompleteClearanceEvidence, CompletePlacementEvidence, CompleteSupportEvidence,
    ContainmentOutcome, ContainmentRequest, FrameOffsetPlacement, FreeAreaEvidence,
    FreeAreaRequest, FreeSpaceError, FreeSpaceService, MetricDirection, MetricFrame, MetricPoint,
    ObstructionEvidence, PlacementDomain, PlacementOrientation, PlacementOutcome, PlacementRequest,
    PlacementShape, SupportedPlacement,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Extent, Triangle, extent_gap, mesh_extent, triangles};
use crate::placement::{self, Axis, Scene, Search, Window};
use crate::planar::{plan_frame, polygon_area, projected_polygons};
use crate::walkable::{band_footprint, trapezoids};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};

/// The audit tolerance every measurement here shares.
pub(crate) fn tolerance() -> Result<axiolid_core::Tolerance, FreeSpaceError> {
    axiolid_core::Tolerance::new(1.0e-9, 1.0e-9)
        .map_err(|error| FreeSpaceError::Unavailable(format!("tolerance: {error:?}")))
}

/// Areas below this are numerical dust, not real obstruction.
pub(crate) const AREA_EPSILON_M2: f64 = 1.0e-9;

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

/// Sides of the polygons that bound a cylinder's disc from inside and outside.
const DISC_SIDES: u32 = 64;

/// A clearance volume's plan footprint, bounded from both sides.
///
/// `inner` lies inside the true footprint and `outer` contains it, so an
/// obstacle meeting `inner` certainly obstructs, and one missing `outer`
/// certainly does not. For a box both are the exact rectangle along the
/// frame's axes; for a cylinder they are the inscribed and circumscribed
/// polygons of its disc.
pub(crate) struct Footprint {
    pub(crate) inner: Polygon,
    pub(crate) outer: Polygon,
}

fn ring(points: Vec<Point2>) -> Polygon {
    Polygon {
        outer: Ring { points },
        holes: Vec::new(),
    }
}

pub(crate) fn shape_footprint(request: &ClearanceRequest) -> Result<Footprint, FreeSpaceError> {
    shape_bounds(request, 0.0)
}

/// The footprint's bounds with every side moved inward by `inset` metres.
fn shape_bounds(request: &ClearanceRequest, inset: f64) -> Result<Footprint, FreeSpaceError> {
    let frame = request.frame();
    let [centre_x, centre_y, _] = frame.origin().coordinates_metres();
    let [rx, ry, rz] = frame.right().components();
    let [fx, fy, fz] = frame.forward().components();
    let [ux, uy, _] = frame.up().components();
    // A tilted frame has no plan rectangle; refuse rather than project it.
    if rz.abs() > 1.0e-12 || fz.abs() > 1.0e-12 || ux.abs() > 1.0e-12 || uy.abs() > 1.0e-12 {
        return Err(FreeSpaceError::Unavailable(
            "clearance frames must be upright".into(),
        ));
    }
    Ok(match request.shape() {
        ClearanceShape::Box(b) => {
            let (half_width, half_depth) = (
                b.width_metres() / 2.0 - inset,
                b.depth_metres() / 2.0 - inset,
            );
            let corner = |along: f64, across: f64| {
                Point2::new(
                    centre_x + along * rx + across * fx,
                    centre_y + along * ry + across * fy,
                )
            };
            let rectangle = ring(vec![
                corner(-half_width, -half_depth),
                corner(half_width, -half_depth),
                corner(half_width, half_depth),
                corner(-half_width, half_depth),
            ]);
            Footprint {
                inner: rectangle.clone(),
                outer: rectangle,
            }
        }
        ClearanceShape::Cylinder(cylinder) => {
            let sides = f64::from(DISC_SIDES);
            let polygon = |radius: f64| {
                ring(
                    (0..DISC_SIDES)
                        .map(|i| {
                            let angle = 2.0 * std::f64::consts::PI * f64::from(i) / sides;
                            Point2::new(
                                centre_x + radius * angle.cos(),
                                centre_y + radius * angle.sin(),
                            )
                        })
                        .collect(),
                )
            };
            // A regular polygon's sides lie `cos(π/n)` of its vertex radius
            // from its centre, so moving them in by `inset` takes
            // `inset / cos(π/n)` off the vertex radius.
            let apothem = (std::f64::consts::PI / sides).cos();
            let radius = cylinder.radius_metres();
            Footprint {
                inner: polygon(radius - inset / apothem),
                outer: polygon((radius - inset) / apothem),
            }
        }
    })
}

/// How far inside the volume an obstacle must reach to obstruct it.
///
/// The volume is tested shrunk by this much on every side, so an obstacle
/// resting on its base, standing against a side or touching its top leaves it
/// clear, and floating-point rounding (far below a micrometre at building
/// coordinates) cannot turn contact into an obstruction.
pub(crate) const CONTACT_TOLERANCE_M: f64 = 1.0e-6;

/// A convex prism: a convex plan polygon swept over `[bottom, top]`.
struct Prism {
    /// Unit plan normals of the polygon's edges with the polygon's own
    /// `(min, max)` projection on each.
    sides: Vec<([f64; 2], f64, f64)>,
    plan: Vec<[f64; 2]>,
    bottom: f64,
    top: f64,
    extent: Extent,
}

impl Prism {
    fn new(polygon: &Polygon, bottom: f64, top: f64) -> Self {
        let plan: Vec<[f64; 2]> = polygon.outer.points.iter().map(|p| [p.x, p.y]).collect();
        let sides = edge_normals(&plan)
            .map(|normal| {
                let (min, max) = project(&plan, normal);
                (normal, min, max)
            })
            .collect();
        let mut extent = (
            [f64::INFINITY, f64::INFINITY, bottom],
            [f64::NEG_INFINITY, f64::NEG_INFINITY, top],
        );
        for [x, y] in &plan {
            extent.0[0] = extent.0[0].min(*x);
            extent.0[1] = extent.0[1].min(*y);
            extent.1[0] = extent.1[0].max(*x);
            extent.1[1] = extent.1[1].max(*y);
        }
        Self {
            sides,
            plan,
            bottom,
            top,
            extent,
        }
    }

    /// Whether a triangle has a point in the (closed) prism.
    ///
    /// A point lies in the prism when its height lies in the band and its
    /// plan position in the polygon. So the triangle meets the prism exactly
    /// when its part inside the band, a convex polygon, meets the plan
    /// polygon in plan: two convex polygons, which meet unless an edge
    /// normal of one of them separates them.
    fn meets(&self, triangle: &Triangle) -> bool {
        let [lo, hi] = triangle_extent(triangle);
        if extent_gap(&self.extent, &(lo, hi), false) > 0.0 {
            return false;
        }
        let band = clip(&clip(triangle, |p| p.z - self.bottom), |p| self.top - p.z);
        if band.is_empty() {
            return false;
        }
        let part: Vec<[f64; 2]> = band.iter().map(|p| [p.x, p.y]).collect();
        let apart = |(lo_a, hi_a): (f64, f64), (lo_b, hi_b): (f64, f64)| hi_a < lo_b || hi_b < lo_a;
        if self
            .sides
            .iter()
            .any(|(normal, lo, hi)| apart(project(&part, *normal), (*lo, *hi)))
        {
            return false;
        }
        !edge_normals(&part)
            .any(|normal| apart(project(&part, normal), project(&self.plan, normal)))
    }
}

/// The unit normals of a closed plan polygon's edges, skipping zero-length
/// edges (a clipped triangle can repeat a point).
fn edge_normals(points: &[[f64; 2]]) -> impl Iterator<Item = [f64; 2]> + '_ {
    (0..points.len()).filter_map(|i| {
        let [ax, ay] = points[i];
        let [bx, by] = points[(i + 1) % points.len()];
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx.hypot(dy);
        (length > 0.0).then(|| [-dy / length, dx / length])
    })
}

/// `(min, max)` of the points' positions along `axis`.
fn project(points: &[[f64; 2]], axis: [f64; 2]) -> (f64, f64) {
    points
        .iter()
        .map(|[x, y]| x * axis[0] + y * axis[1])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        })
}

/// The part of a convex polygon where `side` is not negative.
fn clip(polygon: &[Point3], side: impl Fn(&Point3) -> f64) -> Vec<Point3> {
    let mut kept = Vec::with_capacity(polygon.len() + 1);
    for (i, a) in polygon.iter().enumerate() {
        let b = &polygon[(i + 1) % polygon.len()];
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            kept.push(*a);
        }
        if (sa < 0.0) != (sb < 0.0) {
            let t = sa / (sa - sb);
            kept.push(*a + (*b - *a) * t);
        }
    }
    kept
}

fn triangle_extent(triangle: &Triangle) -> [[f64; 3]; 2] {
    let [a, b, c] = triangle;
    [a.min(*b).min(*c).to_array(), a.max(*b).max(*c).to_array()]
}

/// The clearance volume as prisms, shrunk by [`CONTACT_TOLERANCE_M`].
///
/// `inner` lies inside the volume, so an obstacle sharing a point with it
/// shares interior with the volume. `outer` contains the shrunk volume, so an
/// obstacle missing it reaches no deeper than the tolerance. For a box they
/// are the same prism and `outer` is `None`.
struct Volume {
    inner: Prism,
    outer: Option<Prism>,
    centre: Point3,
}

fn clearance_volume(request: &ClearanceRequest) -> Result<Volume, FreeSpaceError> {
    let [x, y, z] = request.frame().origin().coordinates_metres();
    let (height, narrowest) = match request.shape() {
        ClearanceShape::Box(b) => (
            b.height_metres(),
            b.width_metres().min(b.depth_metres()) / 2.0,
        ),
        ClearanceShape::Cylinder(c) => (
            c.height_metres(),
            c.radius_metres() * (std::f64::consts::PI / f64::from(DISC_SIDES)).cos(),
        ),
    };
    if narrowest <= CONTACT_TOLERANCE_M || height <= 2.0 * CONTACT_TOLERANCE_M {
        return Err(FreeSpaceError::Unavailable(
            "the clearance volume is thinner than the contact tolerance".into(),
        ));
    }
    let bounds = shape_bounds(request, CONTACT_TOLERANCE_M)?;
    let (bottom, top) = (z + CONTACT_TOLERANCE_M, z + height - CONTACT_TOLERANCE_M);
    Ok(Volume {
        inner: Prism::new(&bounds.inner, bottom, top),
        outer: matches!(request.shape(), ClearanceShape::Cylinder(_))
            .then(|| Prism::new(&bounds.outer, bottom, top)),
        centre: Point3::new(x, y, z + height / 2.0),
    })
}

/// How an obstacle stands to the volume.
enum Reach {
    /// It shares interior with the volume.
    Obstructs,
    /// It reaches no deeper into the volume than the contact tolerance.
    Misses,
    /// It meets the cylinder's approximation band only.
    Undecided,
}

/// A point is inside a closed body when its winding number reaches one half.
const INSIDE_WINDING: f64 = 0.5;

/// Decides one obstacle against the volume.
///
/// Its surface meeting the inner prism is a witness: the surface bounds the
/// solid, so solid lies on both sides of the meeting point, inside the
/// volume. An obstacle wholly inside the volume is caught here too, since
/// its surface is. When the surface misses the inner prism, the prism is
/// wholly inside or wholly outside the solid, and the winding number at the
/// volume's centre, which lies at least the prism's inset half-width from
/// the surface, tells which. Only a closed, consistently and outward wound
/// mesh bounds a solid, so any other is refused when it comes near.
fn reach(
    volume: &Volume,
    object: &ObjectId,
    mesh: &TriMesh,
    tolerance: axiolid_core::Tolerance,
) -> Result<Reach, FreeSpaceError> {
    let reachable = volume.outer.as_ref().unwrap_or(&volume.inner).extent;
    match mesh_extent(mesh) {
        None => return Err(FreeSpaceError::MissingGeometry(Box::new(object.clone()))),
        // Wherever its solid is, it lies within its mesh's box.
        Some(extent) if extent_gap(&reachable, &extent, false) > 0.0 => return Ok(Reach::Misses),
        Some(_) => {}
    }
    let health = audit_mesh(mesh, tolerance);
    if !health.is_surface_usable() {
        return Err(FreeSpaceError::Unavailable(format!(
            "the mesh of obstacle {object} cannot be read"
        )));
    }
    if !health.is_closed_two_manifold() {
        return Err(FreeSpaceError::Unavailable(format!(
            "obstacle {object} is not a closed, consistently wound surface, so it bounds no \
             solid to test the clearance volume against"
        )));
    }
    let body = triangles(mesh);
    let origin = body[0][0];
    let signed_volume: f64 = body
        .iter()
        .map(|[a, b, c]| (*a - origin).dot((*b - origin).cross(*c - origin)))
        .sum();
    if signed_volume <= 0.0 {
        return Err(FreeSpaceError::Unavailable(format!(
            "obstacle {object} faces inward, so which side of it is solid is unknown"
        )));
    }
    if body.iter().any(|triangle| volume.inner.meets(triangle)) {
        return Ok(Reach::Obstructs);
    }
    let winding = WindingMesh::prepare(mesh, tolerance)
        .and_then(|prepared| prepared.winding_number(volume.centre))
        .map_err(|error| FreeSpaceError::Unavailable(format!("winding: {error}")))?;
    if winding.skipped_singular_triangles > 0 || winding.value <= -INSIDE_WINDING {
        return Err(FreeSpaceError::Unavailable(format!(
            "obstacle {object} does not decide whether it encloses the clearance volume"
        )));
    }
    if winding.value >= INSIDE_WINDING {
        return Ok(Reach::Obstructs);
    }
    match &volume.outer {
        Some(outer) if body.iter().any(|triangle| outer.meets(triangle)) => Ok(Reach::Undecided),
        _ => Ok(Reach::Misses),
    }
}

impl AxiolidFreeSpaceService {
    /// The searched footprint and what the obstacles occupy in the elevation
    /// band, and the floor elevation the shape stands on.
    ///
    /// The footprint is the union of the scope's and every merged scope's;
    /// merged scopes must share the scope's floor. An obstacle counts only
    /// by the part of its solid inside the open band, as in clearance: a
    /// closed body's band footprint is its boundary clipped to the band plus
    /// its section just above the band's bottom, so an L-shaped body
    /// contributes only its foot to a low band. Clipping a planar triangle to
    /// horizontal planes is exact.
    fn placement_scene(
        &self,
        request: &PlacementRequest,
        tolerance: axiolid_core::Tolerance,
    ) -> Result<(Scene, f64), FreeSpaceError> {
        let missing = |id: &ObjectId| FreeSpaceError::MissingGeometry(Box::new(id.clone()));
        let reach = match request.shape() {
            PlacementShape::Box { shape, .. } => {
                shape.width_metres().hypot(shape.depth_metres()) / 2.0
            }
            PlacementShape::Cylinder(c) => c.radius_metres(),
        };
        let mut floor = None;
        let mut scope_triangles = Vec::new();
        for scope in std::iter::once(request.scope()).chain(request.merged_scopes()) {
            let mesh = self.geometry.mesh(scope).ok_or_else(|| missing(scope))?;
            let (low, _) = mesh_extent(mesh).ok_or_else(|| missing(scope))?;
            match floor {
                None => floor = Some(low[2]),
                Some(first) if (low[2] - first).abs() <= FLOOR_AGREEMENT_METRES => {}
                Some(_) => {
                    return Err(FreeSpaceError::Unavailable(format!(
                        "merged scope {scope} stands on another floor than {}",
                        request.scope()
                    )));
                }
            }
            // Exact evidence: a tessellated scope, or a tessellated obstacle
            // whose true body could reach a placement, makes the verdict an
            // estimate.
            let extent = self
                .geometry
                .enclosing_extent(scope)
                .ok_or_else(|| missing(scope))?;
            if self.geometry.is_tessellated(scope)
                || self
                    .geometry
                    .tessellated_near(&extent, reach, true, |object| {
                        !request.obstacles().contains(object)
                    })
                    .is_some()
            {
                return Err(FreeSpaceError::InexactPlacementEvidence);
            }
            scope_triangles.extend(triangles(mesh));
        }
        let floor = floor.ok_or_else(|| missing(request.scope()))?;

        let scope = placement::footprint(&scope_triangles, tolerance)?;
        if scope.is_empty() {
            return Err(missing(request.scope()));
        }
        let band = request.effective_band();
        let (low, high) = (floor + band.from_metres(), floor + band.to_metres());
        let mut obstacle_rings = Vec::new();
        for obstacle in request.obstacles() {
            if self.geometry.has_no_body(obstacle) {
                continue;
            }
            // An obstacle without geometry could stand anywhere, so neither a
            // witness nor a proof of absence would be complete.
            let mesh = self
                .geometry
                .mesh(obstacle)
                .ok_or_else(|| missing(obstacle))?;
            let occupied =
                band_footprint(obstacle, mesh, low, high).map_err(FreeSpaceError::Unavailable)?;
            obstacle_rings.extend(trapezoids(&occupied).into_iter().map(|piece| piece.outer));
        }
        let scene = Scene {
            scope,
            obstacles: placement::union(&obstacle_rings, tolerance)?,
            tolerance,
            window: None,
        };
        Ok((scene, floor))
    }
}

/// Merged scopes whose floors differ by more than this are not one floor.
const FLOOR_AGREEMENT_METRES: f64 = 1.0e-9;

/// The offset box of a frame-offset domain on the floor at `floor`.
///
/// The anchor must be exactly upright, so the right and forward offsets of
/// a centre are its plan offsets and its up offset is the floor's height
/// above the anchor. A floor outside the up offsets is refused rather than
/// proven empty.
fn offset_window(
    offsets: &FrameOffsetPlacement,
    scope: &ObjectId,
    floor: f64,
) -> Result<Window, FreeSpaceError> {
    let anchor = offsets.anchor();
    let [rx, ry, rz] = anchor.right().components();
    let [fx, fy, fz] = anchor.forward().components();
    #[allow(clippy::float_cmp)]
    let upright = rz == 0.0 && fz == 0.0 && anchor.up().components() == [0.0, 0.0, 1.0];
    if !upright {
        return Err(FreeSpaceError::Unavailable(
            "a frame-offset anchor must be exactly upright".into(),
        ));
    }
    let [ax, ay, az] = anchor.origin().coordinates_metres();
    let up = offsets.up();
    let rise = floor - az;
    if rise < up.lower_metres() || rise > up.upper_metres() {
        return Err(FreeSpaceError::Unavailable(
            "the scope's floor lies outside the anchor's vertical offsets".into(),
        ));
    }
    let admitted = offsets.clone();
    let scope = scope.clone();
    let axes = (anchor.right(), anchor.forward(), anchor.up());
    Ok(Window {
        origin: Point2::new(ax, ay),
        right: Axis { x: rx, y: ry },
        forward: Axis { x: fx, y: fy },
        across: (
            offsets.right().lower_metres(),
            offsets.right().upper_metres(),
        ),
        along: (
            offsets.forward().lower_metres(),
            offsets.forward().upper_metres(),
        ),
        admits: Box::new(move |centre| {
            MetricPoint::try_new(scope.clone(), [centre.x, centre.y, floor])
                .ok()
                .and_then(|origin| MetricFrame::try_new(origin, axes.0, axes.1, axes.2).ok())
                .is_some_and(|frame| admitted.contains_frame(&frame))
        }),
    })
}

impl FreeSpaceService for AxiolidFreeSpaceService {
    fn assess_clearance(
        &self,
        request: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError> {
        let tolerance = tolerance()?;
        let [_, _, z] = request.frame().origin().coordinates_metres();
        let footprint = shape_footprint(request)?;
        let height = match request.shape() {
            ClearanceShape::Box(b) => b.height_metres(),
            ClearanceShape::Cylinder(c) => c.height_metres(),
        };

        // The volume's box. A tessellated obstacle whose true body could reach
        // it makes the verdict an estimate; this evidence is exact.
        let volume_box = footprint.outer.outer.points.iter().fold(
            (
                [f64::INFINITY, f64::INFINITY, z],
                [f64::NEG_INFINITY, f64::NEG_INFINITY, z + height],
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
            .tessellated_near(&volume_box, 0.0, false, |object| {
                !request.obstacles().contains(object)
            })
            .is_some()
        {
            return Err(FreeSpaceError::InexactObstructionEvidence);
        }

        let volume = clearance_volume(request)?;
        let mut blockers = Vec::new();
        let mut undecided = false;
        for obstacle in request.obstacles() {
            if self.geometry.has_no_body(obstacle) {
                continue;
            }
            // A named obstacle without geometry cannot be shown to be clear of
            // the volume, and `Clear` asserts that nothing obstructs it.
            let mesh = self
                .geometry
                .mesh(obstacle)
                .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(obstacle.clone())))?;
            match reach(&volume, obstacle, mesh, tolerance)? {
                Reach::Obstructs => blockers.push(obstacle.clone()),
                Reach::Undecided => undecided = true,
                Reach::Misses => {}
            }
        }

        if blockers.is_empty() && undecided {
            return Err(FreeSpaceError::Unavailable(
                "an obstacle lies within the cylinder's approximation band".into(),
            ));
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
        let tolerance = tolerance()?;
        // The scope's own floor is the only support measured: every centre
        // the search accepts keeps the whole base inside the scope
        // footprint. A merged search spans several floors, so no single
        // support holds its base.
        let own = |support: &SupportedPlacement| {
            support.support() == request.scope() && request.merged_scopes().is_empty()
        };
        let (support, offsets) = match request.domain() {
            PlacementDomain::Unconstrained => (None, None),
            PlacementDomain::FrameOffsets(offsets) => (None, Some(offsets)),
            PlacementDomain::Supported(support) if own(support) => (Some(support), None),
            PlacementDomain::SupportedFrameOffsets { support, offsets } if own(support) => {
                (Some(support), Some(offsets))
            }
            _ => {
                return Err(FreeSpaceError::Unavailable(
                    "placement is supported by the scope's own floor only; \
                     other supports and merged scopes with a support are not measured"
                        .into(),
                ));
            }
        };
        let (mut scene, floor) = self.placement_scene(request, tolerance)?;
        if let Some(offsets) = offsets {
            scene.window = Some(offset_window(offsets, request.scope(), floor)?);
        }

        let search = match request.shape() {
            PlacementShape::Cylinder(c) => placement::circle(&scene, c.radius_metres())?,
            PlacementShape::Box {
                shape,
                orientation: PlacementOrientation::Any,
            } => placement::any_rectangle(&scene, shape.width_metres(), shape.depth_metres())?,
            PlacementShape::Box {
                shape,
                orientation: PlacementOrientation::Fixed(frame),
            } => {
                let [rx, ry, rz] = frame.right().components();
                let [ux, uy, uz] = frame.up().components();
                if rz.abs() > 1.0e-12 || ux.abs() > 1.0e-12 || uy.abs() > 1.0e-12 || uz <= 0.0 {
                    return Err(FreeSpaceError::Unavailable(
                        "a fixed orientation must be upright".into(),
                    ));
                }
                placement::fixed_rectangle(
                    &scene,
                    shape.width_metres(),
                    shape.depth_metres(),
                    Axis { x: rx, y: ry },
                )?
            }
        };

        // Placement is evidence about the scope, so it cites the scope's own
        // source.
        let mut locator = format!("axiolid:placement:{}", request.scope().local_id);
        for merged in request.merged_scopes() {
            locator.push('+');
            locator.push_str(&merged.to_string());
        }
        let evidence = Evidence::exact(request.scope().source.clone(), locator);
        let (centre, right) = match search {
            Search::Nowhere => {
                return Ok(PlacementOutcome::NoPlacement(
                    CompletePlacementEvidence::try_new(request.clone(), evidence)?,
                ));
            }
            Search::Found { centre, right } => (centre, right),
        };
        let origin = MetricPoint::try_new(request.scope().clone(), [centre.x, centre.y, floor])
            .map_err(|e| FreeSpaceError::Unavailable(format!("witness: {e}")))?;
        // A fixed orientation's witness uses the requested axes themselves,
        // and a frame-offset witness the anchor's.
        let frame = if let Some(PlacementOrientation::Fixed(fixed)) = request.shape().orientation()
        {
            MetricFrame::try_new(origin, fixed.right(), fixed.forward(), fixed.up())?
        } else if let Some(offsets) = offsets {
            let anchor = offsets.anchor();
            MetricFrame::try_new(origin, anchor.right(), anchor.forward(), anchor.up())?
        } else {
            let forward = right.left();
            MetricFrame::try_new(
                origin,
                MetricDirection::try_new([right.x, right.y, 0.0])?,
                MetricDirection::try_new([forward.x, forward.y, 0.0])?,
                MetricDirection::try_new([0.0, 0.0, 1.0])?,
            )?
        };
        let found = match support {
            None => ClearancePlacementEvidence::try_new(request.clone(), frame, evidence)?,
            Some(support) => ClearancePlacementEvidence::try_new_supported(
                request.clone(),
                frame.clone(),
                CompleteSupportEvidence::try_new(
                    support.support().clone(),
                    frame,
                    0.0,
                    evidence.clone(),
                )?,
                evidence,
            )?,
        };
        Ok(PlacementOutcome::Found(found))
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
            if self.geometry.has_no_body(obstacle) {
                continue;
            }
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

    fn assess_containment(
        &self,
        request: &ContainmentRequest,
    ) -> Result<ContainmentOutcome, FreeSpaceError> {
        crate::containment::assess(&self.geometry, &self.source, request)
    }
}
