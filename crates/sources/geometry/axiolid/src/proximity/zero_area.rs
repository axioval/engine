//! Zero-area triangles, measured as the segments they are.
//!
//! A mesh may hold triangles whose corners lie on one line (a warped face's
//! triangulation leaves them, so does a T-junction closed with a sliver). The
//! kernel's triangle primitives refuse such a triangle (`DegenerateTriangle`
//! from `closest_point_on_triangle`, `closest_points_on_triangles` and
//! `intersect_triangle`), and the winding number refuses a mesh holding one.
//! A zero-area triangle adds no surface: every point of it lies on its
//! boundary, its three edges, which cover its longest edge. So here
//!
//! - a distance to it is the distance to its edges,
//! - a segment meets it only where it meets one of its edges, and
//! - the winding number is taken over the mesh without it, which changes
//!   nothing a zero-area triangle could add, and is bounded for the area
//!   the kernel's audit calls zero ([`Winding::inside`]).
//!
//! The kernel calls a triangle degenerate when its computed doubled area is
//! exactly zero. Rounding can hide a corner lying off the longest edge by up
//! to a few units in the last place of the coordinates; that height bounds
//! how far the triangle's interior lies from its edges. A triangle whose
//! height exceeds [`LINEAR_TOLERANCE`], the distance below which surfaces
//! meet, is refused by name rather than measured as its edges.

use axiolid_core::{Point3, Ray3, Tolerance};
use axiolid_measure::{
    ProximityError as KernelError, WindingMesh, closest_point_on_triangle,
    closest_points_on_segments, closest_points_on_triangles,
};
use axiolid_mesh::{TriMesh, audit_mesh};
use axiolid_ray_mesh::{RayMeshError, intersect_triangle};
use axioval_engine::ProximityError;

use super::{INSIDE_WINDING, LINEAR_TOLERANCE, tolerance};
use crate::geometry::Triangle;

/// A zero-area triangle whose corners lie off its longest edge by more than
/// the contact tolerance: its edges would not hold all its points.
const OFF_ITS_EDGE: &str = "a triangle the kernel reads as zero-area has a corner off its longest \
     edge by more than the contact tolerance, so it is neither a segment nor a face";

/// A point too near a left-out zero-area triangle for the winding number
/// to place it.
const TOO_NEAR: &str = "a point lies too near a zero-area triangle left out of the winding \
     number to tell inside from outside";

/// Whether the kernel's triangle primitives refuse `triangle` as zero-area.
pub(crate) fn zero_area(triangle: &Triangle) -> bool {
    let [a, b, c] = *triangle;
    (b - a).cross(c - a).length_squared() == 0.0
}

/// The three edges of a zero-area triangle, which hold every one of its
/// points; refused when a corner lies off the longest edge by more than
/// [`LINEAR_TOLERANCE`].
fn edges(triangle: Triangle) -> Result<[[Point3; 2]; 3], ProximityError> {
    let [a, b, c] = triangle;
    let all = [[a, b], [b, c], [c, a]];
    let opposite = [c, a, b];
    let longest = (0..3)
        .max_by(|&i, &j| {
            let length = |[p, q]: [Point3; 2]| (q - p).length_squared();
            length(all[i]).total_cmp(&length(all[j]))
        })
        .unwrap_or(0);
    if point_segment_distance(opposite[longest], all[longest])? > LINEAR_TOLERANCE {
        return Err(ProximityError::Refused(OFF_ITS_EDGE));
    }
    Ok(all)
}

fn point_segment_distance(point: Point3, segment: [Point3; 2]) -> Result<f64, ProximityError> {
    segment_distance([point, point], segment)
}

fn segment_distance(first: [Point3; 2], second: [Point3; 2]) -> Result<f64, ProximityError> {
    closest_points_on_segments(first, second)
        .map(|pair| pair.distance_squared.sqrt())
        .map_err(kernel_refusal)
}

/// Why a kernel triangle primitive refused, in a report's words.
fn kernel_refusal(error: KernelError) -> ProximityError {
    match error {
        KernelError::NonFiniteInput => ProximityError::InvalidMeasurement,
        KernelError::DegenerateTriangle => ProximityError::Refused(
            "the distance kernel refused a triangle with zero area (DegenerateTriangle)",
        ),
    }
}

/// Distance from `point` to `triangle`; a zero-area triangle is measured as
/// its edges.
pub(crate) fn point_triangle_distance(
    point: Point3,
    triangle: Triangle,
) -> Result<f64, ProximityError> {
    match closest_point_on_triangle(point, triangle) {
        Ok(closest) => Ok(closest.distance(point)),
        Err(KernelError::DegenerateTriangle) => {
            let mut best = f64::INFINITY;
            for edge in edges(triangle)? {
                best = best.min(point_segment_distance(point, edge)?);
            }
            Ok(best)
        }
        Err(error) => Err(kernel_refusal(error)),
    }
}

