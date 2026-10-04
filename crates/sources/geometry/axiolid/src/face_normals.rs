//! Certified normals of the triangles that make up one face of a mesh.
//!
//! ADR 0004: this module measures normals; what slope a rule accepts is the
//! rule's decision.
//!
//! On a closed mesh the face is chosen by the outward normal: `top` holds
//! the triangles looking up, `bottom` those looking down, and the vertical
//! sides belong to neither. The outside is told by the sign of the enclosed
//! volume, so a mesh wound inside out still reads its faces correctly. An
//! open mesh has no outside, so both faces are every triangle not standing
//! vertical.
//!
//! Each normal is the cross product of two edges, boxed by a bound on its
//! rounding; a component computed without any rounding stays a point, so a
//! planar body with exactly stated vertices measures exactly. A tessellated
//! mesh lies within its chord deviation `d` of the true surface, whose
//! normal over a triangle can lean from the triangle's by up to about
//! `2d / h` for the triangle's least height `h`; the box widens by that
//! much, and a triangle too small for its deviation leaves the face
//! unmeasured.

use axioval_engine::{FaceNormal, FaceNormals, SurfaceFace, VerticalExtentError};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::Triangle;

/// A triangle leaning less than this from the vertical, relative to its
/// normal's length, is a side: neither top nor bottom.
const VERTICAL_TOLERANCE: f64 = 1e-9;

/// `a - b`, and whether it rounded.
fn difference(a: f64, b: f64) -> (f64, bool) {
    let sum = a - b;
    // Two-sum: the exact error of the subtraction.
    let back = sum - a;
    let error = (a - (sum - back)) + (-b - back);
    (sum, error != 0.0)
}

/// `a·d - b·c` and a bound on its rounding: zero only when every product
/// and the difference are exact.
fn determinant(a: f64, d: f64, b: f64, c: f64, inexact_edges: bool) -> (f64, f64) {
    let (left, right) = (a * d, b * c);
    let products_exact = a.mul_add(d, -left) == 0.0 && b.mul_add(c, -right) == 0.0;
    let (value, rounded) = difference(left, right);
    if products_exact && !rounded && !inexact_edges {
        return (value, 0.0);
    }
    // Rounded edges (relative ε each), two products and a difference: well
    // within 8ε of the magnitudes, plus the least subnormal step.
    let bound = 8.0 * f64::EPSILON * (left.abs() + right.abs()) + f64::from_bits(1);
    (value, bound)
}

/// The cross product of a triangle's edges, each component with a bound on
/// its rounding.
fn cross(triangle: &Triangle) -> ([f64; 3], [f64; 3]) {
    let [a, b, c] = triangle.map(|point| point.to_array());
    let mut inexact = false;
    let mut edge = |to: [f64; 3]| {
        let mut out = [0.0; 3];
        for axis in 0..3 {
            let (value, rounded) = difference(to[axis], a[axis]);
            inexact |= rounded;
            out[axis] = value;
        }
        out
    };
    let (first, second) = (edge(b), edge(c));
    let component =
        |i: usize, j: usize| determinant(first[i], second[j], first[j], second[i], inexact);
    let [east, north, up] = [component(1, 2), component(2, 0), component(0, 1)];
    ([east.0, north.0, up.0], [east.1, north.1, up.1])
}

fn length(vector: [f64; 3]) -> f64 {
    vector
        .iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt()
}

/// Six times the signed volume the triangles enclose: positive when they
/// are wound outward.
fn signed_volume(soup: &[Triangle]) -> f64 {
    soup.iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|point| point.to_array());
            let bc = [
                b[1] * c[2] - b[2] * c[1],
                b[2] * c[0] - b[0] * c[2],
                b[0] * c[1] - b[1] * c[0],
            ];
            a[0] * bc[0] + a[1] * bc[1] + a[2] * bc[2]
        })
        .sum()
}

/// How far the true surface's normal may lean from a triangle's, in
/// radians, for chord deviation `deviation`.
fn lean(triangle: &Triangle, normal: f64, deviation: f64) -> f64 {
    let [a, b, c] = triangle.map(|point| point.to_array());
    let edge = |p: [f64; 3], q: [f64; 3]| length([q[0] - p[0], q[1] - p[1], q[2] - p[2]]);
    let longest = edge(a, b).max(edge(b, c)).max(edge(c, a));
    // The least height is twice the area over the longest edge; the normal's
    // length is twice the area.
    let height = normal / longest;
    // Rounded generously: the height and its quotient are not exact.
    2.0 * deviation / height * (1.0 + 16.0 * f64::EPSILON)
}

/// The normals of `face` of the mesh `soup`.
pub(crate) fn measure(
    object: &ObjectId,
    face: SurfaceFace,
    soup: &[Triangle],
    tessellation: Option<f64>,
    closed: bool,
) -> Result<FaceNormals, VerticalExtentError> {
    let unavailable = |reason: &str| {
        VerticalExtentError::Unavailable(format!("the {face:?} face of {object} {reason}"))
    };
    let outward = if closed {
        let volume = signed_volume(soup);
        if volume == 0.0 || !volume.is_finite() {
            return Err(unavailable(
                "cannot be told apart: the body encloses no volume",
            ));
        }
        volume.signum()
    } else {
        1.0
    };
    let mut normals = Vec::new();
    for triangle in soup {
        let (centre, rounding) = cross(triangle);
        let size = length(centre);
        if !size.is_finite() || size == 0.0 {
            // A degenerate triangle covers nothing.
            continue;
        }
        let up = outward * centre[2];
        let looks = if closed {
            match face {
                SurfaceFace::Top => up > VERTICAL_TOLERANCE * size,
                SurfaceFace::Bottom => up < -VERTICAL_TOLERANCE * size,
            }
        } else {
            up.abs() > VERTICAL_TOLERANCE * size
        };
        if !looks {
            continue;
        }
        let spread = match tessellation {
            None => 0.0,
            Some(deviation) => {
                let lean = lean(triangle, size, deviation);
                if lean.is_nan() || lean >= 1.0 {
                    return Err(unavailable(
                        "is tessellated too coarsely for its triangles to bound its slope",
                    ));
                }
                // A lean of `r` moves a normal of length `|n|` by at most
                // `|n|·r` in every component. A tessellation is never exact,
                // even with no declared deviation.
                (size * lean * (1.0 + 4.0 * f64::EPSILON)).max(size * f64::EPSILON)
            }
        };
        let mut lower = [0.0; 3];
        let mut upper = [0.0; 3];
        for axis in 0..3 {
            let value = outward * centre[axis];
            let margin = rounding[axis] + spread;
            if margin == 0.0 {
                (lower[axis], upper[axis]) = (value, value);
            } else {
                lower[axis] = (value - margin).next_down();
                upper[axis] = (value + margin).next_up();
            }
        }
        normals.push(
            FaceNormal::try_new(lower, upper)
                .map_err(|_| unavailable("has a triangle too small to bound its slope"))?,
        );
    }
    if normals.is_empty() {
        return Err(unavailable("does not exist: no triangle looks that way"));
    }
    let exact = normals.iter().all(FaceNormal::is_exact);
    let evidence = Evidence {
        source: object.source.clone(),
        locator: format!(
            "face-normals:{object}:{}",
            match face {
                SurfaceFace::Top => "top",
                SurfaceFace::Bottom => "bottom",
            }
        ),
        exact,
    };
    FaceNormals::try_new(object.clone(), face, normals, evidence)
}
