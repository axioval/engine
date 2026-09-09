//! Guard-edge measurement over application-supplied geometry.
//!
//! ADR 0004: this module measures. It reports what sits near the exposed edge
//! of a walking surface -- barriers above it, landings below it, climbable
//! objects beside a barrier -- and never decides whether the edge is safe.
//!
//! Which objects are walking surfaces is a semantic fact a mesh does not
//! carry, so the host declares them.

use std::collections::BTreeSet;

use axiolid_core::Point2;
use axioval_engine::{
    ClimbableCandidate, GuardCandidate, GuardEdge, GuardError, GuardEvidence, GuardSearch,
    GuardService,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{boundary_rings, ring_perimeter, ring_segments};

/// Measures guard edges from application-supplied meshes.
pub struct AxiolidGuardService {
    geometry: AxiolidGeometry,
    source: SourceId,
    surfaces: BTreeSet<ObjectId>,
}

impl AxiolidGuardService {
    /// Creates a service over the supplied geometry.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            geometry,
            source,
            surfaces: BTreeSet::new(),
        }
    }

    /// Declares an object to be a walking surface whose edges are measured.
    #[must_use]
    pub fn with_walking_surface(mut self, surface: ObjectId) -> Self {
        self.surfaces.insert(surface);
        self
    }
}

