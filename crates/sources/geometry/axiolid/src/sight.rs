//! Lines of sight from an eye point to a model object, past blockers.
//!
//! ADR 0004: this module proves what is in view; how many targets must be,
//! and from where, is a rule's decision.
//!
//! The kernel's `line_of_sight` decides each question exactly. A **visible**
//! answer is one ray, checked with exact predicates, that crosses a target
//! triangle before any blocker triangle. A **hidden** answer covers every
//! ray to the target by single blocker pieces: a triangle, two coplanar
//! triangles of one mesh forming a convex quadrilateral (a wall face), or a
//! whole closed convex blocker (a column). A target covered only where two
//! separate blockers meet stays **undecided**; the kernel never guesses.
//!
//! Only exact meshes are looked at: a tessellated target or blocker could
//! be seen, or see past, where its mesh says otherwise, so the request
//! refuses once either could matter (a tessellated target's distance is
//! still measured, widened by its chord deviation). A blocker whose body was not measured may hide anything, so it
//! refuses too; a declared bodiless blocker hides nothing and is skipped. A
//! blocker whose box lies apart from the box holding the eye and the target
//! cannot meet any segment between them, so it is not handed over.
//!
//! The distance to the target is the least distance from the eye to its
//! triangles, widened by a rounding bound, so a target is left unlooked-at
//! only when it lies surely beyond the requested range.

use axiolid_core::{Point3, Vec3};
use axiolid_inspect::{Sight, contains, line_of_sight};
use axiolid_measure::{
    closest_point_on_triangle, closest_points_on_segments, closest_points_on_triangles,
};
use axiolid_mesh::TriMesh;
use axioval_engine::{
    SightError, SightEvidence, SightOutcome, SightRequest, SightService, SkyObstruction,
    SkyObstructionRequest,
};
use axioval_ir::{Evidence, ObjectId};
use std::f64::consts::FRAC_PI_2;

use crate::geometry::{AxiolidGeometry, Extent, Triangle, mesh_extent, triangles};

/// Relative rounding bound on a measured distance, and its absolute floor.
const DISTANCE_SLACK: f64 = 1e-9;

/// Assesses lines of sight over registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidSightService {
    geometry: AxiolidGeometry,
}

impl AxiolidSightService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self { geometry }
    }

    /// The mesh of an object asked about, or why it cannot be used.
    fn mesh(&self, object: &ObjectId, role: &str) -> Result<&TriMesh, SightError> {
        if let Some((_, reason)) = self.geometry.unmeasured_of(object) {
            return Err(SightError::Unavailable(format!(
                "{role} {object} has a body that was not measured: {reason}"
            )));
        }
        self.geometry
            .mesh(object)
            .ok_or_else(|| SightError::UnknownObject(object.clone()))
    }

    /// Refuses a tessellation, whose mesh is not the object's shape.
    fn exact(&self, object: &ObjectId, role: &str) -> Result<(), SightError> {
        if self.geometry.is_tessellated(object) {
            return Err(SightError::Unavailable(format!(
                "{role} {object} is a tessellation, so its mesh is not its shape"
            )));
        }
        Ok(())
    }
}

