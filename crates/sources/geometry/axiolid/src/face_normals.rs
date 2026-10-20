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
//!
//! Where the pieces meet (`measure_edges`): every triangle edge between
//! two pieces, and an open surface's border, merged into straight lines
//! between the same pieces where consecutive segments run straight on
//! without rounding. Each side keeps its triangle's normal box (merged
//! only where the segments' agree, else their scaled hull) and which side
//! of the line the face lies, read from the winding, never from a
//! rounded sign; a tessellation's line ends widen by its chord deviation.

use std::collections::BTreeMap;

use axioval_engine::{
    EdgeSide, FaceEdge, FaceEdges, FaceNormal, FaceNormals, FacePiece, FacePieceSet, FacePieces,
    PointBox, SurfaceFace, VerticalExtentError,
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
///
/// The tetrahedra are taken about the first corner, not the world origin.
/// About the origin every term `a · (b × c)` scales with the cube of the
/// distance to it: at site coordinates (10⁵–10⁶ m) the terms reach 10¹⁸ m³
/// and their rounding exceeds a small body's volume, so the sign that says
/// which way the faces look could flip (#452). Vectors from a corner of the
/// body are exact up to their own rounding, so the sum is accurate to the
/// body's size, not to where it lies.
pub(crate) fn signed_volume(soup: &[Triangle]) -> f64 {
    let Some(origin) = soup.first().map(|triangle| triangle[0].to_array()) else {
        return 0.0;
    };
    soup.iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|point| {
                let [x, y, z] = point.to_array();
                [x - origin[0], y - origin[1], z - origin[2]]
            });
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
    /// `1` where the normal is the triangle's cross product as wound, `-1`
    /// where it is reversed.
    sign: f64,
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
    /// A closed body's whole boundary, normals pointing out, or an open
    /// surface, each normal pointing up.
    Upward,
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
        Which::Upward => "surface",
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
        // An open surface's triangles each look up, whatever their winding;
        // one standing vertical has no upper side.
        let sign = match which {
            Which::Upward if !closed && up.abs() <= VERTICAL_TOLERANCE * size => {
                return Err(unavailable("has no upper side where it stands vertical"));
            }
            Which::Upward if !closed => centre[2].signum(),
            _ => outward,
        };
        let looks = match which {
            Which::Boundary | Which::Upward => true,
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
            let value = sign * centre[axis];
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
            sign,
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
        witness: None,
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

/// Each edge of the parts by its two vertices, lower first: the parts
/// using it, each with whether it runs from the lower vertex to the higher
/// as its triangle is wound.
type SharedEdges = BTreeMap<([u64; 3], [u64; 3]), Vec<(usize, bool)>>;

/// The edges the parts' triangles use, as [`SharedEdges`] lists them.
fn shared_edges(parts: &[Part], soup: &[Triangle]) -> SharedEdges {
    let mut edges = SharedEdges::new();
    for (at, part) in parts.iter().enumerate() {
        let corners = soup[part.index].each_ref().map(vertex);
        for (from, to) in [(0, 1), (1, 2), (2, 0)] {
            let (p, q) = (corners[from], corners[to]);
            edges
                .entry((p.min(q), p.max(q)))
                .or_default()
                .push((at, p < q));
        }
    }
    edges
}

/// The parts joined into pieces across the edges where they may lie on
/// one smooth surface: each part's parent, a piece's root its first part.
fn joined(parts: &[Part], edges: &SharedEdges) -> Vec<usize> {
    let mut parents: Vec<usize> = (0..parts.len()).collect();
    for sharing in edges.values() {
        for (i, &(a, _)) in sharing.iter().enumerate() {
            for &(b, _) in &sharing[i + 1..] {
                if smooth(&parts[a], &parts[b]) {
                    let (a, b) = (root(&mut parents, a), root(&mut parents, b));
                    parents[a.max(b)] = a.min(b);
                }
            }
        }
    }
    parents
}

/// A boundary triangle: its index in the soup, its outward normal box and
/// its area `[lower, upper]`.
pub(crate) type BoundaryPart = (usize, FaceNormal, (f64, f64));

/// Every triangle of a closed mesh's boundary, by its index in `soup`,
/// with its outward normal box and its area `[lower, upper]`, widened for
/// a tessellation as a piece's area is.
pub(crate) fn boundary(
    object: &ObjectId,
    soup: &[Triangle],
    tessellation: Option<f64>,
) -> Result<Vec<BoundaryPart>, VerticalExtentError> {
    let parts = parts(object, Which::Boundary, soup, tessellation, true)?;
    Ok(parts
        .iter()
        .map(|part| {
            let (mut lower, mut upper) = area(part);
            if let Some(deviation) = tessellation {
                let band = (2.0 * perimeter(&soup[part.index]) * deviation
                    + std::f64::consts::PI * deviation * deviation)
                    * (1.0 + 8.0 * f64::EPSILON);
                lower = (lower - band).next_down().max(0.0);
                upper = ((upper + band) / part.lean.cos() * (1.0 + 8.0 * f64::EPSILON)).next_up();
            }
            (part.index, part.normal, (lower, upper))
        })
        .collect())
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
    let mut parents = joined(&parts, &shared_edges(&parts, soup));
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
        witness: None,
    };
    FacePieces::try_new(object.clone(), set, pieces, evidence)
}

/// One triangle edge where two pieces meet, or an open surface's border:
/// its ends as its first side's piece runs along it, and its sides, each
/// its piece, its part and whether the face lies left of the edge.
struct Segment {
    from: [u64; 3],
    to: [u64; 3],
    sides: Vec<(usize, usize, bool)>,
}

/// The coordinates of a vertex key.
fn point(key: [u64; 3]) -> [f64; 3] {
    key.map(f64::from_bits)
}

/// Whether `second` continues `first` in a straight line, as computed
/// without rounding.
fn continues(first: &Segment, second: &Segment) -> bool {
    let along = |segment: &Segment| {
        let (from, to) = (point(segment.from), point(segment.to));
        let mut rounded = false;
        let mut out = [0.0; 3];
        for axis in 0..3 {
            let (value, inexact) = difference(to[axis], from[axis]);
            rounded |= inexact;
            out[axis] = value;
        }
        (out, rounded)
    };
    let ((p, first_rounded), (q, second_rounded)) = (along(first), along(second));
    if first_rounded || second_rounded {
        return false;
    }
    let crossed = [(1, 2), (2, 0), (0, 1)]
        .iter()
        .all(|&(i, j)| determinant(p[i], q[j], p[j], q[i], false) == (0.0, 0.0));
    let dot: f64 = (0..3).map(|axis| p[axis] * q[axis]).sum();
    crossed && dot > 0.0
}

/// Whether two normal boxes are the same exact direction.
fn parallel(first: &FaceNormal, second: &FaceNormal) -> bool {
    let (p, q) = (first.lower(), second.lower());
    first.is_exact()
        && second.is_exact()
        && [(1, 2), (2, 0), (0, 1)]
            .iter()
            .all(|&(i, j)| determinant(p[i], q[j], p[j], q[i], false) == (0.0, 0.0))
        && (0..3).map(|axis| p[axis] * q[axis]).sum::<f64>() > 0.0
}

/// One normal box holding the directions of every box of `normals`: the
/// first where all are the same exact direction, else the hull of each
/// scaled to about unit length, rounded outward.
fn merged(normals: &[FaceNormal]) -> Result<FaceNormal, VerticalExtentError> {
    if normals.iter().all(|normal| parallel(&normals[0], normal)) {
        return Ok(normals[0]);
    }
    let mut lower = [f64::INFINITY; 3];
    let mut upper = [f64::NEG_INFINITY; 3];
    for normal in normals {
        let (low, high) = (normal.lower(), normal.upper());
        let scale = length([0, 1, 2].map(|axis| f64::midpoint(low[axis], high[axis])));
        for axis in 0..3 {
            lower[axis] = lower[axis].min((low[axis] / scale).next_down());
            upper[axis] = upper[axis].max((high[axis] / scale).next_up());
        }
    }
    FaceNormal::try_new(lower, upper)
}

/// The box of a vertex: the point itself on an exact mesh, else within the
/// chord deviation of it.
fn corner(key: [u64; 3], tessellation: Option<f64>) -> Result<PointBox, VerticalExtentError> {
    let at = point(key);
    match tessellation {
        None => PointBox::exact(at),
        Some(deviation) => {
            let mut lower = [0.0; 3];
            let mut upper = [0.0; 3];
            for axis in 0..3 {
                let margin = deviation.max(f64::EPSILON * at[axis].abs().max(1.0));
                lower[axis] = (at[axis] - margin).next_down();
                upper[axis] = (at[axis] + margin).next_up();
            }
            PointBox::try_new(lower, upper)
        }
    }
}

/// The edges where the pieces of the mesh `soup` meet, as
/// `measure_pieces` tells the pieces of a closed body's boundary apart, and
/// an open surface's border, each merged into the straight lines its
/// segments make between the same pieces, in the order of their pieces and
/// their first vertices.
///
/// A closed body's normals point out of it; an open surface's each point
/// up, and one standing vertical anywhere is refused, since its upper side
/// is unknown there. An edge used by other than two triangles of a closed
/// body, or by more than two of an open surface, is refused: the body's
/// creases cannot be read there.
pub(crate) fn measure_edges(
    object: &ObjectId,
    soup: &[Triangle],
    tessellation: Option<f64>,
    closed: bool,
) -> Result<FaceEdges, VerticalExtentError> {
    let parts = parts(object, Which::Upward, soup, tessellation, closed)?;
    let edges = shared_edges(&parts, soup);
    let mut parents = joined(&parts, &edges);
    let mut groups: BTreeMap<(usize, Option<usize>), Vec<Segment>> = BTreeMap::new();
    for ((low, high), sharing) in &edges {
        let mut sides: Vec<(usize, usize, bool)> = sharing
            .iter()
            .map(|&(at, forward)| (root(&mut parents, at), at, forward))
            .collect();
        match sides.len() {
            1 if !closed => {}
            2 if sides[0].0 == sides[1].0 => continue,
            2 => {}
            _ => {
                return Err(VerticalExtentError::Unavailable(format!(
                    "the surface of {object} has an edge not shared by exactly two of its \
                     triangles"
                )));
            }
        }
        sides.sort_unstable();
        let forward = sides[0].2;
        let (from, to) = if forward {
            (*low, *high)
        } else {
            (*high, *low)
        };
        // A face lies left of its own winding where its normal is its cross
        // product as wound; the edge runs with that winding where the
        // triangle runs it as the first side does.
        let sides: Vec<(usize, usize, bool)> = sides
            .iter()
            .map(|&(piece, at, along)| (piece, at, (parts[at].sign > 0.0) == (along == forward)))
            .collect();
        let key = (sides[0].0, sides.get(1).map(|side| side.0));
        groups
            .entry(key)
            .or_default()
            .push(Segment { from, to, sides });
    }
    let mut lines = Vec::new();
    for segments in groups.values() {
        let mut starting: BTreeMap<[u64; 3], Vec<usize>> = BTreeMap::new();
        let mut ending: BTreeMap<[u64; 3], Vec<usize>> = BTreeMap::new();
        for (at, segment) in segments.iter().enumerate() {
            starting.entry(segment.from).or_default().push(at);
            ending.entry(segment.to).or_default().push(at);
        }
        // A segment continues into the one starting where it ends, when
        // that vertex joins only those two and they run straight on.
        let next: Vec<Option<usize>> = segments
            .iter()
            .map(|segment| {
                let (out, into) = (&starting.get(&segment.to)?, &ending.get(&segment.to)?);
                (out.len() == 1 && into.len() == 1 && continues(segment, &segments[out[0]]))
                    .then_some(out[0])
            })
            .collect();
        let mut continued = vec![false; segments.len()];
        for following in next.iter().flatten() {
            continued[*following] = true;
        }
        let mut taken = vec![false; segments.len()];
        for first in 0..segments.len() {
            if continued[first] || taken[first] {
                continue;
            }
            let mut chain = vec![first];
            taken[first] = true;
            while let Some(following) = next[*chain.last().unwrap_or(&first)] {
                if taken[following] {
                    break;
                }
                taken[following] = true;
                chain.push(following);
            }
            lines.push(line(segments, &chain, &parts, tessellation)?);
        }
        // Straight segments never close a loop, but every one is listed.
        for at in (0..segments.len()).filter(|at| !taken[*at]) {
            lines.push(line(segments, &[at], &parts, tessellation)?);
        }
    }
    let evidence = Evidence {
        source: object.source.clone(),
        locator: format!("face-edges:{object}"),
        exact: tessellation.is_none(),
        witness: None,
    };
    FaceEdges::try_new(object.clone(), lines, evidence)
}

/// The straight line the segments `chain` make, in order.
fn line(
    segments: &[Segment],
    chain: &[usize],
    parts: &[Part],
    tessellation: Option<f64>,
) -> Result<FaceEdge, VerticalExtentError> {
    let (first, last) = (&segments[chain[0]], &segments[chain[chain.len() - 1]]);
    let sides = (0..first.sides.len())
        .map(|side| {
            let normals: Vec<FaceNormal> = chain
                .iter()
                .map(|&at| parts[segments[at].sides[side].1].normal)
                .collect();
            Ok(EdgeSide::new(merged(&normals)?, first.sides[side].2))
        })
        .collect::<Result<Vec<_>, VerticalExtentError>>()?;
    FaceEdge::try_new(
        corner(first.from, tessellation)?,
        corner(last.to, tessellation)?,
        sides,
    )
}

/// #452: sums of products of coordinates run about a point of the body, so
/// a small body far from the origin, where georeferenced models place
/// theirs, keeps its orientation, its volume and its plan area.
#[cfg(test)]
mod site_coordinate_tests {
    use axiolid_core::{Point2, Point3};
    use axiolid_overlay::{Polygon, Ring};

    use super::signed_volume;
    use crate::geometry::Triangle;

    /// A national grid's distance (1e6 m on every axis), and a
    /// georeferenced northing (5.6e6 m) with an easting and a height.
    const OFFSETS: [[f64; 3]; 2] = [[1.0e6, 1.0e6, 1.0e6], [4.5e5, 5.6e6, 120.0]];

    /// A box 0.1 m by 0.08 m by 0.05 m from `at`, wound outward.
    fn small_box(at: [f64; 3]) -> Vec<Triangle> {
        let [x, y, z] = at;
        let corner = |i: usize| {
            Point3::new(
                x + if i & 1 == 0 { 0.0 } else { 0.1 },
                y + if i & 2 == 0 { 0.0 } else { 0.08 },
                z + if i & 4 == 0 { 0.0 } else { 0.05 },
            )
        };
        [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ]
        .iter()
        .flat_map(|&[a, b, c, d]| {
            [
                [corner(a), corner(b), corner(c)],
                [corner(a), corner(c), corner(d)],
            ]
        })
        .collect()
    }

    /// The spacing of doubles at the largest coordinate near `at`: how far
    /// placing a corner there may move it.
    fn spacing(at: [f64; 3]) -> f64 {
        let largest = at.iter().fold(0.0_f64, |m, c| m.max(c.abs() + 0.1));
        largest.next_up() - largest
    }

    #[test]
    fn a_small_body_far_from_the_origin_keeps_its_volume_and_orientation() {
        let home = signed_volume(&small_box([0.0; 3])) / 6.0;
        assert!((home - 0.1 * 0.08 * 0.05).abs() < 1e-15, "{home}");
        for at in OFFSETS {
            let soup = small_box(at);
            let far = signed_volume(&soup) / 6.0;
            assert!(far > 0.0, "the box at {at:?} turned inside out: {far}");
            // Its corners move by up to the spacing, so its volume by up to
            // its surface (0.034 m²) times that, twice over.
            assert!(
                (far - home).abs() <= 2.0 * 0.034 * spacing(at),
                "the box at {at:?} moved from {home} m³ to {far} m³"
            );
            // About the world origin, as the engine summed before: the
            // terms reach 10²⁰ m³ and the box's 4e-4 m³ is lost in them.
            let naive: f64 = soup.iter().map(|[a, b, c]| a.dot(b.cross(*c)) / 6.0).sum();
            assert!(
                (naive - home).abs() > 0.5 * home,
                "the box at {at:?} no longer shows the cancellation: {naive}"
            );
            // Reversed, it reads inward wherever it lies.
            let inward: Vec<Triangle> = soup.iter().map(|&[a, b, c]| [a, c, b]).collect();
            assert!(signed_volume(&inward) < 0.0, "{at:?}");
        }
    }

    #[test]
    fn a_thin_plan_far_from_the_origin_keeps_its_area_and_centre() {
        for at @ [x, y, _] in OFFSETS {
            // 0.1 m by 0.01 m, anticlockwise.
            let points = vec![
                Point2::new(x, y),
                Point2::new(x + 0.1, y),
                Point2::new(x + 0.1, y + 0.01),
                Point2::new(x, y + 0.01),
            ];
            // Its perimeter (0.22 m) times the spacing, twice over.
            let bound = 2.0 * 0.22 * spacing(at);
            let lifted: Vec<Point3> = points.iter().map(|p| Point3::new(p.x, p.y, 0.0)).collect();
            let area = crate::vertical_surface::plan_area(&lifted);
            assert!((area - 1e-3).abs() <= bound, "({x}, {y}): {area}");
            let polygon = Polygon {
                outer: Ring { points },
                holes: Vec::new(),
            };
            let (area, mx, my) = crate::planar::polygon_moments(&polygon);
            assert!((area - 1e-3).abs() <= bound, "({x}, {y}): {area}");
            let centre = [mx / area, my / area];
            assert!(
                (centre[0] - (x + 0.05)).abs() < 1e-6 && (centre[1] - (y + 0.005)).abs() < 1e-6,
                "the centre of the plan at ({x}, {y}) is {centre:?}"
            );
        }
    }
}
