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
//!
//! The same triangles grouped into pieces (`measure_pieces`): triangles
//! sharing an edge join one piece when their normals may be the true
//! surface's on both sides, coplanar within a part in a billion on an
//! exact mesh and within both triangles' leans on a tessellated one; a
//! crease the mesh states (a crest beside a batter) always separates two
//! pieces. Each piece's area is the sum of its triangles', boxed by their
//! rounding; a tessellated piece's true patch differs from its triangles
//! at most within a band of width `2d` along their edges, as
//! `facade_area` bounds it, and may lean by the greatest lean, so its area
//! widens by `2·P·d + π·d²` for the triangles' summed perimeter `P` and
//! its upper bound by the secant of that lean. Only a mesh that is the
//! exact shape states exact pieces.

use std::collections::BTreeMap;

use axioval_engine::{
    FaceNormal, FaceNormals, FacePiece, FacePieceSet, FacePieces, SurfaceFace, VerticalExtentError,
};
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
pub(crate) fn signed_volume(soup: &[Triangle]) -> f64 {
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

/// One triangle of a face: its index in the soup, its certified normal
/// box, its outward cross product, the length of that product, the bound
/// on the product's rounding, and how far the true surface's normal may
/// lean from it.
struct Part {
    index: usize,
    normal: FaceNormal,
    centre: [f64; 3],
    size: f64,
    rounding: [f64; 3],
    lean: f64,
}

/// Which triangles a face holds.
#[derive(Clone, Copy)]
enum Which {
    Face(SurfaceFace),
    Boundary,
}

/// The triangles of `which` of the mesh `soup`, each with its normal box.
fn parts(
    object: &ObjectId,
    which: Which,
    soup: &[Triangle],
    tessellation: Option<f64>,
    closed: bool,
) -> Result<Vec<Part>, VerticalExtentError> {
    let name = match which {
        Which::Face(SurfaceFace::Top) => "top face",
        Which::Face(SurfaceFace::Bottom) => "bottom face",
        Which::Boundary => "boundary",
    };
    let unavailable =
        |reason: &str| VerticalExtentError::Unavailable(format!("the {name} of {object} {reason}"));
    let outward = if closed {
        let volume = signed_volume(soup);
        if volume == 0.0 || !volume.is_finite() {
            return Err(unavailable(
                "cannot be told apart: the body encloses no volume",
            ));
        }
        volume.signum()
    } else if matches!(which, Which::Boundary) {
        return Err(unavailable(
            "has no outside: an open surface faces no direction",
        ));
    } else {
        1.0
    };
    let mut parts = Vec::new();
    for (index, triangle) in soup.iter().enumerate() {
        let (centre, rounding) = cross(triangle);
        let size = length(centre);
        if !size.is_finite() || size == 0.0 {
            // A degenerate triangle covers nothing.
            continue;
        }
        let up = outward * centre[2];
        let looks = match which {
            Which::Boundary => true,
            Which::Face(face) if closed => match face {
                SurfaceFace::Top => up > VERTICAL_TOLERANCE * size,
                SurfaceFace::Bottom => up < -VERTICAL_TOLERANCE * size,
            },
            Which::Face(_) => up.abs() > VERTICAL_TOLERANCE * size,
        };
        if !looks {
            continue;
        }
        let (spread, lean) = match tessellation {
            None => (0.0, 0.0),
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
                (
                    (size * lean * (1.0 + 4.0 * f64::EPSILON)).max(size * f64::EPSILON),
                    lean,
                )
            }
        };
        let mut lower = [0.0; 3];
        let mut upper = [0.0; 3];
        let mut oriented = [0.0; 3];
        for axis in 0..3 {
            let value = outward * centre[axis];
            oriented[axis] = value;
            let margin = rounding[axis] + spread;
            if margin == 0.0 {
                (lower[axis], upper[axis]) = (value, value);
            } else {
                lower[axis] = (value - margin).next_down();
                upper[axis] = (value + margin).next_up();
            }
        }
        let normal = FaceNormal::try_new(lower, upper)
            .map_err(|_| unavailable("has a triangle too small to bound its slope"))?;
        parts.push(Part {
            index,
            normal,
            centre: oriented,
            size,
            rounding,
            lean,
        });
    }
    if parts.is_empty() {
        return Err(unavailable("does not exist: no triangle looks that way"));
    }
    Ok(parts)
}

