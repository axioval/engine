//! Stair flights, ramps and headroom from registered meshes.
//!
//! ADR 0004: this module measures positions; whether a riser is too high or
//! a ramp too steep is a rule's decision.
//!
//! Only an exact planar mesh is measured, and only a closed, outward-facing
//! one, since whether a face looks up is read from its winding. A
//! tessellation of curved faces is refused: its "treads" are chords, not the
//! body's surfaces. Plane detection on meshed bodies (axiolid/kernel#131) is
//! not available, so a tread is recognised only where it is level, which an
//! exact mesh of a planar body is.
//!
//! - **Treads** are the upward-facing faces whose three corners share one
//!   elevation up to [`LEVEL_TOLERANCE`], the rounding a placement transform
//!   leaves in coordinates, grouped by elevation; a tread is measured at the
//!   interval of its corners' elevations. A flight also having a sloped
//!   face flatter than 45° looking up is refused: that face is neither a
//!   tread nor measured.
//! - The **walking direction** is derived from the treads, not the
//!   placement, since a source's placement axes say nothing about which way
//!   a flight climbs: it runs in plan from the centre of the lowest tread's
//!   bounding rectangle to that of the highest. Every tread's centre must
//!   lie on that line within [`STRAIGHT_TOLERANCE`] and further along it
//!   than the one below; otherwise the flight turns (winders, a quarter
//!   landing) and is refused. A direction along a coordinate axis projects
//!   exactly; any other is widened by a bound on the rounding of the
//!   projection and reported as approximate.
//! - A flight must be **one piece**: its lowest point is where its first
//!   riser starts. A flight in several pieces (open risers, separate
//!   treads) does not carry the floor it starts from, and is refused.
//! - A ramp's **runs** are the connected sets of upward-facing sloped faces
//!   flatter than 45°; each must be planar within [`PLANAR_TOLERANCE`], and
//!   its direction is its plane's steepest ascent. Horizontal faces between
//!   runs are landings, not runs.
//! - **Headroom** is the least vertical distance from a walking face to a
//!   requested obstacle's faces directly above it, over the plan region they
//!   share. It is computed in floating point and widened by a numerical
//!   margin, so it is never exact. An obstacle whose faces over the walking
//!   surface lie both below and above it crosses the surface and is refused.
//! - **Widths** are stated only for a tread or run that fills a rectangle
//!   along its walking direction: every edge on the boundary of its faces
//!   lies on a side of the rectangle its positions span, and its plan area
//!   is that rectangle's. Anything else has no measured width.
//! - A **landing** is the connected set of upward level faces of one
//!   requested candidate (or of a ramp itself, never of a flight) at the
//!   elevation of the end, reaching into the plan strip between the end's
//!   walking surface and the end of its body, grown by [`LANDING_REACH`],
//!   across the end's width. Several such surfaces are refused, since their
//!   union is not measured; one that fills a rectangle along the leaving
//!   direction is measured, any other only found.
//! - The **clearance below** a subject is the least height of its
//!   downward-facing faces above the downward-facing level faces (the floor)
//!   of a requested space, over the plan region they share, leaving out the
//!   region where a face of the subject lies flush on that floor: the
//!   subject rests there, and nobody stands under it.

use std::collections::BTreeMap;

use axiolid_core::{Point3, Tolerance};
use axiolid_mesh::{TriMesh, TriangleMeshView, audit_mesh, component_count};
use axioval_engine::{
    ClearanceBelow, ClearanceBelowRequest, ElevationInterval, Headroom, HeadroomRequest, Landing,
    LandingEvidence, LandingExtent, LandingRequest, MeasuredInterval, MetricDirection, SlopedRun,
    SlopedSurface, Tread, TreadFlight, WalkingEnd, WalkingSurfaceError, WalkingSurfaceService,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Triangle, mesh_extent, triangles};

/// Mesh audit tolerance, as tight as the other services'.
const LINEAR_TOLERANCE: f64 = 1e-9;
const ANGULAR_TOLERANCE: f64 = 1e-9;

/// How far, in metres, a tread's centre may lie beside the line through the
/// lowest and highest treads' centres before the flight counts as turning.
pub const STRAIGHT_TOLERANCE: f64 = 1e-6;

/// How far, relative to the coordinates' magnitude, a corner of a ramp run
/// may lie off the run's plane before the run counts as warped.
pub const PLANAR_TOLERANCE: f64 = 1e-9;

/// How far, relative to the coordinates' magnitude, a face's corners may
/// differ in elevation and still be level: the rounding a placement
/// transform leaves in coordinates, a few units in the last place.
pub const LEVEL_TOLERANCE: f64 = 64.0 * f64::EPSILON;

/// Relative numerical margin on a headroom, scaled by the coordinates'
/// magnitude and the slopes of the two faces.
const HEADROOM_MARGIN: f64 = 1e-9;

/// How far, in metres, a landing may stand off the end of a flight or run
/// and still meet it: the rounding two objects' placements leave, not a
/// modelling gap.
pub const LANDING_REACH: f64 = 1e-6;

/// Measures stair flights, ramps and headroom over registered meshes.
#[derive(Debug)]
pub struct AxiolidWalkingSurfaceService {
    geometry: AxiolidGeometry,
}

impl AxiolidWalkingSurfaceService {
    /// Creates a service over the supplied geometry, which may hold several
    /// sources: each measurement cites the measured object's own source.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self { geometry }
    }
}

/// How a face of a closed, outward-facing body looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Facing {
    /// Upward and exactly horizontal: a tread or landing.
    Level,
    /// Upward and flatter than 45°: a walking slope.
    Sloped,
    /// Anything else: steep, vertical or facing down.
    Other,
}

/// The plan cross product of a triangle (twice its signed plan area) and a
/// bound under which its sign is not trusted.
fn plan_cross([a, b, c]: &Triangle) -> (f64, f64) {
    let first = (b.x - a.x) * (c.y - a.y);
    let second = (b.y - a.y) * (c.x - a.x);
    (
        first - second,
        8.0 * f64::EPSILON * (first.abs() + second.abs()),
    )
}

/// The full normal of a triangle, from its winding.
fn normal([first, second, third]: &Triangle) -> [f64; 3] {
    let cross = (*second - *first).cross(*third - *first);
    [cross.x, cross.y, cross.z]
}

