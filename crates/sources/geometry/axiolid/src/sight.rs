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

use axiolid_core::Point3;
use axiolid_inspect::{Sight, line_of_sight};
use axiolid_measure::{closest_point_on_triangle, closest_points_on_segments};
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
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == object)
        {
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
        let locator = |what: &str| {
            format!(
                "line-of-sight:({x},{y},{z}):{target}:{what}",
                x = request.eye()[0],
                y = request.eye()[1],
                z = request.eye()[2]
            )
        };
        let evidence = |what: &str| Evidence {
            source: target.source.clone(),
            locator: locator(what),
            exact: deviation <= 0.0,
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
        let [x, y, z] = request.eye();
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
        };
        SkyObstruction::try_new(angle, highest, evidence)
    }
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