impl SightService for AxiolidSightService {
    fn assess_sight(&self, request: &SightRequest) -> Result<SightEvidence, SightError> {
        let target = request.target();
        if self.geometry.has_no_body(target) {
            return Err(SightError::Unavailable(format!(
                "target {target} has no body to be seen"
            )));
        }
        let mesh = self.mesh(target, "target")?;
        let [x, y, z] = request.eye();
        let eye = Point3::new(x, y, z);
        let distance = nearest(eye, mesh).ok_or_else(|| {
            SightError::Unavailable(format!("target {target} has no triangles to be seen"))
        })?;
        // A tessellation lies within its chord deviation of the true surface.
        let deviation = self
            .geometry
            .fidelity(target)
            .map_err(|error| SightError::Unavailable(format!("target {target}: {error}")))?
            .deviation_metres();
        let slack = DISTANCE_SLACK * (1.0 + distance) + deviation;
        let bounds = ((distance - slack).max(0.0), distance + slack);
        // The eye in model coordinates (#372).
        let [x, y, z] = self
            .geometry
            .local_origin(&target.source)
            .point_to_model(request.eye());
        let locator = |what: &str| format!("line-of-sight:({x},{y},{z}):{target}:{what}");
        let evidence = |what: &str| Evidence {
            source: target.source.clone(),
            locator: locator(what),
            exact: deviation <= 0.0,
            witness: None,
        };
        if request
            .within_metres()
            .is_some_and(|range| bounds.0 > range)
        {
            return SightEvidence::try_new(target.clone(), bounds, None, evidence("beyond-range"));
        }
        self.exact(target, "target")?;
        let reach = mesh_extent(mesh)
            .map(|(min, max)| {
                (
                    [min[0].min(x), min[1].min(y), min[2].min(z)],
                    [max[0].max(x), max[1].max(y), max[2].max(z)],
                )
            })
            .ok_or_else(|| SightError::Unavailable(format!("target {target} has no positions")))?;
        let mut blockers: Vec<(&ObjectId, &TriMesh)> = Vec::new();
        for blocker in request.blockers() {
            if self.geometry.has_no_body(blocker) {
                continue;
            }
            let blocking = self.mesh(blocker, "blocker")?;
            if mesh_extent(blocking).is_some_and(|extent| !boxes_meet(&extent, &reach)) {
                continue;
            }
            self.exact(blocker, "blocker")?;
            blockers.push((blocker, blocking));
        }
        let meshes: Vec<&TriMesh> = blockers.iter().map(|(_, mesh)| *mesh).collect();
        let sight = line_of_sight(eye, mesh, &meshes).map_err(|error| {
            SightError::Unavailable(format!(
                "the view of {target} cannot be assessed: {error:?}"
            ))
        })?;
        let (outcome, what) = match sight {
            Sight::Visible { through, .. } => (
                SightOutcome::Visible {
                    through: [through.x, through.y, through.z],
                },
                "visible".to_owned(),
            ),
            Sight::Hidden { occluders } => {
                let occluders: Vec<ObjectId> = occluders
                    .iter()
                    .filter_map(|index| blockers.get(*index).map(|(id, _)| (*id).clone()))
                    .collect();
                let named = occluders
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                (
                    SightOutcome::Hidden { occluders },
                    format!("hidden-by:{named}"),
                )
            }
            _ => (SightOutcome::Undecided, "undecided".to_owned()),
        };
        SightEvidence::try_new(target.clone(), bounds, Some(outcome), evidence(&what))
    }

    fn measure_sky_obstruction(
        &self,
        request: &SkyObstructionRequest,
    ) -> Result<SkyObstruction, SightError> {
        let wedge = Wedge::of(request);
        // Each obstacle's own greatest elevation, where it has a point in
        // front within the sector.
        let mut tops: Vec<(&ObjectId, f64)> = Vec::new();
        for obstacle in request.obstacles() {
            if self.geometry.has_no_body(obstacle) {
                continue;
            }
            let mesh = self.mesh(obstacle, "obstacle")?;
            if mesh_extent(mesh).is_some_and(|extent| wedge.behind(&extent)) {
                continue;
            }
            self.exact(obstacle, "obstacle")?;
            let top = triangles(mesh)
                .iter()
                .filter_map(|triangle| wedge.highest(triangle))
                .reduce(f64::max);
            if let Some(top) = top {
                tops.push((obstacle, top));
            }
        }
        let greatest = tops
            .iter()
            .map(|(_, top)| *top)
            .fold(f64::NEG_INFINITY, f64::max);
        let highest: Vec<ObjectId> = tops
            .iter()
            .filter(|(_, top)| *top > -ANGLE_SLACK && *top >= greatest - 2.0 * ANGLE_SLACK)
            .map(|(id, _)| (*id).clone())
            .collect();
        let angle = if greatest < -ANGLE_SLACK {
            (0.0, 0.0)
        } else {
            (
                (greatest - ANGLE_SLACK).max(0.0),
                (greatest + ANGLE_SLACK).min(FRAC_PI_2),
            )
        };
        let [x, y, z] = self
            .geometry
            .local_origin(&request.from().source)
            .point_to_model(request.eye());
        let [east, north] = request.toward();
        let named = highest
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let evidence = Evidence {
            source: request.from().source.clone(),
            locator: format!(
                "sky-obstruction:{}:({x},{y},{z}):toward=({east},{north}):half-width={}:\
                 highest={named}",
                request.from(),
                request.half_width()
            ),
            exact: true,
            witness: None,
        };
        SkyObstruction::try_new(angle, highest, evidence)
    }
}