/// Distance between two triangles; a zero-area one is measured as its
/// edges.
pub(crate) fn triangle_pair_distance(
    first: Triangle,
    second: Triangle,
) -> Result<f64, ProximityError> {
    match closest_points_on_triangles(first, second) {
        Ok(pair) => Ok(pair.distance_squared.sqrt()),
        Err(KernelError::DegenerateTriangle) => match (zero_area(&first), zero_area(&second)) {
            (true, true) => {
                let mut best = f64::INFINITY;
                for a in edges(first)? {
                    for b in edges(second)? {
                        best = best.min(segment_distance(a, b)?);
                    }
                }
                Ok(best)
            }
            (true, false) => segments_to_triangle(edges(first)?, second),
            (false, true) => segments_to_triangle(edges(second)?, first),
            (false, false) => Err(kernel_refusal(KernelError::DegenerateTriangle)),
        },
        Err(error) => Err(kernel_refusal(error)),
    }
}

/// Least distance from `segments` to a triangle with area.
fn segments_to_triangle(
    segments: [[Point3; 2]; 3],
    triangle: Triangle,
) -> Result<f64, ProximityError> {
    let tolerance = tolerance()?;
    let [a, b, c] = triangle;
    let mut best = f64::INFINITY;
    for segment in segments {
        // A segment through the face meets it although neither end nor any
        // edge comes near it.
        let ray = Ray3 {
            origin: segment[0],
            direction: segment[1] - segment[0],
        };
        if ray.direction.length_squared() != 0.0
            && intersect_triangle(&ray, triangle, tolerance, 0)
                .map_err(|error| ProximityError::Refused(ray_refusal(&error)))?
                .is_some_and(|hit| hit.t <= 1.0)
        {
            return Ok(0.0);
        }
        for point in segment {
            let closest = closest_point_on_triangle(point, triangle).map_err(kernel_refusal)?;
            best = best.min(closest.distance(point));
        }
        for edge in [[a, b], [b, c], [c, a]] {
            best = best.min(segment_distance(segment, edge)?);
        }
    }
    Ok(best)
}

/// Where the segment from `ray.origin` to `ray.origin + ray.direction`
/// meets `triangle`, as a parameter along it; `None` for no meeting.
///
/// A zero-area triangle is met only where the segment comes within
/// [`LINEAR_TOLERANCE`] of one of its edges, at the first such point. Only
/// parameters in `0..=1` are answered: every caller tests an edge of a
/// mesh, never a ray beyond it.
pub(crate) fn segment_hit(
    ray: &Ray3,
    triangle: Triangle,
    tolerance: Tolerance,
    index: usize,
) -> Result<Option<f64>, ProximityError> {
    match intersect_triangle(ray, triangle, tolerance, index) {
        Ok(hit) => Ok(hit.map(|hit| hit.t)),
        Err(RayMeshError::DegenerateTriangle { .. }) => {
            let length_squared = ray.direction.length_squared();
            let segment = [ray.origin, ray.origin + ray.direction];
            let mut first: Option<f64> = None;
            for edge in edges(triangle)? {
                let pair = closest_points_on_segments(segment, edge).map_err(kernel_refusal)?;
                if pair.distance_squared.sqrt() <= LINEAR_TOLERANCE {
                    let t = ((pair.point_a - ray.origin).dot(ray.direction) / length_squared)
                        .clamp(0.0, 1.0);
                    first = Some(first.map_or(t, |known| known.min(t)));
                }
            }
            Ok(first)
        }
        Err(error) => Err(ProximityError::Refused(ray_refusal(&error))),
    }
}

/// Why the ray kernel refused, in a report's words.
fn ray_refusal(error: &RayMeshError) -> &'static str {
    match error {
        RayMeshError::NonFiniteInput => "the ray kernel refused a non-finite coordinate",
        RayMeshError::ZeroDirection => "the ray kernel refused an edge of zero length",
        RayMeshError::InvalidTolerance => "the ray kernel refused the tolerance",
        RayMeshError::DegenerateTriangle { .. } => {
            "the ray kernel refused a triangle with zero area (DegenerateTriangle)"
        }
        _ => "the ray kernel refused the crossing test",
    }
}

/// The squared doubled area under which the kernel's mesh audit, and so the
/// winding number, calls a triangle degenerate (`audit_mesh`).
fn audit_limit() -> f64 {
    LINEAR_TOLERANCE.powi(4)
}

/// A mesh holding zero-area triangles, prepared for the winding number.
pub(crate) struct Surface {
    /// The mesh without them.
    mesh: TriMesh,
    /// An upper bound on the area left out.
    omitted_area: f64,
    /// Whether the whole mesh, zero-area triangles included, is closed and
    /// consistently wound, so its winding number is a whole number off the
    /// surface.
    pub(crate) closed: bool,
}

