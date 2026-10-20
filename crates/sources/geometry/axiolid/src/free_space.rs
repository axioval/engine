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

mod obstacles;
mod scenes;
use std::fmt::Write as _;
use std::sync::Arc;

use axiolid_core::{Point2, Point3, Vec2};
use axiolid_measure::WindingMesh;
use axiolid_mesh::{TriMesh, audit_mesh};
use axioval_engine::{
    AreaInterval, CirculationMap, CirculationRequest, ClearanceOutcome, ClearancePlacementEvidence,
    ClearanceRequest, ClearanceShape, CompleteClearanceEvidence, CompletePlacementEvidence,
    CompleteSupportEvidence, ContainmentOutcome, ContainmentRequest, DerivationStore,
    ElevationBand, FrameOffsetPlacement, FreeAreaEvidence, FreeAreaRequest, FreeFloorEvidence,
    FreeSpaceError, FreeSpaceService, LargestDiscEstimate, MetricDirection, MetricFrame,
    MetricPoint, ObstructionEvidence, PlacementDomain, PlacementOrientation, PlacementOutcome,
    PlacementRequest, PlacementShape, SupportedPlacement, SweptDoor,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Extent, Triangle, extent_gap, mesh_extent, triangles};
use crate::placement::{self, Axis, Scene, Search, Window};
use crate::planar::budget::Work;
use crate::planar::{enclosing_hull, plan_frame, polygon_area, projected_polygons};
use crate::walkable::{ON_SURFACE, Plan, band_footprint, solid_column, trapezoids};
use axiolid_overlay::{
    FillRule, MorphologyMode, OverlayInput, OverlayOperation, Polygon, Region, Ring, overlay,
};
use obstacles::{BandFootprints, Body, Column, ObstacleKey, SolidColumns};

/// The audit tolerance every measurement here shares.
pub(crate) fn tolerance() -> Result<axiolid_core::Tolerance, FreeSpaceError> {
    axiolid_core::Tolerance::new(1.0e-9, 1.0e-9)
        .map_err(|error| FreeSpaceError::Unavailable(format!("tolerance: {error:?}")))
}

/// Areas below this are numerical dust, not real obstruction.
pub(crate) const AREA_EPSILON_M2: f64 = 1.0e-9;

/// Measures free space from application-supplied meshes.
pub struct AxiolidFreeSpaceService {
    /// What the service measures over, shared with the placement scenes
    /// the check's derivation store keeps ([`scenes::SceneKey`]).
    over: Arc<Measuring>,
    source: SourceId,
}

/// The geometry a free-space service measures, and the obstacle lists its
/// scenes are keyed by. What it measures of the geometry (each obstacle's
/// band footprint and column, each scene) is kept in the check's
/// derivation store, keyed over this state ([`obstacles`], [`scenes`]),
/// never here (#443).
pub(crate) struct Measuring {
    geometry: AxiolidGeometry,
    /// The obstacle lists of the scenes keyed over this state, each once:
    /// part of their keys, not a cache of values.
    lists: scenes::Lists,
    /// Whether footprints, columns and scenes are read from the check's
    /// store, built once for every request asking for them
    /// ([`AxiolidFreeSpaceService::with_reuse`]).
    reuse: bool,
}

impl AxiolidFreeSpaceService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            over: Arc::new(Measuring {
                geometry,
                lists: scenes::Lists::default(),
                reuse: true,
            }),
            source,
        }
    }

    /// The same, keeping no footprint, column or placement scene for a
    /// later request unless `reuse` is set (on by default). Every answer
    /// is the same either way, since what a search may spend is a function
    /// of its request alone (#434); off, only to show that (#441).
    #[must_use]
    pub fn with_reuse(self, reuse: bool) -> Self {
        let over = match Arc::try_unwrap(self.over) {
            Ok(over) => Measuring { reuse, ..over },
            Err(shared) => Measuring {
                geometry: shared.geometry.clone(),
                lists: scenes::Lists::default(),
                reuse,
            },
        };
        Self {
            over: Arc::new(over),
            source: self.source,
        }
    }

    fn evidence(&self) -> Evidence {
        Evidence::exact(self.source.clone(), "axiolid:free-space")
    }

    pub(crate) fn geometry(&self) -> &AxiolidGeometry {
        &self.over.geometry
    }

    /// The placement scene of `scope` (with `merged`) less `obstacles` and
    /// the `swept` doors' sectors in `band`: the check's, built once for
    /// every rule and thread asking for it ([`scenes::PlacementScenes`],
    /// #408), or with reuse off, built for this request alone.
    pub(crate) fn derived_scene(
        &self,
        (scope, merged): (&ObjectId, &[ObjectId]),
        (obstacles, swept): (&[ObjectId], &[SweptDoor]),
        band: ElevationBand,
        tolerance: axiolid_core::Tolerance,
    ) -> Result<scenes::Derived, FreeSpaceError> {
        let key = scenes::SceneKey::new(
            &self.over,
            (scope, merged),
            (obstacles, swept),
            band,
            tolerance,
        );
        scenes::Derived::of(key, self.over.reuse)
    }

    /// The searched footprint and what the obstacles occupy in the elevation
    /// band of `request` ([`Measuring::floor_scene`]), derived once per
    /// check ([`Self::derived_scene`]), with what earlier searches of it
    /// took.
    fn placement_scene(
        &self,
        request: &PlacementRequest,
        tolerance: axiolid_core::Tolerance,
    ) -> Result<(Scene, f64, scenes::Derived), FreeSpaceError> {
        let derived = self.derived_scene(
            (request.scope(), request.merged_scopes()),
            (request.obstacles(), request.swept_doors()),
            request.effective_band(),
            tolerance,
        )?;
        let kept = derived.kept()?;
        Ok((kept.scene(), kept.floor, derived))
    }
}