/// The steepness of a face: horizontal over vertical normal component.
fn gradient(triangle: &Triangle) -> f64 {
    let [x, y, z] = normal(triangle);
    x.hypot(y) / z.abs()
}

/// Whether a face is level: its corners' elevations agree up to the
/// rounding a placement transform leaves in coordinates.
fn is_level(triangle: &Triangle) -> bool {
    let (low, high) = elevations(triangle);
    let scale = triangle.iter().fold(1.0_f64, |scale, p| {
        scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    });
    high - low <= LEVEL_TOLERANCE * scale
}

/// The lowest and highest corner elevation of a face.
fn elevations(triangle: &Triangle) -> (f64, f64) {
    triangle
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), p| {
            (low.min(p.z), high.max(p.z))
        })
}

fn facing(object: &ObjectId, triangle: &Triangle) -> Result<Facing, WalkingSurfaceError> {
    let (cross, bound) = plan_cross(triangle);
    let level = is_level(triangle);
    if cross.abs() <= bound {
        if level {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "a horizontal face of {object} is too small to tell whether it faces up"
            )));
        }
        return Ok(Facing::Other);
    }
    if cross < 0.0 {
        return Ok(Facing::Other);
    }
    if level {
        return Ok(Facing::Level);
    }
    let steepness = gradient(triangle);
    if steepness < 1.0 - 1e-9 {
        Ok(Facing::Sloped)
    } else if steepness > 1.0 + 1e-9 {
        Ok(Facing::Other)
    } else {
        Err(WalkingSurfaceError::Unsupported(format!(
            "a face of {object} slopes at 45°, neither walkable nor steep"
        )))
    }
}

/// A body this service can read faces of.
struct Solid<'a> {
    mesh: &'a TriMesh,
    soup: Vec<Triangle>,
}

impl AxiolidWalkingSurfaceService {
    fn solid(&self, object: &ObjectId) -> Result<Solid<'_>, WalkingSurfaceError> {
        if self.geometry.has_no_body(object) {
            return Err(WalkingSurfaceError::Unavailable(format!(
                "{object} is declared to have no body"
            )));
        }
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
            return Err(WalkingSurfaceError::Unavailable(format!(
                "{object} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self
            .geometry
            .mesh(object)
            .ok_or_else(|| WalkingSurfaceError::UnknownObject(object.clone()))?;
        if self.geometry.is_tessellated(object) {
            return Err(WalkingSurfaceError::InexactGeometry(format!(
                "{object} is a tessellation of curved faces, whose faces are chords of its \
                 surface"
            )));
        }
        let tolerance = Tolerance::new(LINEAR_TOLERANCE, ANGULAR_TOLERANCE)
            .map_err(|_| WalkingSurfaceError::Unavailable("invalid audit tolerance".into()))?;
        let health = audit_mesh(mesh, tolerance);
        if !health.is_surface_usable() {
            return Err(WalkingSurfaceError::Unavailable(format!(
                "the mesh of {object} cannot be read"
            )));
        }
        if !health.is_closed_two_manifold() {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "the mesh of {object} is not a closed surface, so which faces look up is unknown"
            )));
        }
        let soup = triangles(mesh);
        let volume: f64 = soup.iter().map(|[a, b, c]| a.dot(b.cross(*c))).sum::<f64>() / 6.0;
        if volume <= 0.0 {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "the mesh of {object} faces inward, so which faces look up is unknown"
            )));
        }
        Ok(Solid { mesh, soup })
    }
}

/// A position known exactly.
fn exact(value: f64) -> Result<ElevationInterval, WalkingSurfaceError> {
    ElevationInterval::exact(value).map_err(|_| WalkingSurfaceError::InvalidMeasurement)
}

/// Positions of points along a horizontal direction.
struct Projection {
    axis: [f64; 3],
    on_axis: bool,
}

impl Projection {
    fn new(direction: MetricDirection) -> Self {
        let axis = direction.components();
        let on_axis = axis.iter().filter(|component| **component != 0.0).count() == 1;
        Self { axis, on_axis }
    }

    /// `(lowest, highest)` positions of `points`, each an interval.
    fn span<'p>(
        &self,
        points: impl Iterator<Item = &'p Point3>,
    ) -> Result<(ElevationInterval, ElevationInterval), WalkingSurfaceError> {
        let mut lowest = (f64::INFINITY, f64::INFINITY);
        let mut highest = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for point in points {
            let terms = [point.x * self.axis[0], point.y * self.axis[1]];
            let projected = terms[0] + terms[1];
            // A coordinate axis reads one coordinate exactly. Otherwise two
            // products and a sum round by at most 2u·Σ|terms|, and the
            // stored unit direction is off unit length by a few u: 8ε of
            // the coordinates covers both.
            let rounding = if self.on_axis {
                0.0
            } else {
                8.0 * f64::EPSILON * (point.x.abs() + point.y.abs()).max(f64::MIN_POSITIVE)
            };
            let (low, high) = (projected - rounding, projected + rounding);
            lowest = (lowest.0.min(low), lowest.1.min(high));
            highest = (highest.0.max(low), highest.1.max(high));
        }
        let interval = |(low, high): (f64, f64)| {
            ElevationInterval::try_new(low, high)
                .map_err(|_| WalkingSurfaceError::InvalidMeasurement)
        };
        Ok((interval(lowest)?, interval(highest)?))
    }
}

/// The plan direction from `from` to `to`, exactly a coordinate axis when
/// the two share a coordinate.
#[allow(clippy::float_cmp)]
fn plan_direction(from: [f64; 2], to: [f64; 2]) -> Result<MetricDirection, WalkingSurfaceError> {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let vector = if dy == 0.0 {
        [dx.signum(), 0.0, 0.0]
    } else if dx == 0.0 {
        [0.0, dy.signum(), 0.0]
    } else {
        [dx, dy, 0.0]
    };
    MetricDirection::try_new(vector).map_err(|_| WalkingSurfaceError::InvalidMeasurement)
}

/// Upward level faces at one elevation, known to lie in `[low, high]`.
struct Level {
    low: f64,
    high: f64,
    faces: Vec<Triangle>,
}