/// Bodies a straight sight line between two points is tested against,
/// read once for the many sight lines of one measurement (a sight
/// distance along an alignment samples many targets).
///
/// Two questions are answered, each a proof:
///
/// - [`Self::near`] names the obstacles that may come within a reach of
///   the segment from an eye to a target. An obstacle is cleared when
///   every triangle of its mesh lies farther than the reach, its chord
///   deviation and a rounding bound from the segment (measured against a
///   sliver triangle holding it, so never nearer than the true distance),
///   and the eye lies outside its mesh by the kernel's exact winding
///   number. Every segment from the eye to a point within the reach of
///   the target then misses it.
/// - [`Self::blocking`] names the exact obstacles the kernel's
///   `line_of_sight` proves to hide the target point: they hide every ray
///   from the eye to a small triangle across the sight line holding the
///   target, so the ray to the target meets them before it.
///
/// An unmeasured obstacle may stand anywhere, so it refuses the whole
/// question; a declared bodiless one is skipped. A tessellated obstacle
/// may be cleared by its deviation but never proves a block.
#[derive(Debug)]
pub struct SightObstacles<'g> {
    obstacles: Vec<Obstacle<'g>>,
}

#[derive(Debug)]
struct Obstacle<'g> {
    id: ObjectId,
    mesh: &'g TriMesh,
    triangles: Vec<Triangle>,
    extent: Extent,
    deviation: f64,
}

impl<'g> SightObstacles<'g> {
    /// The obstacles among `objects` that have a body in `geometry`.
    ///
    /// # Errors
    ///
    /// [`SightError::Unavailable`] for an unmeasured obstacle or one whose
    /// deviation is unusable, [`SightError::UnknownObject`] for one the
    /// geometry holds nothing about.
    pub fn new(geometry: &'g AxiolidGeometry, objects: &[ObjectId]) -> Result<Self, SightError> {
        let mut obstacles = Vec::new();
        for object in objects {
            if geometry.has_no_body(object) {
                continue;
            }
            if let Some((_, reason)) = geometry.unmeasured_of(object) {
                return Err(SightError::Unavailable(format!(
                    "obstacle {object} has a body that was not measured: {reason}"
                )));
            }
            let mesh = geometry
                .mesh(object)
                .ok_or_else(|| SightError::UnknownObject(object.clone()))?;
            let deviation = geometry
                .fidelity(object)
                .map_err(|error| SightError::Unavailable(format!("obstacle {object}: {error}")))?
                .deviation_metres();
            let Some(extent) = mesh_extent(mesh) else {
                continue;
            };
            obstacles.push(Obstacle {
                id: object.clone(),
                mesh,
                triangles: triangles(mesh),
                extent,
                deviation,
            });
        }
        Ok(Self { obstacles })
    }