impl Measuring {
    /// [`solid_column`] of `obstacle`'s `body` about `level`: the check's,
    /// built once ([`SolidColumns`]), or with reuse off built here.
    fn solid_column(
        self: &Arc<Self>,
        obstacle: &ObjectId,
        body: Body,
        level: f64,
        half: f64,
    ) -> Column {
        let key = ObstacleKey::new(self, obstacle, body, [level, half]);
        if !self.reuse {
            return key.column();
        }
        DerivationStore::derive_current::<SolidColumns>(&key)
            .map_err(|error| error.to_string())?
            .as_ref()
            .clone()
    }

    /// [`band_footprint`] of `obstacle`'s `body` in the open band
    /// `low < z < high`: the check's, built once ([`BandFootprints`]), or
    /// with reuse off built here.
    fn band_footprint(
        self: &Arc<Self>,
        obstacle: &ObjectId,
        body: Body,
        low: f64,
        high: f64,
    ) -> Result<Plan, String> {
        let key = ObstacleKey::new(self, obstacle, body, [low, high]);
        if !self.reuse {
            return key.footprint();
        }
        DerivationStore::derive_current::<BandFootprints>(&key)
            .map_err(|error| error.to_string())?
            .as_ref()
            .clone()
    }

    /// Why the footprints a scene takes of a measured obstacle (as
    /// [`Self::floor_scene`] takes them: in the band, and, for a tessellated
    /// body, its sure column) are refused, if they are; `None` when each
    /// is built, or a tessellated body's bounds leave it out of the scene
    /// either way. The footprints are the check's, so the scene reads them
    /// again.
    fn band_refusal(
        self: &Arc<Self>,
        obstacle: &ObjectId,
        d: f64,
        low: f64,
        high: f64,
        u: f64,
    ) -> Option<String> {
        if d == 0.0 {
            self.band_footprint(obstacle, Body::Mesh, low - u, high + u)
                .err()
                .or_else(|| {
                    (u != 0.0 && high - u > low + u)
                        .then(|| {
                            self.band_footprint(obstacle, Body::Mesh, low + u, high - u)
                                .err()
                        })
                        .flatten()
                })
        } else {
            let bounds = self.geometry.vertical_bounds(obstacle)?;
            if bounds.clear_of(low - u, high + u, ON_SURFACE) {
                return None;
            }
            self.band_footprint(obstacle, Body::Mesh, low - u - d, high + u + d)
                .err()
                .or_else(|| {
                    let level = low + u + 2.0 * d;
                    (level < high - u)
                        .then(|| self.solid_column(obstacle, Body::Mesh, level, d).err())
                        .flatten()
                })
        }
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

impl Measuring {
    /// The footprint of `scope` and `merged` (which must share its floor),
    /// what `obstacles` and the sectors of `swept` doors occupy in `band`
    /// above that floor, and the floor's elevation.
    ///
    /// A tessellated body joins the scene bracketed by its chord deviation
    /// `d` (#302): witnesses avoid what it may occupy (its band footprint
    /// in the band widened by `d`, grown by `d`, `Region::dilate_outer`)
    /// and absence is proven against what it surely occupies (the plan
    /// points whose vertical column of height `2d`, a deviation above the
    /// band's bottom, lies inside it, eroded by `d`, `Region::erode_inner`). A
    /// tessellated scope is eroded by its deviation for witnesses and grown
    /// by it for absence, and an uncertain floor widens the band for
    /// witnesses and narrows it for absence. A body whose vertical bounds
    /// keep it out of the band (a slab under the floor whose certified
    /// extent ends there) occupies nothing either way. A deviation that
    /// bounds nothing refuses.
    ///
    /// With `taken`, each obstacle that may occupy anything near the scope
    /// is also kept apart there, with what it may and what it surely
    /// occupies ([`Taken`]), for [`FreeSpaceService::measure_free_floor`].
    ///
    /// The bracketing morphology runs in `mode` (#438): exactly, as every
    /// request is answered, or in the overlay's fast mode for a scene of
    /// the Fast precision class, each result on the side it is asked for
    /// and within the budget. The greatest excess a fast result states
    /// ([`Region::excess`]) is returned beside the floor; `None` where no
    /// morphology ran in the fast mode.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn floor_scene(
        self: &Arc<Self>,
        scope: &ObjectId,
        merged: &[ObjectId],
        obstacles: &[ObjectId],
        swept: &[SweptDoor],
        band: ElevationBand,
        tolerance: axiolid_core::Tolerance,
        mode: MorphologyMode,
        mut taken: Option<&mut Vec<Taken>>,
    ) -> Result<(Scene, f64, Option<f64>), FreeSpaceError> {
        let missing = |id: &ObjectId| FreeSpaceError::MissingGeometry(Box::new(id.clone()));
        let deviation = |id: &ObjectId| {
            self.geometry
                .deviation(id)
                .ok_or(FreeSpaceError::InexactPlacementEvidence)
        };
        let region_error = |e| FreeSpaceError::Unavailable(format!("bracketed footprint: {e:?}"));
        // The greatest excess of a fast morphology result, kept as each is
        // taken: the union it joins states none.
        let excess = std::cell::Cell::new(None::<f64>);
        let noted = |region: Region| {
            if let Some(more) = region.excess() {
                excess.set(Some(excess.get().map_or(more, |held| held.max(more))));
            }
            region
        };
        let mut bracketed: Vec<(ObjectId, f64)> = Vec::new();
        // Obstacles bounded rather than measured (#401).
        let mut bounded: Vec<ObjectId> = Vec::new();
        let mut floor = None;
        // How far the true floor may lie from the mesh's.
        let mut uncertainty: f64 = 0.0;
        let mut parts = Vec::new();
        for part in std::iter::once(scope).chain(merged) {
            let mesh = self.geometry.mesh(part).ok_or_else(|| missing(part))?;
            let (low, _) = mesh_extent(mesh).ok_or_else(|| missing(part))?;
            match floor {
                None => floor = Some(low[2]),
                Some(first) if (low[2] - first).abs() <= FLOOR_AGREEMENT_METRES => {}
                Some(_) => {
                    return Err(FreeSpaceError::Unavailable(format!(
                        "merged scope {part} stands on another floor than {scope}"
                    )));
                }
            }
            let d = deviation(part)?;
            if d > 0.0 {
                let bounds = self
                    .geometry
                    .vertical_bounds(part)
                    .ok_or(FreeSpaceError::InexactPlacementEvidence)?;
                uncertainty = uncertainty
                    .max(low[2] - bounds.bottom.0)
                    .max(bounds.bottom.1 - low[2]);
                bracketed.push((part.clone(), d));
            }
            parts.push((triangles(mesh), d));
        }
        let floor = floor.ok_or_else(|| missing(scope))?;
        // Bracketing morphology runs in a plan frame at the scope's corner:
        // at georeferenced coordinates (6e6 m) the overlay refuses the
        // slivers its own rounding makes about a grown or eroded outline
        // (`ZeroArea`). Moving there is exact for every point within a
        // factor two of the corner (Sterbenz), and back rounds within a
        // unit in the last place of the coordinates.
        let corner = parts
            .iter()
            .flat_map(|(part, _)| part.iter().flatten())
            .fold([f64::INFINITY; 2], |[x, y], p| [x.min(p.x), y.min(p.y)]);
        // The plan box every placement lies in: the scope's, grown by its
        // largest deviation (the room proofs of absence search). An
        // obstacle whose box, grown by its own deviation, is apart from it
        // occupies nothing a shape inside the scope can meet: the shape's
        // centrally symmetric dilation of it misses every centre the eroded
        // room keeps. Such obstacles are left out before their band
        // footprints and bracketing morphology are built, which cost the
        // overlay most of a floor's search when every object of a large
        // model is an obstacle of every space.
        let reach = parts.iter().map(|(_, d)| *d).fold(0.0, f64::max) + ON_SURFACE;
        let room_box: Extent = {
            let far_corner = parts
                .iter()
                .flat_map(|(part, _)| part.iter().flatten())
                .fold([f64::NEG_INFINITY; 2], |[x, y], p| [x.max(p.x), y.max(p.y)]);
            (
                [corner[0] - reach, corner[1] - reach, f64::NEG_INFINITY],
                [far_corner[0] + reach, far_corner[1] + reach, f64::INFINITY],
            )
        };
        let origin = Vec2::new(corner[0].floor(), corner[1].floor());
        let local = |region: Region| region.translate(-origin).map_err(region_error);
        let world = |region: Region| region.translate(origin).map_err(region_error);
        // A footprint at georeferenced coordinates is united in that frame
        // too (#304), and moved back; elsewhere where it lies, as before.
        let far = corner[0].abs().max(corner[1].abs()) >= 1.0e5;
        // Each footprint is bracketed around the shadows the overlay
        // refuses (`placement::bracketed_footprint`): inside for witnesses,
        // around for absence.
        let footprint_of = |part: &[Triangle]| -> Result<(Region, Region), FreeSpaceError> {
            if !far {
                return placement::bracketed_footprint(part, tolerance);
            }
            let moved: Vec<Triangle> = part
                .iter()
                .map(|face| face.map(|p| Point3::new(p.x - origin.x, p.y - origin.y, p.z)))
                .collect();
            let (inner, outer) = placement::bracketed_footprint(&moved, tolerance)?;
            Ok((world(inner)?, world(outer)?))
        };

        // The scope: one footprint when exact, else each part eroded
        // (witnesses) and grown (absence) by its own deviation.
        let (footprint, room) = if parts.iter().all(|(_, d)| *d == 0.0) {
            let all: Vec<Triangle> = parts.into_iter().flat_map(|(part, _)| part).collect();
            footprint_of(&all)?
        } else {
            let (mut inner, mut outer) = (Region::empty(), Region::empty());
            for (part, d) in &parts {
                let (own_inner, own_outer) = footprint_of(part)?;
                let (own_inner, own_outer) = (local(own_inner)?, local(own_outer)?);
                let (sure, possible) = if *d > 0.0 {
                    (
                        own_inner
                            .erode_inner_with(*d, tolerance, mode)
                            .map(noted)
                            .map_err(region_error)?,
                        own_outer
                            .dilate_outer_with(*d, tolerance, mode)
                            .map(noted)
                            .map_err(region_error)?,
                    )
                } else {
                    (own_inner, own_outer)
                };
                inner = inner.union(&sure, tolerance).map_err(region_error)?;
                outer = outer.union(&possible, tolerance).map_err(region_error)?;
            }
            (world(inner)?, world(outer)?)
        };
        if room.is_empty() {
            return Err(missing(scope));
        }
        let (low, high) = (floor + band.from_metres(), floor + band.to_metres());
        let u = uncertainty;
        let mut exact_rings = Vec::new();
        let mut sure_rings = Vec::new();
        let mut possible = Region::empty();
        let mut certain = Region::empty();
        // A band footprint as a region in the local frame.
        let to_local = |plan: &Plan| {
            let rings: Vec<Ring> = trapezoids(plan)
                .into_iter()
                .map(|piece| Ring {
                    points: piece
                        .outer
                        .points
                        .iter()
                        .map(|p| Point2::new(p.x - origin.x, p.y - origin.y))
                        .collect(),
                })
                .collect();
            placement::union(&rings, tolerance)
        };
        for obstacle in obstacles {
            if self.geometry.has_no_body(obstacle) {
                continue;
            }
            // An obstacle without geometry could stand anywhere, so neither a
            // witness nor a proof of absence would be complete, unless the
            // host bounds it (`with_unmeasured_bounds`) apart from the scope:
            // every box it lies within the union of is apart in plan, or
            // below or above the band (which is open at both ends, as for an
            // exact body touching its limits).
            if self.geometry.mesh(obstacle).is_none()
                && self
                    .geometry
                    .unmeasured_bound(obstacle)
                    .is_some_and(|bound| {
                        bound.iter().all(|piece| {
                            extent_gap(piece, &room_box, true) > 0.0
                                || piece.1[2] <= low - u
                                || piece.0[2] >= high + u
                        })
                    })
            {
                continue;
            }
            // An unmeasured body the host encloses in a closed solid (#401):
            // a possible obstacle within it. Witnesses avoid what the solid
            // may occupy, as a tessellated body's; no absence rests on it.
            if self.geometry.mesh(obstacle).is_none()
                && let Some(enclosure) = self.geometry.enclosure(obstacle)
            {
                let vertical = enclosure
                    .vertical_bounds()
                    .ok_or_else(|| missing(obstacle))?;
                if vertical.clear_of(low - u, high + u, ON_SURFACE) {
                    continue;
                }
                let d = enclosure.deviation;
                if let Some((low_corner, high_corner)) = mesh_extent(&enclosure.mesh) {
                    let grown: Extent = (
                        [low_corner[0] - d, low_corner[1] - d, low_corner[2]],
                        [high_corner[0] + d, high_corner[1] + d, high_corner[2]],
                    );
                    if extent_gap(&grown, &room_box, true) > 0.0 {
                        continue;
                    }
                }
                let widened = self
                    .band_footprint(obstacle, Body::Enclosure, low - u - d, high + u + d)
                    .map_err(FreeSpaceError::Unavailable)?;
                let mut may = to_local(&widened)?;
                if d > 0.0 && !may.is_empty() {
                    may = may
                        .dilate_outer_with(d, tolerance, mode)
                        .map(noted)
                        .map_err(region_error)?;
                }
                bounded.push(obstacle.clone());
                possible = possible.union(&may, tolerance).map_err(region_error)?;
                if let Some(taken) = taken.as_mut() {
                    taken.push(Taken {
                        object: obstacle.clone(),
                        possible: world(may)?,
                        sure: Region::empty(),
                    });
                }
                continue;
            }
            let mesh = self
                .geometry
                .mesh(obstacle)
                .ok_or_else(|| missing(obstacle))?;
            let d = deviation(obstacle)?;
            if let Some((low_corner, high_corner)) = mesh_extent(mesh) {
                let grown: Extent = (
                    [low_corner[0] - d, low_corner[1] - d, low_corner[2]],
                    [high_corner[0] + d, high_corner[1] + d, high_corner[2]],
                );
                if extent_gap(&grown, &room_box, true) > 0.0 {
                    continue;
                }
            }
            // An open authored mesh whose solid the band needs (a sanitary
            // terminal's open shell standing on the floor) is bounded by
            // its vertices' hull (#401), a possible obstacle as above; its
            // closed pieces still surely occupy what they do.
            if let Some(refusal) = self.band_refusal(obstacle, d, low, high, u) {
                if !open(mesh) {
                    return Err(FreeSpaceError::Unavailable(refusal));
                }
                let may = hull_region(mesh, origin, d, tolerance)?;
                let mut surely = Region::empty();
                if let Some(closed) = closed_pieces(mesh) {
                    let core = if d == 0.0 {
                        let (bottom, top) = if u == 0.0 {
                            (low, high)
                        } else {
                            (low + u, high - u)
                        };
                        (top > bottom)
                            .then(|| band_footprint(obstacle, &closed, bottom, top).ok())
                            .flatten()
                            .and_then(|plan| to_local(&plan).ok())
                    } else {
                        let level = low + u + 2.0 * d;
                        (level < high - u)
                            .then(|| solid_column(obstacle, &closed, level, d).ok())
                            .flatten()
                            .and_then(|(section, shadow)| {
                                to_local(&section)
                                    .ok()?
                                    .difference(&to_local(&shadow).ok()?, tolerance)
                                    .ok()?
                                    .erode_inner_with(d, tolerance, mode)
                                    .ok()
                                    .map(noted)
                            })
                    };
                    // A core the overlay refuses is left out: absence is
                    // then proven without it.
                    surely = core.unwrap_or_else(Region::empty);
                }
                bounded.push(obstacle.clone());
                possible = possible.union(&may, tolerance).map_err(region_error)?;
                if !surely.is_empty() {
                    certain = certain.union(&surely, tolerance).map_err(region_error)?;
                }
                if let Some(taken) = taken.as_mut() {
                    taken.push(Taken {
                        object: obstacle.clone(),
                        possible: world(may)?,
                        sure: world(surely)?,
                    });
                }
                continue;
            }
            if d == 0.0 {
                let witness = self
                    .band_footprint(obstacle, Body::Mesh, low - u, high + u)
                    .map_err(FreeSpaceError::Unavailable)?;
                let may: Vec<Ring> = trapezoids(&witness)
                    .into_iter()
                    .map(|piece| piece.outer)
                    .collect();
                let surely: Vec<Ring> = if u == 0.0 {
                    may.clone()
                } else if high - u > low + u {
                    let sure = self
                        .band_footprint(obstacle, Body::Mesh, low + u, high - u)
                        .map_err(FreeSpaceError::Unavailable)?;
                    trapezoids(&sure)
                        .into_iter()
                        .map(|piece| piece.outer)
                        .collect()
                } else {
                    Vec::new()
                };
                if let Some(taken) = taken.as_mut() {
                    taken.push(Taken {
                        object: obstacle.clone(),
                        possible: placement::union(&may, tolerance)?,
                        sure: placement::union(&surely, tolerance)?,
                    });
                }
                exact_rings.extend(may);
                sure_rings.extend(surely);
                continue;
            }
            let bounds = self
                .geometry
                .vertical_bounds(obstacle)
                .ok_or(FreeSpaceError::InexactPlacementEvidence)?;
            // Wherever the true body lies, it stays out of the band: it
            // occupies nothing there, as an exact body touching the band's
            // limits does not.
            if bounds.clear_of(low - u, high + u, ON_SURFACE) {
                continue;
            }
            bracketed.push((obstacle.clone(), d));
            let widened = self
                .band_footprint(obstacle, Body::Mesh, low - u - d, high + u + d)
                .map_err(FreeSpaceError::Unavailable)?;
            let grown = to_local(&widened)?
                .dilate_outer_with(d, tolerance, mode)
                .map(noted)
                .map_err(region_error)?;
            possible = possible.union(&grown, tolerance).map_err(region_error)?;
            // A column of height 2d a deviation above the band's surest
            // bottom, so a body standing on the floor is entered from its
            // side only; its centre lies in the band however the floor lies.
            let level = low + u + 2.0 * d;
            let mut surely = Region::empty();
            if level < high - u {
                let (section, shadow) = self
                    .solid_column(obstacle, Body::Mesh, level, d)
                    .map_err(FreeSpaceError::Unavailable)?;
                let core = to_local(&section)?
                    .difference(&to_local(&shadow)?, tolerance)
                    .map_err(region_error)?;
                if !core.is_empty() {
                    surely = core
                        .erode_inner_with(d, tolerance, mode)
                        .map(noted)
                        .map_err(region_error)?;
                    certain = certain.union(&surely, tolerance).map_err(region_error)?;
                }
            }
            if let Some(taken) = taken.as_mut() {
                taken.push(Taken {
                    object: obstacle.clone(),
                    possible: world(grown)?,
                    sure: world(surely)?,
                });
            }
        }
        if let Some(taken) = taken {
            for door in swept {
                let (inner, outer) = swept_rings(std::slice::from_ref(door), floor, high);
                taken.push(Taken {
                    object: door.door().clone(),
                    possible: placement::union(&outer, tolerance)?,
                    sure: placement::union(&inner, tolerance)?,
                });
            }
        }
        let (inner, outer) = swept_rings(swept, floor, high);
        let with = |mut rings: Vec<Ring>, sectors: Vec<Ring>, bracket: Region| {
            rings.extend(sectors);
            let region = placement::union(&rings, tolerance)?;
            if bracket.is_empty() {
                Ok(region)
            } else {
                region
                    .union(&world(bracket)?, tolerance)
                    .map_err(region_error)
            }
        };
        let scene = Scene {
            scope: footprint,
            room,
            obstacles: with(exact_rings, outer, possible)?,
            sure: with(sure_rings, inner, certain)?,
            tolerance,
            window: None,
            reach: None,
            bracketed,
            bounded,
            clear: Default::default(),
            inscribed: Default::default(),
            simple: Default::default(),
            coarse: Default::default(),
            found: placement::Found::default(),
            searched: placement::Searched::default(),
            work: Work::default(),
        };
        Ok((scene, floor, excess.get()))
    }
}

/// Whether some piece of `mesh` is open ([`crate::shell::pieces`]): an
/// authored surface with no inside, whose solid only its vertices bound.
fn open(mesh: &TriMesh) -> bool {
    crate::shell::pieces(mesh).is_some_and(|pieces| pieces.iter().any(|piece| !piece.closed))
}

/// The closed pieces of `mesh` ([`crate::shell::pieces`]) as a mesh of
/// their own: each bounds a solid that is part of the body, whatever its
/// open pieces stand for. `None` when it has none.
fn closed_pieces(mesh: &TriMesh) -> Option<TriMesh> {
    let mut indices = Vec::new();
    for piece in crate::shell::pieces(mesh)? {
        if piece.closed {
            for triangle in piece.triangles {
                indices.extend_from_slice(mesh.indices.get(3 * triangle..3 * triangle + 3)?);
            }
        }
    }
    (!indices.is_empty()).then(|| TriMesh::new(mesh.positions.clone(), indices))
}

/// The plan region, in the frame at `origin`, that an open mesh's body
/// lies within (#401): the hull of its vertices ([`enclosing_hull`]),
/// grown by a square of half side its chord deviation `d` plus the
/// rounding of the vertices' coordinates and their move to the frame, at
/// least [`HULL_GROWTH_METRES`] (growing more still holds it). Every point of
/// a triangle lies in the hull of its corners, and the faces the surface
/// lacks span its vertices as well, so the solid it stands for lies in
/// the hull of every vertex: exactly, up to that rounding, for an exact
/// mesh, and within `d` of it for a tessellated one. Where the overlay
/// refuses the hull's ring, the box of the vertices stands for it.
fn hull_region(
    mesh: &TriMesh,
    origin: Vec2,
    d: f64,
    tolerance: axiolid_core::Tolerance,
) -> Result<Region, FreeSpaceError> {
    let magnitude = mesh
        .positions
        .iter()
        .fold(0.0_f64, |largest, p| largest.max(p.x.abs()).max(p.y.abs()));
    // The square of half side `grow` holds the disc of that radius, and
    // its sum with the hull is the hull of the vertices moved to its
    // corners: a polygon, so the obstacles gain no arcs. Each move rounds
    // by less than the coordinates' rounding counted in `grow`.
    let grow = (d + 16.0 * f64::EPSILON * magnitude).max(HULL_GROWTH_METRES);
    let points: Vec<(f64, f64)> = mesh
        .positions
        .iter()
        .flat_map(|p| {
            let (x, y) = (p.x - origin.x, p.y - origin.y);
            [
                (x - grow, y - grow),
                (x + grow, y - grow),
                (x + grow, y + grow),
                (x - grow, y + grow),
            ]
        })
        .collect();
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return Err(FreeSpaceError::Unavailable(
            "an open mesh has a vertex that is not finite".into(),
        ));
    }
    let ring = |corners: &[(f64, f64)]| Ring {
        points: corners.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
    };
    let hull = enclosing_hull(points.clone());
    let region = (hull.len() >= 3)
        .then(|| placement::union(&[ring(&hull)], tolerance).ok())
        .flatten()
        .filter(|region| !region.is_empty());
    // The box of the grown vertices.
    let boxed = || {
        let (min, max) = points.iter().fold(
            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
            |(min, max), &(x, y)| {
                (
                    [min[0].min(x), min[1].min(y)],
                    [max[0].max(x), max[1].max(y)],
                )
            },
        );
        placement::union(
            &[ring(&[
                (min[0], min[1]),
                (max[0], min[1]),
                (max[0], max[1]),
                (min[0], max[1]),
            ])],
            tolerance,
        )
    };
    region.map_or_else(boxed, Ok).map_err(|error| {
        FreeSpaceError::Unavailable(format!("the box of an open mesh's vertices: {error}"))
    })
}