/// Level faces grouped by elevation, lowest first: faces whose elevations
/// lie within [`LEVEL_TOLERANCE`] of each other share a level.
fn group_levels(mut faces: Vec<Triangle>) -> Vec<Level> {
    faces.sort_by(|a, b| elevations(a).0.total_cmp(&elevations(b).0));
    let mut levels: Vec<Level> = Vec::new();
    for face in faces {
        let (low, high) = elevations(&face);
        let scale = high.abs().max(1.0);
        match levels.last_mut() {
            Some(level) if low <= level.high + LEVEL_TOLERANCE * scale => {
                level.high = level.high.max(high);
                level.faces.push(face);
            }
            _ => levels.push(Level {
                low,
                high,
                faces: vec![face],
            }),
        }
    }
    levels
}

impl Level {
    fn centre(&self) -> [f64; 2] {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in self.faces.iter().flatten() {
            min = [min[0].min(point.x), min[1].min(point.y)];
            max = [max[0].max(point.x), max[1].max(point.y)];
        }
        [f64::midpoint(min[0], max[0]), f64::midpoint(min[1], max[1])]
    }
}

impl WalkingSurfaceService for AxiolidWalkingSurfaceService {
    fn measure_tread_flight(&self, object: &ObjectId) -> Result<TreadFlight, WalkingSurfaceError> {
        let solid = self.solid(object)?;
        if component_count(solid.mesh) != 1 {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "{object} is in several pieces (open risers or separate treads); its first \
                 riser needs the floor it starts from, which it does not carry"
            )));
        }
        let mut level_faces: Vec<Triangle> = Vec::new();
        for triangle in &solid.soup {
            match facing(object, triangle)? {
                Facing::Level => level_faces.push(*triangle),
                Facing::Sloped => {
                    return Err(WalkingSurfaceError::Unsupported(format!(
                        "{object} has a sloped face looking up, which is no tread"
                    )));
                }
                Facing::Other => {}
            }
        }
        let levels = group_levels(level_faces);
        let (Some(lowest), Some(highest)) = (levels.first(), levels.last()) else {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "{object} has no horizontal face looking up, so no tread"
            )));
        };
        if levels.len() < 2 {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "{object} has a single tread, which gives no walking direction"
            )));
        }
        let origin = lowest.centre();
        let direction = plan_direction(origin, highest.centre())?;
        let [ux, uy, _] = direction.components();
        let mut along = f64::NEG_INFINITY;
        for level in &levels {
            let centre = level.centre();
            let (dx, dy) = (centre[0] - origin[0], centre[1] - origin[1]);
            let lateral = (dx * uy - dy * ux).abs();
            let ahead = dx * ux + dy * uy;
            if lateral > STRAIGHT_TOLERANCE || ahead <= along {
                return Err(WalkingSurfaceError::Unsupported(format!(
                    "the treads of {object} do not follow one straight line: winders or a \
                     turning flight are not measured"
                )));
            }
            along = ahead;
        }
        let projection = Projection::new(direction);
        let frame = PlanFrame::new(direction);
        let treads = levels
            .iter()
            .map(|level| {
                let (front, back) = projection.span(level.faces.iter().flatten())?;
                let elevation = ElevationInterval::try_new(level.low, level.high)
                    .map_err(|_| WalkingSurfaceError::InvalidMeasurement)?;
                let tread = Tread::try_new(elevation, front, back)?;
                match rectangle(&level.faces, &frame)? {
                    Some([_, (left, right)]) => tread.with_sides(left, right),
                    None => Ok(tread),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (base, top) = solid
            .soup
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                (low.min(point.z), high.max(point.z))
            });
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!("tread-flight:{object}"),
            exact: treads.iter().all(Tread::is_exact),
        };
        TreadFlight::try_new(
            object.clone(),
            direction,
            exact(base)?,
            exact(top)?,
            treads,
            evidence,
        )
    }

    fn measure_sloped_runs(&self, object: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError> {
        let solid = self.solid(object)?;
        let mut sloped: Vec<usize> = Vec::new();
        for (index, triangle) in solid.soup.iter().enumerate() {
            if facing(object, triangle)? == Facing::Sloped {
                sloped.push(index);
            }
        }
        if sloped.is_empty() {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "{object} has no sloped face looking up, so no ramp run"
            )));
        }
        let mut runs = Vec::new();
        for component in connected(solid.mesh, &sloped) {
            let faces: Vec<Triangle> = component.iter().map(|index| solid.soup[*index]).collect();
            runs.push(run(object, &faces)?);
        }
        runs.sort_by(|a, b| {
            a.bottom()
                .lower_metres()
                .total_cmp(&b.bottom().lower_metres())
        });
        let exact = runs.iter().all(SlopedRun::is_exact);
        let evidence = Evidence {
            source: object.source.clone(),
            locator: format!("sloped-runs:{object}"),
            exact,
        };
        SlopedSurface::try_new(object.clone(), runs, evidence)
    }

    fn measure_headroom(&self, request: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError> {
        let subject = request.subject();
        let solid = self.solid(subject)?;
        let mut walking = Vec::new();
        for triangle in &solid.soup {
            if facing(subject, triangle)? != Facing::Other {
                walking.push(*triangle);
            }
        }
        if walking.is_empty() {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "{subject} has no walking face looking up"
            )));
        }
        let reach = plan_box(walking.iter());
        let reach_bottom = walking
            .iter()
            .flatten()
            .fold(f64::INFINITY, |low, point| low.min(point.z));
        let mut clearances: Vec<(ObjectId, MeasuredInterval)> = Vec::new();
        for obstacle in request.obstacles() {
            if self.geometry.has_no_body(obstacle) {
                continue;
            }
            if let Some((_, reason)) = self
                .geometry
                .unmeasured()
                .find(|(unmeasured, _)| *unmeasured == obstacle)
            {
                return Err(WalkingSurfaceError::Unavailable(format!(
                    "obstacle {obstacle} has a body that was not measured: {reason}"
                )));
            }
            let mesh = self
                .geometry
                .mesh(obstacle)
                .ok_or_else(|| WalkingSurfaceError::UnknownObject(obstacle.clone()))?;
            // The mesh box, grown by a tessellation's chord deviation so it
            // encloses the true body.
            let Some((min, max)) = self.geometry.enclosing_extent(obstacle) else {
                if mesh_extent(mesh).is_none() {
                    continue;
                }
                return Err(WalkingSurfaceError::Unavailable(format!(
                    "obstacle {obstacle} has an invalid chord deviation"
                )));
            };
            if max[0] < reach.0[0]
                || min[0] > reach.1[0]
                || max[1] < reach.0[1]
                || min[1] > reach.1[1]
                || max[2] < reach_bottom
            {
                continue;
            }
            if self.geometry.is_tessellated(obstacle) {
                return Err(WalkingSurfaceError::InexactGeometry(format!(
                    "obstacle {obstacle} near {subject} is a tessellation of curved faces"
                )));
            }
            let faces = triangles(mesh);
            let Some((low, high, margin)) = gaps(&walking, &faces) else {
                continue;
            };
            if high <= margin {
                // Wholly below the walking surface, or resting on it flush.
                continue;
            }
            if low < -margin {
                return Err(WalkingSurfaceError::Unsupported(format!(
                    "obstacle {obstacle} crosses the walking surface of {subject}"
                )));
            }
            let clearance =
                MeasuredInterval::try_new((low - margin).max(0.0), low.max(0.0) + margin)?;
            clearances.push((obstacle.clone(), clearance));
        }
        let least =
            clearances
                .iter()
                .map(|(_, clearance)| *clearance)
                .reduce(|least, clearance| {
                    MeasuredInterval::try_new(
                        least.lower().min(clearance.lower()),
                        least.upper().min(clearance.upper()),
                    )
                    .unwrap_or(least)
                });
        let governing: Vec<ObjectId> = least.map_or_else(Vec::new, |least| {
            clearances
                .iter()
                .filter(|(_, clearance)| clearance.lower() <= least.upper())
                .map(|(obstacle, _)| obstacle.clone())
                .collect()
        });
        let evidence = Evidence {
            source: subject.source.clone(),
            locator: format!("headroom:{subject}"),
            exact: false,
        };
        Headroom::try_new(request.clone(), least, governing, evidence)
    }

    fn measure_landing(
        &self,
        request: &LandingRequest,
    ) -> Result<LandingEvidence, WalkingSurfaceError> {
        let subject = request.subject();
        let end = self.walking_end(subject, request.end())?;
        let frame = PlanFrame::new(end.direction);
        let mut carriers: Vec<&ObjectId> = request.candidates().iter().collect();
        if end.ramp {
            carriers.push(subject);
        }
        let mut found: Vec<(ObjectId, Vec<Triangle>)> = Vec::new();
        for carrier in carriers {
            for surface in self.meeting_surfaces(carrier, subject, &end, &frame)? {
                found.push((carrier.clone(), surface));
            }
        }
        if found.len() > 1 {
            let names = found
                .iter()
                .map(|(carrier, _)| carrier.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(WalkingSurfaceError::Unsupported(format!(
                "several level surfaces meet the end of {subject} ({names}); their union is not \
                 measured"
            )));
        }
        let landing =
            found.pop().map(|(carrier, faces)| {
                let extent = rectangle(&faces, &frame).ok().flatten().and_then(
                    |[(_, far), (left, right)]| {
                        if far.lower_metres() > end.edge.upper_metres() {
                            LandingExtent::try_new(far, left, right).ok()
                        } else {
                            None
                        }
                    },
                );
                Landing::new(carrier, extent)
            });
        let exact = end.edge.is_exact()
            && landing
                .as_ref()
                .and_then(Landing::extent)
                .is_none_or(|extent| {
                    let (left, right) = extent.sides();
                    extent.far().is_exact() && left.is_exact() && right.is_exact()
                });
        let evidence = Evidence {
            source: subject.source.clone(),
            locator: format!("landing:{subject}"),
            exact,
        };
        LandingEvidence::try_new(request.clone(), end.direction, end.edge, landing, evidence)
    }

    fn measure_clearance_below(
        &self,
        request: &ClearanceBelowRequest,
    ) -> Result<ClearanceBelow, WalkingSurfaceError> {
        let subject = request.subject();
        let solid = self.solid(subject)?;
        let mut under = Vec::new();
        for triangle in &solid.soup {
            let (cross, bound) = plan_cross(triangle);
            if cross < -bound {
                under.push(*triangle);
            }
        }
        let reach = plan_box(under.iter());
        let mut clearances: Vec<(ObjectId, MeasuredInterval)> = Vec::new();
        for space in request.spaces() {
            if self.geometry.has_no_body(space) {
                continue;
            }
            if let Some((_, reason)) = self
                .geometry
                .unmeasured()
                .find(|(unmeasured, _)| *unmeasured == space)
            {
                return Err(WalkingSurfaceError::Unavailable(format!(
                    "space {space} has a body that was not measured: {reason}"
                )));
            }
            let mesh = self
                .geometry
                .mesh(space)
                .ok_or_else(|| WalkingSurfaceError::UnknownObject(space.clone()))?;
            let Some((min, max)) = self.geometry.enclosing_extent(space) else {
                if mesh_extent(mesh).is_none() {
                    continue;
                }
                return Err(WalkingSurfaceError::Unavailable(format!(
                    "space {space} has an invalid chord deviation"
                )));
            };
            if under.is_empty()
                || max[0] < reach.0[0]
                || min[0] > reach.1[0]
                || max[1] < reach.0[1]
                || min[1] > reach.1[1]
            {
                continue;
            }
            let floor_solid = self.solid(space)?;
            let floor: Vec<Triangle> = floor_solid
                .soup
                .iter()
                .filter(|triangle| {
                    let (cross, bound) = plan_cross(triangle);
                    cross < -bound && is_level(triangle)
                })
                .copied()
                .collect();
            let Some((low, margin)) = below(subject, space, &floor, &under)? else {
                continue;
            };
            let clearance =
                MeasuredInterval::try_new((low - margin).max(0.0), low.max(0.0) + margin)?;
            clearances.push((space.clone(), clearance));
        }
        let least =
            clearances
                .iter()
                .map(|(_, clearance)| *clearance)
                .reduce(|least, clearance| {
                    MeasuredInterval::try_new(
                        least.lower().min(clearance.lower()),
                        least.upper().min(clearance.upper()),
                    )
                    .unwrap_or(least)
                });
        let governing: Vec<ObjectId> = least.map_or_else(Vec::new, |least| {
            clearances
                .iter()
                .filter(|(_, clearance)| clearance.lower() <= least.upper())
                .map(|(space, _)| space.clone())
                .collect()
        });
        let evidence = Evidence {
            source: subject.source.clone(),
            locator: format!("clearance-below:{subject}"),
            exact: false,
        };
        ClearanceBelow::try_new(request.clone(), least, governing, evidence)
    }
}