    /// The obstacles not proven to stay farther than `reach` from the
    /// segment from `eye` to `target`, in the order given; none when every
    /// segment from the eye to a point within `reach` of the target is
    /// clear of every obstacle.
    #[must_use]
    pub fn near(&self, eye: [f64; 3], target: [f64; 3], reach: f64) -> Vec<&ObjectId> {
        let (from, to) = (point(eye), point(target));
        let sliver = sliver(from, to);
        let scale = eye
            .iter()
            .chain(&target)
            .fold(1.0_f64, |held, value| held.max(value.abs()));
        let segment_box: Extent = (
            std::array::from_fn(|axis| eye[axis].min(target[axis])),
            std::array::from_fn(|axis| eye[axis].max(target[axis])),
        );
        let mut near = Vec::new();
        for obstacle in &self.obstacles {
            let margin =
                (reach + obstacle.deviation) * (1.0 + DISTANCE_SLACK) + DISTANCE_SLACK * scale;
            let grown: Extent = (
                obstacle.extent.0.map(|value| value - margin),
                obstacle.extent.1.map(|value| value + margin),
            );
            if !boxes_meet(&grown, &segment_box) {
                continue;
            }
            let apart = obstacle.triangles.iter().all(|triangle| {
                distance_to(sliver.as_ref(), (from, to), triangle).is_some_and(|gap| gap > margin)
            });
            // Farther than the margin from every face: the reach around the
            // segment lies wholly inside the body or wholly outside it, as
            // the eye does.
            if !(apart && contains(obstacle.mesh, from) == Some(false)) {
                near.push(&obstacle.id);
            }
        }
        near
    }

    /// The exact obstacles proven to hide `target` from `eye`, sorted;
    /// `None` when no such proof was found. The target is taken as known
    /// within `within` metres: the proof covers every point that near it
    /// across the sight line.
    ///
    /// # Errors
    ///
    /// [`SightError::InvalidRequest`] for a point that is not finite or a
    /// target at the eye.
    pub fn blocking(
        &self,
        eye: [f64; 3],
        target: [f64; 3],
        within: f64,
    ) -> Result<Option<Vec<ObjectId>>, SightError> {
        let (from, to) = (point(eye), point(target));
        let along = to - from;
        let length = along.length();
        if !(length.is_finite() && length > 0.0 && within.is_finite()) {
            return Err(SightError::InvalidRequest(
                "the sight line has no finite length".into(),
            ));
        }
        let segment_box: Extent = (
            std::array::from_fn(|axis| eye[axis].min(target[axis]) - within),
            std::array::from_fn(|axis| eye[axis].max(target[axis]) + within),
        );
        let candidates: Vec<&Obstacle<'_>> = self
            .obstacles
            .iter()
            .filter(|obstacle| obstacle.deviation <= 0.0)
            .filter(|obstacle| boxes_meet(&obstacle.extent, &segment_box))
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        // A small triangle across the sight line whose inscribed circle
        // (half its circumradius) holds every point within `within` of the
        // target.
        let radius = 2.0 * within.max(1e-9 * (1.0 + length));
        let (u, v) = across(along * (1.0 / length));
        let corners: Vec<Point3> = [0.0_f64, 1.0, 2.0]
            .iter()
            .map(|corner| {
                let angle = corner * 2.0 * std::f64::consts::FRAC_PI_3;
                to + u * (radius * angle.cos()) + v * (radius * angle.sin())
            })
            .collect();
        let spot = TriMesh::new(corners, vec![0, 1, 2]);
        let meshes: Vec<&TriMesh> = candidates.iter().map(|obstacle| obstacle.mesh).collect();
        match line_of_sight(from, &spot, &meshes) {
            Ok(Sight::Hidden { occluders }) => {
                let mut named: Vec<ObjectId> = occluders
                    .iter()
                    .filter_map(|index| candidates.get(*index).map(|found| found.id.clone()))
                    .collect();
                named.sort();
                named.dedup();
                Ok((!named.is_empty()).then_some(named))
            }
            Ok(_) => Ok(None),
            Err(error) => Err(SightError::InvalidRequest(format!(
                "the sight line cannot be assessed: {error:?}"
            ))),
        }
    }
}

fn point([x, y, z]: [f64; 3]) -> Point3 {
    Point3::new(x, y, z)
}

/// Two unit vectors across the unit direction `along` and across each
/// other.
fn across(along: Vec3) -> (Vec3, Vec3) {
    let helper = if along.x.abs() < 0.9 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let u = along.cross(helper);
    let u = u * (1.0 / u.length());
    (u, along.cross(u))
}

/// A thin triangle holding the segment from `from` to `to`; `None` for a
/// segment of no length.
fn sliver(from: Point3, to: Point3) -> Option<Triangle> {
    let along = to - from;
    let length = along.length();
    if !(length.is_finite() && length > 0.0) {
        return None;
    }
    let (u, _) = across(along * (1.0 / length));
    Some([from, to, to + u * (1e-6 * length)])
}