impl Surface {
    /// `None` when the audit finds no zero-area triangle in `mesh`.
    pub(crate) fn of(mesh: &TriMesh, degenerate: usize) -> Option<Self> {
        if degenerate == 0 {
            return None;
        }
        let limit = audit_limit();
        let mut indices = Vec::with_capacity(mesh.indices.len());
        let mut omitted_area = 0.0;
        for corners in mesh.indices.chunks_exact(3) {
            let [a, b, c] =
                [corners[0], corners[1], corners[2]].map(|index| mesh.positions[index as usize]);
            let squared = (b - a).cross(c - a).length_squared();
            if squared <= limit {
                omitted_area += squared.sqrt() / 2.0;
            } else {
                indices.extend_from_slice(corners);
            }
        }
        let rest = TriMesh::new(mesh.positions.clone(), indices);
        let usable = tolerance()
            .map(|tolerance| audit_mesh(&rest, tolerance).is_surface_usable())
            .unwrap_or(false);
        Some(Self {
            closed: usable && closed_chain(&mesh.indices),
            mesh: rest,
            // The sum is rounded; a relative margin keeps it an upper bound.
            omitted_area: omitted_area * (1.0 + 1e-9),
        })
    }

    pub(crate) fn mesh(&self) -> &TriMesh {
        &self.mesh
    }

    pub(crate) fn omitted_area(&self) -> f64 {
        self.omitted_area
    }
}

/// Whether the triangles form a closed, consistently wound chain: every
/// edge used by exactly two triangles, once each way. Zero-area triangles
/// count, so a T-junction closed by a sliver is closed; a triangle repeating
/// a corner index bounds nothing and is left out.
fn closed_chain(indices: &[u32]) -> bool {
    let mut edges: Vec<(u32, u32, i8)> = Vec::with_capacity(indices.len());
    for corners in indices.chunks_exact(3) {
        let [a, b, c] = [corners[0], corners[1], corners[2]];
        if a == b || b == c || c == a {
            continue;
        }
        for (from, to) in [(a, b), (b, c), (c, a)] {
            edges.push((from.min(to), from.max(to), if from < to { 1 } else { -1 }));
        }
    }
    if edges.is_empty() {
        return false;
    }
    edges.sort_unstable();
    edges.chunks(2).all(|pair| {
        pair.len() == 2
            && (pair[0].0, pair[0].1) == (pair[1].0, pair[1].1)
            && pair[0].2 != pair[1].2
    }) && edges
        .windows(3)
        .all(|run| (run[0].0, run[0].1) != (run[2].0, run[2].1))
}

/// The winding number of a body's mesh, taken without its zero-area
/// triangles.
pub(crate) struct Winding<'a> {
    mesh: WindingMesh<'a, TriMesh>,
    omitted_area: f64,
}

impl<'a> Winding<'a> {
    pub(crate) fn prepare(mesh: &'a TriMesh, omitted_area: f64) -> Result<Self, ProximityError> {
        Ok(Self {
            mesh: WindingMesh::prepare(mesh, tolerance()?).map_err(|_| {
                ProximityError::Refused("the winding number refused the body's mesh")
            })?,
            omitted_area,
        })
    }

    /// Whether `point` lies inside, or `None` when the triangles left out
    /// could decide it. `depth` is the point's distance to the whole mesh,
    /// asked only when something was left out.
    ///
    /// A triangle of area `A` at least `d` away subtends a solid angle of at
    /// most `A / d²`, so leaving out area `A` moves the winding number by at
    /// most `A / (4π d²)`. Off a closed surface the true number is whole;
    /// a quarter keeps the decision well clear of the half it is made at.
    pub(crate) fn inside(
        &self,
        point: Point3,
        depth: impl FnOnce() -> Result<f64, ProximityError>,
    ) -> Result<Option<bool>, ProximityError> {
        if self.omitted_area > 0.0 {
            let depth = depth()?;
            let shift = self.omitted_area / (4.0 * std::f64::consts::PI * depth * depth);
            if shift.is_nan() || shift >= 0.25 {
                return Ok(None);
            }
        }
        let number = self
            .mesh
            .winding_number(point)
            .map_err(|_| ProximityError::Refused("the winding number refused a point"))?;
        Ok(Some(number.value.abs() >= INSIDE_WINDING))
    }