/// The triangles of `faces` (indices into the mesh) connected through
/// shared mesh edges, each component in ascending index order.
fn connected(mesh: &TriMesh, faces: &[usize]) -> Vec<Vec<usize>> {
    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let mut edges: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    for (position, face) in faces.iter().enumerate() {
        let corners = mesh.triangle(*face);
        for i in 0..3 {
            let (from, to) = (corners[i], corners[(i + 1) % 3]);
            let key = (from.min(to), from.max(to));
            if let Some(other) = edges.insert(key, position) {
                let (left, right) = (find(&mut parent, other), find(&mut parent, position));
                parent[left.max(right)] = left.min(right);
            }
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (position, face) in faces.iter().enumerate() {
        let root = find(&mut parent, position);
        components.entry(root).or_default().push(*face);
    }
    components.into_values().collect()
}

/// One planar sloped run.
#[allow(clippy::float_cmp)]
fn run(object: &ObjectId, faces: &[Triangle]) -> Result<SlopedRun, WalkingSurfaceError> {
    let reference = faces
        .iter()
        .max_by(|a, b| {
            let area = |t: &Triangle| {
                let [x, y, z] = normal(t);
                x.hypot(y).hypot(z)
            };
            area(a).total_cmp(&area(b))
        })
        .ok_or(WalkingSurfaceError::InvalidMeasurement)?;
    let [nx, ny, nz] = normal(reference);
    let length = nx.hypot(ny).hypot(nz);
    let scale = faces.iter().flatten().fold(1.0_f64, |scale, p| {
        scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
    });
    let origin = reference[0];
    for point in faces.iter().flatten() {
        let offset = *point - origin;
        let distance = (nx * offset.x + ny * offset.y + nz * offset.z).abs() / length;
        if distance > PLANAR_TOLERANCE * scale {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "a sloped run of {object} is not planar"
            )));
        }
    }
    // Along an edge with one plan coordinate fixed, the height changes only
    // with the other: no change at all proves the plane level that way, so
    // the run climbs exactly along a coordinate axis.
    let level_along = |fixed: usize| {
        faces.iter().any(|face| {
            (0..3).any(|i| {
                let (p, q) = (face[i].to_array(), face[(i + 1) % 3].to_array());
                p[fixed] == q[fixed] && p[1 - fixed] != q[1 - fixed] && p[2] == q[2]
            })
        })
    };
    // The plane rises against the plan part of its outward normal.
    let vector = if level_along(0) && level_along(1) {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    } else if level_along(0) {
        // Level along y: climbs along x.
        [-nx.signum(), 0.0, 0.0]
    } else if level_along(1) {
        [0.0, -ny.signum(), 0.0]
    } else {
        [-nx, -ny, 0.0]
    };
    let direction =
        MetricDirection::try_new(vector).map_err(|_| WalkingSurfaceError::InvalidMeasurement)?;
    let (start, end) = Projection::new(direction).span(faces.iter().flatten())?;
    let (bottom, top) = faces
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
            (low.min(point.z), high.max(point.z))
        });
    let run = SlopedRun::try_new(direction, exact(bottom)?, exact(top)?, start, end)?;
    match rectangle(faces, &PlanFrame::new(direction))? {
        Some([_, (left, right)]) => run.with_sides(left, right),
        None => Ok(run),
    }
}