/// A lower bound on the distance from the segment `[from, to]` to
/// `triangle`: its distance to `sliver`, which holds the segment, or the
/// segment's own distance to a degenerate triangle's edges. `None` where
/// none can be measured.
fn distance_to(
    sliver: Option<&Triangle>,
    (from, to): (Point3, Point3),
    triangle: &Triangle,
) -> Option<f64> {
    let whole = match sliver {
        Some(sliver) => closest_points_on_triangles(*sliver, *triangle)
            .ok()
            .map(|pair| pair.distance_squared.sqrt()),
        None => closest_point_on_triangle(from, *triangle)
            .ok()
            .map(|closest| (closest - from).length()),
    };
    whole.or_else(|| {
        // A degenerate triangle is its edges.
        (0..3)
            .map(|i| {
                closest_points_on_segments([triangle[i], triangle[(i + 1) % 3]], [from, to])
                    .ok()
                    .map(|pair| pair.distance_squared.sqrt())
            })
            .try_fold(f64::INFINITY, |held, gap| gap.map(|gap| held.min(gap)))
    })
}

/// Bound on the rounding of a measured elevation angle, in radians.
const ANGLE_SLACK: f64 = 1e-9;

/// How far in front of the eye, along the direction looked, a point must
/// lie to count: a body flush with the face the eye stands on does not
/// rise straight above it.
const IN_FRONT: f64 = 1e-6;

/// A point relative to the eye: how far ahead along the direction looked,
/// how far to its left, and how far above the eye, in metres.
#[derive(Clone, Copy, Debug)]
struct Local {
    ahead: f64,
    left: f64,
    above: f64,
}

impl Local {
    /// The point `share` of the way from `self` to `other`.
    fn towards(self, other: Self, share: f64) -> Self {
        Self {
            ahead: self.ahead + (other.ahead - self.ahead) * share,
            left: self.left + (other.left - self.left) * share,
            above: self.above + (other.above - self.above) * share,
        }
    }

    /// Its elevation angle above the eye's horizontal plane.
    fn elevation(self) -> f64 {
        self.above.atan2(self.ahead.hypot(self.left))
    }
}

/// The sector in front of an eye, in which obstacles are looked at.
struct Wedge {
    eye: [f64; 3],
    toward: [f64; 2],
    /// The tangent of the half-width; `None` for a half-plane.
    spread: Option<f64>,
}

impl Wedge {
    fn of(request: &SkyObstructionRequest) -> Self {
        let half = request.half_width();
        Self {
            eye: request.eye(),
            toward: request.toward(),
            spread: (half < FRAC_PI_2).then(|| half.tan()),
        }
    }

    fn local(&self, point: &Point3) -> Local {
        let [x, y, z] = [
            point.x - self.eye[0],
            point.y - self.eye[1],
            point.z - self.eye[2],
        ];
        let [east, north] = self.toward;
        Local {
            ahead: x * east + y * north,
            left: y * east - x * north,
            above: z,
        }
    }

    /// Whether a box lies wholly at or behind the line the eye looks out
    /// of, so no point of it is in front.
    fn behind(&self, (min, max): &Extent) -> bool {
        let [east, north] = self.toward;
        let ahead = |x: f64, y: f64| (x - self.eye[0]) * east + (y - self.eye[1]) * north;
        [
            ahead(min[0], min[1]),
            ahead(min[0], max[1]),
            ahead(max[0], min[1]),
            ahead(max[0], max[1]),
        ]
        .into_iter()
        .all(|value| value <= IN_FRONT)
    }