impl AxiolidGuardService {
    /// Objects near a barrier and lower than it, which could be climbed
    /// to defeat it.
    ///
    /// Separate from the edge pass because a climbing aid is measured
    /// against the barrier it would defeat, not against the surface edge.
    fn climbing_aids(
        &self,
        barriers: &[GuardCandidate],
        walking_level: f64,
        radius: f64,
    ) -> Result<Vec<ClimbableCandidate>, GuardError> {
        let mut aids = Vec::new();
        // A climbable is measured against a barrier, not against the edge:
        // an object is only a climbing aid if it stands next to something
        // it would help defeat.
        for barrier in barriers {
            let Some(barrier_mesh) = self.geometry.mesh(barrier.element()) else {
                continue;
            };
            let barrier_points = plan_points(&triangles(barrier_mesh));
            for (candidate_id, candidate_mesh) in self.geometry.objects() {
                if candidate_id == barrier.element() || self.surfaces.contains(candidate_id) {
                    continue;
                }
                let candidate_triangles = triangles(candidate_mesh);
                let Some((_, candidate_top)) = vertical_span(&candidate_triangles) else {
                    continue;
                };
                let points = plan_points(&candidate_triangles);
                let distance = footprint_gap(&points, &barrier_points);
                if !distance.is_finite() || distance > radius {
                    continue;
                }
                let (min_x, max_x, min_y, max_y) = points.iter().fold(
                    (f64::MAX, f64::MIN, f64::MAX, f64::MIN),
                    |(lx, hx, ly, hy), p| (lx.min(p.x), hx.max(p.x), ly.min(p.y), hy.max(p.y)),
                );
                // Only a LOWER object helps defeat a barrier: something
                // taller is a barrier in its own right, not a step up to
                // this one. Without this an element would be reported as
                // its own climbing aid's aid, in both directions.
                if candidate_top - walking_level >= barrier.top_offset_metres() {
                    continue;
                }
                aids.push(ClimbableCandidate::try_new(
                    candidate_id.clone(),
                    barrier.element().clone(),
                    distance,
                    candidate_top - walking_level,
                    (max_x - min_x).min(max_y - min_y),
                )?);
            }
        }

        Ok(aids)
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

/// Distance from `point` to the segment `a`-`b`, and where along it the
/// closest approach falls, normalised to `[0, 1]`.
fn point_to_segment(point: Point2, a: Point2, b: Point2) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f64::EPSILON {
        let (px, py) = (point.x - a.x, point.y - a.y);
        return ((px * px + py * py).sqrt(), 0.0);
    }
    let t = (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared).clamp(0.0, 1.0);
    let (cx, cy) = (a.x + t * dx, a.y + t * dy);
    let (ex, ey) = (point.x - cx, point.y - cy);
    ((ex * ex + ey * ey).sqrt(), t)
}

/// Footprint of a candidate as plan points, for edge proximity tests.
fn plan_points(triangles: &[Triangle]) -> Vec<Point2> {
    triangles
        .iter()
        .flatten()
        .map(|p| Point2::new(p.x, p.y))
        .collect()
}

/// Plan bounding extent of a point set as `(min_x, max_x, min_y, max_y)`.
fn plan_extent(points: &[Point2]) -> Option<(f64, f64, f64, f64)> {
    let first = points.first()?;
    Some(points.iter().fold(
        (first.x, first.x, first.y, first.y),
        |(lx, hx, ly, hy), p| (lx.min(p.x), hx.max(p.x), ly.min(p.y), hy.max(p.y)),
    ))
}

/// Separation between two footprints in plan, zero when they overlap.
///
/// Vertex-to-vertex distance is not the question: two boxes whose faces are
/// 50 mm apart have corner vertices a metre apart, so a real climbing aid
/// would read as remote. The gap between footprints is what a person steps
/// across.
fn footprint_gap(candidate: &[Point2], barrier: &[Point2]) -> f64 {
    let (Some(a), Some(b)) = (plan_extent(candidate), plan_extent(barrier)) else {
        return f64::INFINITY;
    };
    let dx = (b.0 - a.1).max(a.0 - b.1).max(0.0);
    let dy = (b.2 - a.3).max(a.2 - b.3).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// How a candidate relates to one boundary ring of a surface.
///
/// Returns the nearest horizontal gap and the covered edge interval, or
/// `None` when the candidate never comes within `radius` of the edge.
fn edge_relation(
    ring_segments: &[(Point2, Point2)],
    cumulative: &[f64],
    perimeter: f64,
    candidate: &[Point2],
    radius: f64,
) -> Option<(f64, [f64; 2])> {
    let mut nearest = f64::INFINITY;
    let mut span: Option<(f64, f64)> = None;

    for point in candidate {
        for (index, (a, b)) in ring_segments.iter().enumerate() {
            let (distance, t) = point_to_segment(*point, *a, *b);
            if distance > radius {
                continue;
            }
            nearest = nearest.min(distance);
            // Position along the whole perimeter, normalised: a candidate is
            // reported against the edge as a whole, not per segment, so two
            // segments it spans merge into one interval.
            let segment_length = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
            let position = if perimeter > 0.0 {
                (cumulative[index] + t * segment_length) / perimeter
            } else {
                0.0
            };
            span = Some(match span {
                None => (position, position),
                Some((lo, hi)) => (lo.min(position), hi.max(position)),
            });
        }
    }

    let (lo, hi) = span?;
    Some((nearest, [lo.clamp(0.0, 1.0), hi.clamp(0.0, 1.0)]))
}

impl GuardService for AxiolidGuardService {
    fn measure_guard_edges(&self, search: GuardSearch) -> Result<GuardEvidence, GuardError> {
        let tolerance =
            axiolid_core::Tolerance::new(1.0e-9, 1.0e-9).map_err(|_| GuardError::Unavailable)?;
        let radius = search.candidate_radius_metres();

        let mut edges = Vec::new();
        let mut evaluated = 0usize;

        for surface in &self.surfaces {
            let Some(mesh) = self.geometry.mesh(surface) else {
                continue;
            };
            let surface_triangles = triangles(mesh);
            let Some((_, walking_level)) = vertical_span(&surface_triangles) else {
                continue;
            };
            let Some(rings) = boundary_rings(&surface_triangles, tolerance) else {
                continue;
            };
            evaluated += 1;

            // The outer ring is the exposed edge; interior rings are holes,
            // which a guard policy treats through the same candidates.
            let mut barriers = Vec::new();
            let mut landings = Vec::new();

            for ring in &rings {
                let segments = ring_segments(ring);
                let perimeter = ring_perimeter(ring);
                let mut cumulative = Vec::with_capacity(segments.len());
                let mut running = 0.0;
                for (a, b) in &segments {
                    cumulative.push(running);
                    running += ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
                }

                for (candidate_id, candidate_mesh) in self.geometry.objects() {
                    if candidate_id == surface || self.surfaces.contains(candidate_id) {
                        continue;
                    }
                    let candidate_triangles = triangles(candidate_mesh);
                    let Some((_, candidate_top)) = vertical_span(&candidate_triangles) else {
                        continue;
                    };
                    let points = plan_points(&candidate_triangles);
                    let Some((gap, interval)) =
                        edge_relation(&segments, &cumulative, perimeter, &points, radius)
                    else {
                        continue;
                    };

                    // Sign is the whole distinction: a top above the walking
                    // level is a barrier, at or below it is a landing.
                    let top_offset = candidate_top - walking_level;
                    let entry = GuardCandidate::try_new(
                        candidate_id.clone(),
                        gap,
                        top_offset,
                        interval,
                        // Landing width is the reach across the candidate in
                        // plan; for a barrier it carries no meaning and is 0.
                        if top_offset > 0.0 { 0.0 } else { gap.max(0.0) },
                        None,
                    )?;
                    if top_offset > 0.0 {
                        barriers.push(entry);
                    } else {
                        landings.push(entry);
                    }
                }
            }

            let climbables = self.climbing_aids(&barriers, walking_level, radius)?;

            edges.push(GuardEdge::new(
                surface.clone(),
                barriers,
                landings,
                climbables,
            ));
        }

        GuardEvidence::try_new(
            edges,
            evaluated,
            Evidence::exact(self.source.clone(), "axiolid:guard"),
        )
    }
}