/// The plan bounding box of some triangles.
fn plan_box<'t>(faces: impl Iterator<Item = &'t Triangle>) -> ([f64; 2], [f64; 2]) {
    faces.flatten().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(min, max), p| {
            (
                [min[0].min(p.x), min[1].min(p.y)],
                [max[0].max(p.x), max[1].max(p.y)],
            )
        },
    )
}

/// The height of `triangle`'s plane over plan point `(x, y)`.
fn height(triangle: &Triangle, x: f64, y: f64) -> f64 {
    let [nx, ny, nz] = normal(triangle);
    let a = triangle[0];
    a.z - (nx * (x - a.x) + ny * (y - a.y)) / nz
}

/// The least and greatest height of `obstacle`'s faces above the walking
/// faces, over the plan regions they share with positive area, and the
/// numerical margin on both. `None` when they share none.
fn gaps(walking: &[Triangle], obstacle: &[Triangle]) -> Option<(f64, f64, f64)> {
    let mut found: Option<(f64, f64, f64)> = None;
    for over in obstacle {
        let (cross, bound) = plan_cross(over);
        if cross.abs() <= bound {
            // Edge-on in plan: its lowest points lie on faces that are not.
            continue;
        }
        let clip: Vec<[f64; 2]> = if cross > 0.0 {
            over.iter().map(|p| [p.x, p.y]).collect()
        } else {
            over.iter().rev().map(|p| [p.x, p.y]).collect()
        };
        let slope_over = gradient(over);
        for under in walking {
            let polygon = clip_convex(under.iter().map(|p| [p.x, p.y]).collect(), &clip);
            let scale = over.iter().chain(under.iter()).fold(1.0_f64, |scale, p| {
                scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
            });
            if polygon.len() < 3 || area(&polygon) <= 1e-12 * scale * scale {
                continue;
            }
            let margin = HEADROOM_MARGIN * scale * (1.0 + slope_over + gradient(under));
            for [x, y] in polygon {
                let gap = height(over, x, y) - height(under, x, y);
                found = Some(match found {
                    None => (gap, gap, margin),
                    Some((low, high, most)) => (low.min(gap), high.max(gap), most.max(margin)),
                });
            }
        }
    }
    found
}