    /// The greatest elevation of the triangle's part within the sector,
    /// `None` where none of it is.
    ///
    /// Along any ray from the eye's vertical line, the elevation over a
    /// plane is monotone in the distance, so the greatest lies on the
    /// boundary of the part: on its edges, at an end or where an edge's
    /// elevation is stationary.
    fn highest(&self, triangle: &Triangle) -> Option<f64> {
        let corners: Vec<Local> = triangle.iter().map(|point| self.local(point)).collect();
        let ahead = clip(&corners, |point| point.ahead - IN_FRONT);
        let part = match self.spread {
            None => ahead,
            Some(spread) if spread > 0.0 => {
                let right = clip(&ahead, |point| point.ahead * spread - point.left);
                clip(&right, |point| point.ahead * spread + point.left)
            }
            Some(_) => on_line(&ahead),
        };
        match part.as_slice() {
            [] => None,
            [point] => Some(point.elevation()),
            points => (0..points.len())
                .map(|at| edge_highest(points[at], points[(at + 1) % points.len()]))
                .reduce(f64::max),
        }
    }
}

/// The part of the convex polygon `points` where `side` is not negative.
fn clip(points: &[Local], side: impl Fn(&Local) -> f64) -> Vec<Local> {
    let mut kept = Vec::new();
    for (at, point) in points.iter().enumerate() {
        let next = points[(at + 1) % points.len()];
        let (here, there) = (side(point), side(&next));
        if here >= 0.0 {
            kept.push(*point);
        }
        if (here >= 0.0) != (there >= 0.0) {
            kept.push(point.towards(next, here / (here - there)));
        }
    }
    kept
}

/// The part of the convex polygon `points` on the vertical plane straight
/// ahead of the eye (`left` zero): the polygon itself where it lies in
/// that plane, otherwise the points where its edges cross it.
#[allow(clippy::float_cmp)]
fn on_line(points: &[Local]) -> Vec<Local> {
    if points.iter().all(|point| point.left == 0.0) {
        return points.to_vec();
    }
    let mut crossing = Vec::new();
    for (at, point) in points.iter().enumerate() {
        let next = points[(at + 1) % points.len()];
        if point.left == 0.0 {
            crossing.push(*point);
        } else if (point.left < 0.0) != (next.left < 0.0) && next.left != 0.0 {
            let mut met = point.towards(next, point.left / (point.left - next.left));
            met.left = 0.0;
            crossing.push(met);
        }
    }
    crossing
}

/// The greatest elevation along the segment from `start` to `end`: at an
/// end, or where `above / distance` is stationary, which for a straight
/// segment is one root of a linear equation.
fn edge_highest(start: Local, end: Local) -> f64 {
    let ends = start.elevation().max(end.elevation());
    let (ahead, left, above) = (
        end.ahead - start.ahead,
        end.left - start.left,
        end.above - start.above,
    );
    // The squared plan distance is `a·s² + b·s + c` along the share `s`.
    let a = ahead * ahead + left * left;
    let b = 2.0 * (start.ahead * ahead + start.left * left);
    let c = start.ahead * start.ahead + start.left * start.left;
    let denominator = above * b / 2.0 - start.above * a;
    if denominator == 0.0 {
        return ends;
    }
    let share = (start.above * b / 2.0 - above * c) / denominator;
    if share > 0.0 && share < 1.0 {
        ends.max(start.towards(end, share).elevation())
    } else {
        ends
    }
}

/// Whether two closed boxes share a point.
fn boxes_meet(a: &Extent, b: &Extent) -> bool {
    (0..3).all(|axis| a.0[axis] <= b.1[axis] && b.0[axis] <= a.1[axis])
}

/// The least distance from `point` to the mesh's triangles; `None` for a
/// mesh without triangles. A degenerate triangle is measured by its edges.
fn nearest(point: Point3, mesh: &TriMesh) -> Option<f64> {
    let mut best: Option<f64> = None;
    for triangle in triangles(mesh) {
        let distance = match closest_point_on_triangle(point, triangle) {
            Ok(closest) => Some((closest - point).length()),
            Err(_) => (0..3)
                .filter_map(|i| {
                    closest_points_on_segments([triangle[i], triangle[(i + 1) % 3]], [point, point])
                        .ok()
                        .map(|pair| pair.distance_squared.sqrt())
                })
                .reduce(f64::min),
        };
        if let Some(distance) = distance {
            best = Some(best.map_or(distance, |held| held.min(distance)));
        }
    }
    best
}