    /// [`Self::inside`], refused by name when undecided.
    pub(crate) fn decide(
        &self,
        point: Point3,
        depth: impl FnOnce() -> Result<f64, ProximityError>,
    ) -> Result<bool, ProximityError> {
        self.inside(point, depth)?
            .ok_or(ProximityError::Refused(TOO_NEAR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    #[test]
    fn a_zero_area_triangle_is_as_far_as_its_longest_edge() {
        let sliver = [p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(1.0, 0.0, 0.0)];
        assert!(zero_area(&sliver));
        let distance = point_triangle_distance(p(1.0, 3.0, 4.0), sliver).unwrap();
        assert!((distance - 5.0).abs() < 1e-12);
        let face = [p(0.0, -1.0, 2.0), p(2.0, -1.0, 2.0), p(1.0, 1.0, 2.0)];
        assert!((triangle_pair_distance(sliver, face).unwrap() - 2.0).abs() < 1e-12);
        assert!((triangle_pair_distance(face, sliver).unwrap() - 2.0).abs() < 1e-12);
        let other = [p(5.0, 0.0, 0.0), p(5.0, 0.0, 0.0), p(5.0, 1.0, 0.0)];
        assert!((triangle_pair_distance(sliver, other).unwrap() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn a_segment_through_a_face_meets_it() {
        let sliver = [p(0.5, 0.5, -1.0), p(0.5, 0.5, 1.0), p(0.5, 0.5, 0.0)];
        let face = [p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)];
        assert!(triangle_pair_distance(sliver, face).unwrap().abs() < f64::EPSILON);
    }

    #[test]
    fn a_segment_meets_a_zero_area_triangle_only_on_its_edges() {
        let sliver = [p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(1.0, 0.0, 0.0)];
        let tolerance = tolerance().unwrap();
        let across = Ray3 {
            origin: p(1.5, -1.0, 0.0),
            direction: Point3::new(0.0, 4.0, 0.0),
        };
        let t = segment_hit(&across, sliver, tolerance, 0).unwrap().unwrap();
        assert!((t - 0.25).abs() < 1e-12);
        let past = Ray3 {
            origin: p(3.0, -1.0, 0.0),
            direction: Point3::new(0.0, 4.0, 0.0),
        };
        assert_eq!(segment_hit(&past, sliver, tolerance, 0).unwrap(), None);
    }

    #[test]
    fn a_corner_off_the_longest_edge_is_refused_by_name() {
        // Rounding can hide a corner's height from the area test; the edges
        // stand in for such a triangle only within the contact tolerance.
        let off = [p(0.0, 0.0, 0.0), p(2.0, 0.0, 0.0), p(1.0, 1e-6, 0.0)];
        assert_eq!(
            edges(off).unwrap_err(),
            ProximityError::Refused(OFF_ITS_EDGE)
        );
    }

    #[test]
    fn a_point_the_left_out_area_could_flip_is_undecided() {
        let tetrahedron = TriMesh::new(
            vec![
                p(0.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(0.0, 1.0, 0.0),
                p(0.0, 0.0, 1.0),
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 2, 0, 3],
        );
        let centre = p(0.25, 0.25, 0.25);
        let whole = Winding::prepare(&tetrahedron, 0.0).unwrap();
        assert_eq!(whole.inside(centre, || unreachable!()), Ok(Some(true)));
        // Area left out within reach of the point could move its winding
        // number by a quarter or more: no answer, and a named refusal
        // where one is needed.
        let left_out = Winding::prepare(&tetrahedron, 1e-18).unwrap();
        assert_eq!(left_out.inside(centre, || Ok(1e-10)), Ok(None));
        assert_eq!(
            left_out.decide(centre, || Ok(1e-10)),
            Err(ProximityError::Refused(TOO_NEAR))
        );
        assert_eq!(left_out.decide(centre, || Ok(0.25)), Ok(true));
    }

    #[test]
    fn a_t_junction_closed_by_a_sliver_is_a_closed_chain() {
        // Two triangles over a quad's diagonal, the lower one split at the
        // diagonal's midpoint (4) and the gap closed by a sliver.
        let open = [0, 1, 2, 0, 4, 3, 4, 2, 3];
        assert!(!closed_chain(&open));
        // A tetrahedron is closed; the same with a face split by a sliver too.
        let tetrahedron = [0, 2, 1, 0, 1, 3, 1, 2, 3, 2, 0, 3];
        assert!(closed_chain(&tetrahedron));
        let split = [0, 2, 1, 0, 1, 3, 1, 4, 3, 4, 2, 3, 2, 0, 3, 1, 2, 4];
        assert!(closed_chain(&split));
        // A repeated corner bounds nothing.
        let with_repeat = [0, 2, 1, 0, 1, 3, 1, 2, 3, 2, 0, 3, 0, 0, 1];
        assert!(closed_chain(&with_repeat));
        // An edge used three times is no closed chain.
        let fin = [0, 2, 1, 0, 1, 3, 1, 2, 3, 2, 0, 3, 0, 1, 5, 1, 0, 6];
        assert!(!closed_chain(&fin));
    }
}