/// Twice the signed area of a plan polygon.
fn area(polygon: &[[f64; 2]]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| {
            let (p, q) = (polygon[i], polygon[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

/// `subject` clipped to the convex counter-clockwise polygon `clip`
/// (Sutherland–Hodgman).
fn clip_convex(subject: Vec<[f64; 2]>, clip: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut output = subject;
    for (index, start) in clip.iter().enumerate() {
        let end = clip[(index + 1) % clip.len()];
        let side = |point: [f64; 2]| {
            (end[0] - start[0]) * (point[1] - start[1])
                - (end[1] - start[1]) * (point[0] - start[0])
        };
        let input = std::mem::take(&mut output);
        for (corner, from) in input.iter().enumerate() {
            let to = input[(corner + 1) % input.len()];
            let (inside_from, inside_to) = (side(*from), side(to));
            if inside_from >= 0.0 {
                output.push(*from);
            }
            if (inside_from >= 0.0) != (inside_to >= 0.0) {
                let share = inside_from / (inside_from - inside_to);
                output.push([
                    from[0] + share * (to[0] - from[0]),
                    from[1] + share * (to[1] - from[1]),
                ]);
            }
        }
        if output.is_empty() {
            break;
        }
    }
    output
}

/// Plan positions along a horizontal direction and across it (a quarter
/// turn anticlockwise), as [`axioval_engine::across`] defines it.
struct PlanFrame {
    along: [f64; 2],
    across: [f64; 2],
    on_axis: bool,
}

impl PlanFrame {
    #[allow(clippy::float_cmp)]
    fn new(direction: MetricDirection) -> Self {
        let [x, y, _] = direction.components();
        Self {
            along: [x, y],
            across: [-y, x],
            on_axis: x == 0.0 || y == 0.0,
        }
    }

    /// A point's positions along and across the direction. Along a
    /// coordinate axis each reads one coordinate exactly.
    fn map(&self, x: f64, y: f64) -> [f64; 2] {
        [
            x * self.along[0] + y * self.along[1],
            x * self.across[0] + y * self.across[1],
        ]
    }

    /// A bound on the rounding of [`Self::map`] at a point, as
    /// [`Projection::span`] bounds it.
    fn rounding(&self, x: f64, y: f64) -> f64 {
        if self.on_axis {
            0.0
        } else {
            8.0 * f64::EPSILON * (x.abs() + y.abs()).max(f64::MIN_POSITIVE)
        }
    }
}

/// The direction opposite `direction`.
fn reversed(direction: MetricDirection) -> Result<MetricDirection, WalkingSurfaceError> {
    let [x, y, z] = direction.components();
    MetricDirection::try_new([-x, -y, -z]).map_err(|_| WalkingSurfaceError::InvalidMeasurement)
}

/// A position measured along the opposite direction.
fn negated(position: ElevationInterval) -> Result<ElevationInterval, WalkingSurfaceError> {
    ElevationInterval::try_new(-position.upper_metres(), -position.lower_metres())
        .map_err(|_| WalkingSurfaceError::InvalidMeasurement)
}

/// A position interval from sure bounds.
fn interval((lower, upper): (f64, f64)) -> Result<ElevationInterval, WalkingSurfaceError> {
    ElevationInterval::try_new(lower, upper).map_err(|_| WalkingSurfaceError::InvalidMeasurement)
}

/// A point's key for matching edges between faces: its exact coordinates,
/// with negative zero read as zero.
fn key(point: &Point3) -> [u64; 3] {
    [point.x + 0.0, point.y + 0.0, point.z + 0.0].map(f64::to_bits)
}

/// An edge by the exact coordinates of its ends, in order.
type EdgeKey = ([u64; 3], [u64; 3]);

/// How many faces share an edge, and its ends.
type EdgeUse = (usize, Point3, Point3);

/// The edges of `faces` by the exact coordinates of their ends, with how
/// many faces share each.
fn edge_counts(faces: &[Triangle]) -> BTreeMap<EdgeKey, EdgeUse> {
    let mut edges: BTreeMap<EdgeKey, EdgeUse> = BTreeMap::new();
    for face in faces {
        for i in 0..3 {
            let (a, b) = (face[i], face[(i + 1) % 3]);
            let (ka, kb) = (key(&a), key(&b));
            let entry = edges.entry((ka.min(kb), ka.max(kb))).or_insert((0, a, b));
            entry.0 += 1;
        }
    }
    edges
}

/// The rectangle `faces` fill along and across `frame`: the `(lowest,
/// highest)` positions along it and across it, when every edge on the
/// boundary of the faces lies on one of the rectangle's sides and their plan
/// area is the rectangle's; `None` otherwise.
///
/// A region whose boundary lies on a rectangle's sides is that rectangle, so
/// its width holds all along its length. Along a coordinate axis positions
/// are exact and an edge must lie exactly on a side; along any other
/// direction both are widened by the rounding of the projection.
fn rectangle(
    faces: &[Triangle],
    frame: &PlanFrame,
) -> Result<Option<[(ElevationInterval, ElevationInterval); 2]>, WalkingSurfaceError> {
    if faces.is_empty() {
        return Ok(None);
    }
    let mut lowest = [(f64::INFINITY, f64::INFINITY); 2];
    let mut highest = [(f64::NEG_INFINITY, f64::NEG_INFINITY); 2];
    let mut scale = 1.0_f64;
    for point in faces.iter().flatten() {
        let mapped = frame.map(point.x, point.y);
        let rounding = frame.rounding(point.x, point.y);
        scale = scale.max(point.x.abs()).max(point.y.abs());
        for axis in 0..2 {
            let (low, high) = (mapped[axis] - rounding, mapped[axis] + rounding);
            lowest[axis] = (lowest[axis].0.min(low), lowest[axis].1.min(high));
            highest[axis] = (highest[axis].0.max(low), highest[axis].1.max(high));
        }
    }
    let tolerance = if frame.on_axis {
        0.0
    } else {
        32.0 * f64::EPSILON * scale
    };
    let on =
        |value: f64, (low, high): (f64, f64)| value >= low - tolerance && value <= high + tolerance;
    for (count, a, b) in edge_counts(faces).into_values() {
        if count > 2 {
            return Ok(None);
        }
        if count == 2 {
            continue;
        }
        let (a, b) = (frame.map(a.x, a.y), frame.map(b.x, b.y));
        let on_side = (0..2).any(|axis| {
            [lowest[axis], highest[axis]]
                .iter()
                .any(|side| on(a[axis], *side) && on(b[axis], *side))
        });
        if !on_side {
            return Ok(None);
        }
    }
    let middle = |(low, high): (f64, f64)| f64::midpoint(low, high);
    let rectangle =
        (middle(highest[0]) - middle(lowest[0])) * (middle(highest[1]) - middle(lowest[1]));
    let covered: f64 = faces
        .iter()
        .map(|face| plan_cross(face).0.abs() / 2.0)
        .sum();
    if (covered - rectangle).abs() > 1e-9 * rectangle + 1e-12 * scale * scale {
        return Ok(None);
    }
    Ok(Some([
        (interval(lowest[0])?, interval(highest[0])?),
        (interval(lowest[1])?, interval(highest[1])?),
    ]))
}

/// Where a landing is looked for at one end of a flight or run.
struct EndGeometry {
    /// Whether the subject is a ramp, which may carry its own landings.
    ramp: bool,
    /// The horizontal direction leaving the subject at the end.
    direction: MetricDirection,
    /// The end's arrival line along `direction`.
    edge: ElevationInterval,
    /// The elevation of the walking surface at the end.
    elevation: ElevationInterval,
    /// The stretch along `direction` a landing must reach into: from where
    /// the walking surface ends to where the body ends.
    reach: (f64, f64),
    /// The end's sure width across `direction`.
    across: (f64, f64),
}

/// The positions across a leaving direction of sides measured across the
/// opposite one: the sure interior between them.
fn sides_across(
    sides: Option<(ElevationInterval, ElevationInterval)>,
    reverse: bool,
    subject: &ObjectId,
) -> Result<(f64, f64), WalkingSurfaceError> {
    let Some((left, right)) = sides else {
        return Err(WalkingSurfaceError::Unsupported(format!(
            "the end of {subject} fills no rectangle, so its width is not measured"
        )));
    };
    Ok(if reverse {
        (-right.lower_metres(), -left.upper_metres())
    } else {
        (left.upper_metres(), right.lower_metres())
    })
}

impl AxiolidWalkingSurfaceService {
    fn walking_end(
        &self,
        subject: &ObjectId,
        end: WalkingEnd,
    ) -> Result<EndGeometry, WalkingSurfaceError> {
        match end {
            WalkingEnd::FlightBottom | WalkingEnd::FlightTop => {
                let flight = self.measure_tread_flight(subject)?;
                let (Some(first), Some(last)) = (flight.treads().first(), flight.treads().last())
                else {
                    return Err(WalkingSurfaceError::InvalidMeasurement);
                };
                let bottom = end == WalkingEnd::FlightBottom;
                let direction = if bottom {
                    reversed(flight.direction())?
                } else {
                    flight.direction()
                };
                // Where the body ends along the leaving direction.
                let frame = PlanFrame::new(direction);
                let solid = self.solid(subject)?;
                let body_end = solid
                    .soup
                    .iter()
                    .flatten()
                    .fold(f64::NEG_INFINITY, |end, p| {
                        end.max(frame.map(p.x, p.y)[0] + frame.rounding(p.x, p.y))
                    });
                if bottom {
                    let edge = negated(first.front())?;
                    Ok(EndGeometry {
                        ramp: false,
                        direction,
                        edge,
                        elevation: flight.base(),
                        reach: (edge.lower_metres(), body_end),
                        across: sides_across(first.sides(), true, subject)?,
                    })
                } else {
                    let (edge, elevation) = if flight.ends_in_riser() {
                        (last.back(), flight.top())
                    } else {
                        (last.front(), last.elevation())
                    };
                    Ok(EndGeometry {
                        ramp: false,
                        direction,
                        edge,
                        elevation,
                        reach: (last.back().lower_metres(), body_end),
                        across: sides_across(last.sides(), false, subject)?,
                    })
                }
            }
            WalkingEnd::RunBottom(index) | WalkingEnd::RunTop(index) => {
                let ramp = self.measure_sloped_runs(subject)?;
                let run = ramp.runs().get(index).ok_or_else(|| {
                    WalkingSurfaceError::Unsupported(format!(
                        "{subject} has no run {} of {}",
                        index + 1,
                        ramp.runs().len()
                    ))
                })?;
                let bottom = matches!(end, WalkingEnd::RunBottom(_));
                let (direction, edge, elevation) = if bottom {
                    (
                        reversed(run.direction())?,
                        negated(run.start())?,
                        run.bottom(),
                    )
                } else {
                    (run.direction(), run.end(), run.top())
                };
                Ok(EndGeometry {
                    ramp: true,
                    direction,
                    edge,
                    elevation,
                    reach: (edge.lower_metres(), edge.upper_metres()),
                    across: sides_across(run.sides(), bottom, subject)?,
                })
            }
        }
    }

    /// The connected level surfaces of `carrier` at the end's elevation that
    /// reach into the end's strip.
    fn meeting_surfaces(
        &self,
        carrier: &ObjectId,
        subject: &ObjectId,
        end: &EndGeometry,
        frame: &PlanFrame,
    ) -> Result<Vec<Vec<Triangle>>, WalkingSurfaceError> {
        if self.geometry.has_no_body(carrier) {
            return Ok(Vec::new());
        }
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == carrier)
        {
            return Err(WalkingSurfaceError::Unavailable(format!(
                "candidate landing {carrier} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self
            .geometry
            .mesh(carrier)
            .ok_or_else(|| WalkingSurfaceError::UnknownObject(carrier.clone()))?;
        let Some((min, max)) = self.geometry.enclosing_extent(carrier) else {
            if mesh_extent(mesh).is_none() {
                return Ok(Vec::new());
            }
            return Err(WalkingSurfaceError::Unavailable(format!(
                "candidate landing {carrier} has an invalid chord deviation"
            )));
        };
        let scale = end.elevation.upper_metres().abs().max(1.0);
        let level = LEVEL_TOLERANCE * scale;
        let (low, high) = (
            end.elevation.lower_metres() - level,
            end.elevation.upper_metres() + level,
        );
        if max[2] < low || min[2] > high {
            return Ok(Vec::new());
        }
        // The extent's plan corners along and across the leaving direction.
        let strip = (
            (end.reach.0 - LANDING_REACH, end.reach.1 + LANDING_REACH),
            end.across,
        );
        let corners = [
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ]
        .map(|[x, y]| frame.map(x, y));
        let apart = (0..2).any(|axis| {
            let (from, to) = if axis == 0 { strip.0 } else { strip.1 };
            let slack = 1e-9 * scale;
            corners.iter().all(|corner| corner[axis] < from - slack)
                || corners.iter().all(|corner| corner[axis] > to + slack)
        });
        if apart {
            return Ok(Vec::new());
        }
        if self.geometry.is_tessellated(carrier) {
            return Err(WalkingSurfaceError::InexactGeometry(format!(
                "candidate landing {carrier} at the end of {subject} is a tessellation of curved \
                 faces"
            )));
        }
        let solid = self.solid(carrier)?;
        let mut faces = Vec::new();
        for triangle in &solid.soup {
            if facing(carrier, triangle)? != Facing::Level {
                continue;
            }
            let (bottom, top) = elevations(triangle);
            if bottom >= low && top <= high {
                faces.push(*triangle);
            }
        }
        Ok(components(&faces)
            .into_iter()
            .filter(|surface| touches(surface, frame, strip.0, strip.1))
            .collect())
    }
}

/// `faces` split into sets connected through shared edges, matched by the
/// exact coordinates of their ends.
fn components(faces: &[Triangle]) -> Vec<Vec<Triangle>> {
    fn find(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    let mut edges: BTreeMap<([u64; 3], [u64; 3]), usize> = BTreeMap::new();
    for (position, face) in faces.iter().enumerate() {
        for i in 0..3 {
            let (a, b) = (key(&face[i]), key(&face[(i + 1) % 3]));
            if let Some(other) = edges.insert((a.min(b), a.max(b)), position) {
                let (left, right) = (find(&mut parent, other), find(&mut parent, position));
                parent[left.max(right)] = left.min(right);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<Triangle>> = BTreeMap::new();
    for (position, face) in faces.iter().enumerate() {
        let root = find(&mut parent, position);
        groups.entry(root).or_default().push(*face);
    }
    groups.into_values().collect()
}

/// Whether some face covers part of the strip between `along` and `across`
/// positions with positive plan area.
fn touches(faces: &[Triangle], frame: &PlanFrame, along: (f64, f64), across: (f64, f64)) -> bool {
    if along.0 >= along.1 || across.0 >= across.1 {
        return false;
    }
    let strip = [
        [along.0, across.0],
        [along.1, across.0],
        [along.1, across.1],
        [along.0, across.1],
    ];
    let least = 1e-6 * (along.1 - along.0) * (across.1 - across.0);
    faces.iter().any(|face| {
        let polygon = counter_clockwise(face.iter().map(|p| frame.map(p.x, p.y)).collect());
        let clipped = clip_convex(polygon, &strip);
        clipped.len() >= 3 && area(&clipped) > least
    })
}

/// A polygon's corners in counter-clockwise order.
fn counter_clockwise(mut polygon: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let n = polygon.len();
    let signed: f64 = (0..n)
        .map(|i| {
            let (p, q) = (polygon[i], polygon[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum();
    if signed < 0.0 {
        polygon.reverse();
    }
    polygon
}

/// The part of `polygon` on the left of the line from `a` to `b` (`left`),
/// or on its right.
fn half_plane(polygon: &[[f64; 2]], a: [f64; 2], b: [f64; 2], left: bool) -> Vec<[f64; 2]> {
    let side = |point: [f64; 2]| {
        let value = (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0]);
        if left { value } else { -value }
    };
    let mut output = Vec::new();
    for (corner, from) in polygon.iter().enumerate() {
        let to = polygon[(corner + 1) % polygon.len()];
        let (inside_from, inside_to) = (side(*from), side(to));
        if inside_from >= 0.0 {
            output.push(*from);
        }
        if (inside_from >= 0.0) != (inside_to >= 0.0) {
            let share = inside_from / (inside_from - inside_to);
            output.push([
                from[0] + share * (to[0] - from[0]),
                from[1] + share * (to[1] - from[1]),
            ]);
        }
    }
    output
}

/// `polygon` less the convex counter-clockwise `hole`, as convex pieces.
fn subtract(polygon: Vec<[f64; 2]>, hole: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
    let mut pieces = Vec::new();
    let mut rest = polygon;
    for (index, a) in hole.iter().enumerate() {
        let b = hole[(index + 1) % hole.len()];
        let outside = half_plane(&rest, *a, b, false);
        if outside.len() >= 3 {
            pieces.push(outside);
        }
        rest = half_plane(&rest, *a, b, true);
        if rest.len() < 3 {
            return pieces;
        }
    }
    // What remains lies inside the hole.
    pieces
}

/// The least height of the `under` faces above the `floor` faces, over the
/// plan regions they share with positive area and away from where an
/// `under` face lies flush on the floor, and the numerical margin on it.
/// `None` when they share no such region; refused when the subject crosses
/// the floor.
fn below(
    subject: &ObjectId,
    space: &ObjectId,
    floor: &[Triangle],
    under: &[Triangle],
) -> Result<Option<(f64, f64)>, WalkingSurfaceError> {
    let plan = |t: &Triangle| counter_clockwise(t.iter().map(|p| [p.x, p.y]).collect());
    let mut found: Option<(f64, f64)> = None;
    for ground in floor {
        let ground_plan = plan(ground);
        let mut flush: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut open: Vec<(&Triangle, Vec<[f64; 2]>, f64, f64)> = Vec::new();
        for face in under {
            let polygon = clip_convex(ground_plan.clone(), &plan(face));
            let scale = face.iter().chain(ground.iter()).fold(1.0_f64, |scale, p| {
                scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs())
            });
            let least = 1e-12 * scale * scale;
            if polygon.len() < 3 || area(&polygon) <= least {
                continue;
            }
            let margin = HEADROOM_MARGIN * scale * (1.0 + gradient(face) + gradient(ground));
            let flat = polygon
                .iter()
                .all(|[x, y]| (height(face, *x, *y) - height(ground, *x, *y)).abs() <= margin);
            if flat {
                flush.push(polygon);
            } else {
                open.push((face, polygon, margin, least));
            }
        }
        for (face, polygon, margin, least) in open {
            let mut pieces = vec![polygon];
            for hole in &flush {
                pieces = pieces
                    .into_iter()
                    .flat_map(|piece| subtract(piece, hole))
                    .collect();
            }
            for piece in pieces {
                if area(&piece) <= least {
                    continue;
                }
                let gaps = piece
                    .iter()
                    .map(|[x, y]| height(face, *x, *y) - height(ground, *x, *y));
                let (low, high) = gaps.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), g| {
                    (l.min(g), h.max(g))
                });
                if high <= margin {
                    // Below the floor, or flush on it.
                    continue;
                }
                if low < -margin {
                    return Err(WalkingSurfaceError::Unsupported(format!(
                        "{subject} crosses the floor of {space}"
                    )));
                }
                found = Some(match found {
                    None => (low, margin),
                    Some((least, most)) => (least.min(low), most.max(margin)),
                });
            }
        }
    }
    Ok(found)
}