/// The least growth of an open mesh's hull ([`hull_region`]), in metres:
/// above the coordinates' rounding, far below anything a shape could use.
const HULL_GROWTH_METRES: f64 = 1.0e-6;

/// The least floor an obstacle must possibly take to be named as possibly
/// taking it: a wall flush with a scope bracketed around its slivers
/// (`SLIVER_MARGIN_METRES`) shares some 1e-8 m² with the room, which is
/// no floor anyone could stand on. A body bracketed by a chord deviation
/// shares far more and stays possible.
const POSSIBLE_FLOOR_M2: f64 = 1.0e-6;

/// What one obstacle or swept door may and surely occupies in a scene's
/// band, in plan: `possible` contains its footprint (as witnesses avoid
/// it), `sure` lies inside it (as absence is proven against it).
pub(crate) struct Taken {
    pub(crate) object: ObjectId,
    pub(crate) possible: Region,
    pub(crate) sure: Region,
}

/// Chords (and tangents) per quarter turn of a swept sector: its polygons
/// lie within `r · (1 / cos(π / 128) − 1)`, under 0.3 mm per metre, of
/// the arc.
pub(crate) const SWEEP_SEGMENTS: usize = 32;

/// The plan polygons of every sector of `swept` standing on the floor at
/// `floor` whose band reaches up to `top`, anticlockwise: inscribed ones
/// (inside the sectors) and circumscribed ones (around them).
pub(crate) fn swept_rings(swept: &[SweptDoor], floor: f64, top: f64) -> (Vec<Ring>, Vec<Ring>) {
    let ring = |points: Vec<[f64; 2]>| Ring {
        points: points.into_iter().map(|[x, y]| Point2::new(x, y)).collect(),
    };
    let mut inner = Vec::new();
    let mut outer = Vec::new();
    for door in swept {
        for sector in door.stands_on(floor, top) {
            // A swept door's sectors are horizontal, so they have bounds.
            if let Some((low, high)) = sector.plan_bounds(SWEEP_SEGMENTS) {
                inner.push(ring(low));
                outer.push(ring(high));
            }
        }
    }
    (inner, outer)
}