/// The normals of `face` of the mesh `soup`.
pub(crate) fn measure(
    object: &ObjectId,
    face: SurfaceFace,
    soup: &[Triangle],
    tessellation: Option<f64>,
    closed: bool,
) -> Result<FaceNormals, VerticalExtentError> {
    let normals: Vec<FaceNormal> = parts(object, Which::Face(face), soup, tessellation, closed)?
        .into_iter()
        .map(|part| part.normal)
        .collect();
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

/// Two normals of a planar piece may differ by this much, relative to
/// their lengths, from the rounding of their cross products.
const COPLANAR: f64 = 1e-9;

/// A vertex by its exact coordinates, `-0.0` read as `0.0`.
fn vertex(point: &axiolid_core::Point3) -> [u64; 3] {
    point
        .to_array()
        .map(|coordinate| (coordinate + 0.0).to_bits())
}

/// The representative of `part`'s piece, halving the path to it.
fn root(parents: &mut [usize], mut part: usize) -> usize {
    while parents[part] != part {
        parents[part] = parents[parents[part]];
        part = parents[part];
    }
    part
}

/// Whether two triangles sharing an edge may lie on one smooth surface:
/// their normals agree within the rounding, or within both leans on a
/// tessellation.
fn smooth(first: &Part, second: &Part) -> bool {
    let (p, q) = (first.centre, second.centre);
    let normal = [
        p[1] * q[2] - p[2] * q[1],
        p[2] * q[0] - p[0] * q[2],
        p[0] * q[1] - p[1] * q[0],
    ];
    let dot: f64 = (0..3).map(|axis| p[axis] * q[axis]).sum();
    let sine = length(normal) / (first.size * second.size);
    dot > 0.0 && sine <= COPLANAR + first.lean + second.lean
}

/// A triangle's area, `[lower, upper]`, from its cross product's length
/// and the bound on that product's rounding.
fn area(part: &Part) -> (f64, f64) {
    let error = length(part.rounding) * (1.0 + 4.0 * f64::EPSILON) + 4.0 * f64::EPSILON * part.size;
    (
        ((part.size - error) / 2.0).next_down().max(0.0),
        f64::midpoint(part.size, error).next_up(),
    )
}

/// The perimeter of a triangle, rounded up.
fn perimeter(triangle: &Triangle) -> f64 {
    let [a, b, c] = triangle.map(|point| point.to_array());
    let edge = |p: [f64; 3], q: [f64; 3]| length([q[0] - p[0], q[1] - p[1], q[2] - p[2]]);
    ((edge(a, b) + edge(b, c) + edge(c, a)) * (1.0 + 8.0 * f64::EPSILON)).next_up()
}

/// The pieces of `set` of the mesh `soup`, ordered by their first
/// triangle.
pub(crate) fn measure_pieces(
    object: &ObjectId,
    set: FacePieceSet,
    soup: &[Triangle],
    tessellation: Option<f64>,
    closed: bool,
) -> Result<FacePieces, VerticalExtentError> {
    let which = match set {
        FacePieceSet::Top => Which::Face(SurfaceFace::Top),
        FacePieceSet::Bottom => Which::Face(SurfaceFace::Bottom),
        FacePieceSet::Boundary => Which::Boundary,
    };
    let parts = parts(object, which, soup, tessellation, closed)?;
    let mut parents: Vec<usize> = (0..parts.len()).collect();
    let mut edges: BTreeMap<([u64; 3], [u64; 3]), Vec<usize>> = BTreeMap::new();
    for (at, part) in parts.iter().enumerate() {
        let corners = soup[part.index].each_ref().map(vertex);
        for (from, to) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (corners[from], corners[to]);
            edges.entry((p.min(q), p.max(q))).or_default().push(at);
        }
    }
    for sharing in edges.values() {
        for (i, &a) in sharing.iter().enumerate() {
            for &b in &sharing[i + 1..] {
                if smooth(&parts[a], &parts[b]) {
                    let (a, b) = (root(&mut parents, a), root(&mut parents, b));
                    parents[a.max(b)] = a.min(b);
                }
            }
        }
    }
    // Pieces keyed by their first triangle, which is their root's.
    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for at in 0..parts.len() {
        let representative = root(&mut parents, at);
        grouped.entry(representative).or_default().push(at);
    }
    let mut pieces = Vec::new();
    for members in grouped.values() {
        let (mut lower, mut upper) = (0.0_f64, 0.0_f64);
        let (mut edges, mut leaning) = (0.0_f64, 0.0_f64);
        for &at in members {
            let (low, high) = area(&parts[at]);
            lower = (lower + low).next_down();
            upper = (upper + high).next_up();
            edges = (edges + perimeter(&soup[parts[at].index])).next_up();
            leaning = leaning.max(parts[at].lean);
        }
        if let Some(deviation) = tessellation {
            let band = (2.0 * edges * deviation + std::f64::consts::PI * deviation * deviation)
                * (1.0 + 8.0 * f64::EPSILON);
            lower = (lower - band).next_down().max(0.0);
            upper = ((upper + band) / leaning.cos() * (1.0 + 8.0 * f64::EPSILON)).next_up();
        }
        let normals = members.iter().map(|&at| parts[at].normal).collect();
        pieces.push(
            FacePiece::try_new(normals, lower.max(0.0), upper).map_err(|_| {
                VerticalExtentError::Unavailable(format!(
                    "a piece of {object} has no measurable area"
                ))
            })?,
        );
    }
    let evidence = Evidence {
        source: object.source.clone(),
        locator: format!(
            "face-pieces:{object}:{}",
            match set {
                FacePieceSet::Top => "top",
                FacePieceSet::Bottom => "bottom",
                FacePieceSet::Boundary => "boundary",
            }
        ),
        exact: tessellation.is_none(),
    };
    FacePieces::try_new(object.clone(), set, pieces, evidence)
}