/// The locator suffix naming the tessellated bodies a placement or map was
/// bracketed by, each with its chord deviation: the verdict holds for every
/// body within it, so the evidence is exact, and a reviewer sees which
/// curved bodies it was decided around. Empty when every body was exact.
pub(crate) fn within_deviation(bracketed: &[(ObjectId, f64)]) -> String {
    if bracketed.is_empty() {
        return String::new();
    }
    let named: Vec<String> = bracketed
        .iter()
        .map(|(object, deviation)| format!("{object}={deviation:e}m"))
        .collect();
    format!(":within-chord-deviation={}", named.join(","))
}

/// The locator suffix naming the obstacles a placement or map was decided
/// around within their bounds rather than measured (#401): an unmeasured
/// body within its enclosure, an open mesh within its vertices' hull. A
/// witness avoids each bound; no absence rests on one. Empty when every
/// obstacle was measured.
pub(crate) fn within_bounds(bounded: &[ObjectId]) -> String {
    if bounded.is_empty() {
        return String::new();
    }
    let named: Vec<String> = bounded.iter().map(ToString::to_string).collect();
    format!(":bounded-not-measured={}", named.join(","))
}

/// Objects named in order, comma separated.
fn names(objects: &[ObjectId]) -> String {
    let named: Vec<String> = objects.iter().map(ToString::to_string).collect();
    named.join(", ")
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
    fn map_circulation(
        &self,
        request: &CirculationRequest,
    ) -> Result<CirculationMap, FreeSpaceError> {
        self.circulation_map(request)
    }

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
            .over
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
            if self.over.geometry.has_no_body(obstacle) {
                continue;
            }
            // A named obstacle without geometry cannot be shown to be clear of
            // the volume, and `Clear` asserts that nothing obstructs it.
            let mesh = self
                .over
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

    #[allow(clippy::too_many_lines)]
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
        let (mut scene, floor, derived) = self.placement_scene(request, tolerance)?;
        if let Some(offsets) = offsets {
            scene.window = Some(offset_window(offsets, request.scope(), floor)?);
        }
        if let Some(reach) = request.entrance_reach() {
            scene.reach = Some(self.reached(&scene, reach)?);
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
        if let Some(reach) = request.entrance_reach() {
            let entrances: Vec<String> =
                reach.entrances().iter().map(ToString::to_string).collect();
            let _ = write!(
                locator,
                ":reached-from={}:path-width={:.6}",
                entrances.join(","),
                reach.width_metres()
            );
        }
        locator.push_str(&within_deviation(&scene.bracketed));
        locator.push_str(&within_bounds(&scene.bounded));
        derived.keep(&scene);
        let evidence = Evidence::exact(request.scope().source.clone(), locator);
        let (centre, right) = match search {
            Search::Nowhere => {
                return Ok(PlacementOutcome::NoPlacement(
                    CompletePlacementEvidence::try_new(request.clone(), evidence)?,
                ));
            }
            Search::Found { centre, right } => (centre, right),
            // What an obstacle's bound leaves open is undecided, naming
            // the obstacles bounded rather than measured.
            Search::Undecided(why) if !scene.bounded.is_empty() => {
                return Err(FreeSpaceError::Unavailable(format!(
                    "the obstacles bounded rather than measured ({}) leave the placement \
                     undecided: no witness avoids their bounds and no absence holds without \
                     them ({why})",
                    names(&scene.bounded)
                )));
            }
            // What a chord deviation leaves open is not exact evidence
            // either way.
            Search::Undecided(_) if !scene.bracketed.is_empty() => {
                return Err(FreeSpaceError::InexactPlacementEvidence);
            }
            Search::Undecided(why) => return Err(FreeSpaceError::Unavailable(why.into())),
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

    /// The largest disc of the placement scene's clear floor, estimated
    /// (`placement::inscribed_disc`, the pole of inaccessibility) in the
    /// witnesses' clear part for the radius expected to fit and in the
    /// absence's for the one expected to be proven absent, each moved by
    /// the polygons a circle search verifies and proves with
    /// (`placement::searched_radii`). The scene and its clear parts are
    /// kept as a search keeps them.
    fn estimate_largest_disc(
        &self,
        request: &PlacementRequest,
    ) -> Result<LargestDiscEstimate, FreeSpaceError> {
        let tolerance = tolerance()?;
        let (scene, _, derived) = self.placement_scene(request, tolerance)?;
        let estimate = scene.inscribed(false).zip(scene.inscribed(true));
        derived.keep(&scene);
        let (witness, absence) = estimate.ok_or_else(|| {
            FreeSpaceError::Unavailable("the overlay refused the clear floor".into())
        })?;
        let (fits, exceeds) = placement::searched_radii(witness.found, absence.bound);
        LargestDiscEstimate::new(fits, exceeds).ok_or_else(|| {
            FreeSpaceError::Unavailable("the largest disc's estimate is not finite".into())
        })
    }

    /// The placement scene's free floor: the witnesses' scope less every
    /// possible obstacle from below, the absence's room less the sure ones
    /// from above, as `circulation.rs` brackets the free area. An obstacle
    /// or swept door surely takes floor where what it surely occupies
    /// shares area with the witnesses' scope (inside the true floor), and
    /// possibly where what it may occupy shares more than
    /// `POSSIBLE_FLOOR_M2` (1 mm²) with the room.
    fn measure_free_floor(
        &self,
        request: &PlacementRequest,
    ) -> Result<FreeFloorEvidence, FreeSpaceError> {
        let tolerance = tolerance()?;
        let mut taken = Vec::new();
        let (scene, _, _) = self.over.floor_scene(
            request.scope(),
            request.merged_scopes(),
            request.obstacles(),
            request.swept_doors(),
            request.effective_band(),
            tolerance,
            MorphologyMode::Exact,
            Some(&mut taken),
        )?;
        let fail = |e| FreeSpaceError::Unavailable(format!("free floor: {e:?}"));
        let less = |scope: &Region, obstacles: &Region| {
            if obstacles.is_empty() {
                Ok(scope.clone())
            } else {
                scope.difference(obstacles, tolerance).map_err(fail)
            }
        };
        let free = less(&scene.scope, &scene.obstacles)?.area();
        let roomy = less(&scene.room, &scene.sure)?.area();
        // The two ends are one area where nothing is bracketed, up to the
        // overlay's rounding of either difference.
        let area = AreaInterval::try_new(free.min(roomy).max(0.0), roomy.max(free).max(0.0))?;
        let shares = |region: &Region, with: &Region, least: f64| -> Result<bool, FreeSpaceError> {
            if region.is_empty() || with.is_empty() {
                return Ok(false);
            }
            Ok(region.intersection(with, tolerance).map_err(fail)?.area() > least)
        };
        let mut sure = Vec::new();
        let mut possible = Vec::new();
        for Taken {
            object,
            possible: may,
            sure: surely,
        } in &taken
        {
            if shares(surely, &scene.scope, AREA_EPSILON_M2)? {
                sure.push(object.clone());
            } else if shares(may, &scene.room, POSSIBLE_FLOOR_M2)? {
                possible.push(object.clone());
            }
        }
        // A door both swept and an obstacle is taken by either footprint.
        sure.sort();
        sure.dedup();
        possible.retain(|object| sure.binary_search(object).is_err());
        let mut locator = format!("axiolid:free-floor:{}", request.scope().local_id);
        for merged in request.merged_scopes() {
            locator.push('+');
            locator.push_str(&merged.to_string());
        }
        locator.push_str(&within_deviation(&scene.bracketed));
        locator.push_str(&within_bounds(&scene.bounded));
        FreeFloorEvidence::try_new(
            request.clone(),
            area,
            sure,
            possible,
            Evidence::exact(request.scope().source.clone(), locator),
        )
    }

    fn measure_free_area(
        &self,
        request: &FreeAreaRequest,
    ) -> Result<FreeAreaEvidence, FreeSpaceError> {
        let tolerance = tolerance()?;
        let scope_mesh =
            self.over.geometry.mesh(request.scope()).ok_or_else(|| {
                FreeSpaceError::MissingGeometry(Box::new(request.scope().clone()))
            })?;
        // Free area is exact evidence. A tessellated scope, or a tessellated
        // obstacle whose true footprint could reach it, makes it an estimate.
        let scope_extent = self
            .over
            .geometry
            .enclosing_extent(request.scope())
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(request.scope().clone())))?;
        if self.over.geometry.is_tessellated(request.scope())
            || self
                .over
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
            if self.over.geometry.has_no_body(obstacle) {
                continue;
            }
            let mesh = self
                .over
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
        crate::containment::assess(&self.over.geometry, &self.source, request)
    }

    fn assess_support_coverage(
        &self,
        request: &axioval_engine::SupportCoverageRequest,
    ) -> Result<axioval_engine::SupportCoverageOutcome, FreeSpaceError> {
        crate::support_coverage::assess(&self.over.geometry, &self.source, request)
    }
}
