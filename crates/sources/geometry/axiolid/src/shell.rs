//! Which parts of a mesh bound a solid, read from its positions rather than
//! its indices.
//!
//! The kernel's mesh audit reads a mesh's topology from its indices and
//! counts any zero-area triangle as a defect. Positions that are equal are
//! one point, and a zero-area triangle closing a T-junction leaves the
//! surface closed (#221), so closure is read here from the positions.
//!
//! A source that repeats a coordinate once per face (a triangulated face
//! set written for flat shading, say) gives a closed shell whose faces share
//! no index, and the audit reads every edge as open. Taking equal positions
//! as one vertex changes nothing about the surface; it only lets the
//! closedness the surface has be seen ([`welded_where_closed`], #308).
//!
//! A body of several items may also hold closed items beside open ones (a
//! closed fitting on an open tube). Only the items a measurement needs as
//! solids have to be closed, so the body is split into the pieces its
//! surface connects ([`pieces`]), each closed or not on its own.

use std::collections::BTreeMap;

use axiolid_mesh::{TriMesh, TriangleMeshView};

/// The key of a position: its coordinates' bits, with `-0.0` read as
/// `0.0`, so exactly equal coordinates share a key and nothing else does.
fn key(mesh: &TriMesh, index: usize) -> [u64; 3] {
    mesh.position(index)
        .to_array()
        .map(|coordinate| (coordinate + 0.0).to_bits())
}

/// Each triangle's corners as vertex numbers, positions with equal
/// coordinates sharing one number. `None` when an index addresses no
/// position or a position is not finite: such a mesh bounds nothing.
fn welded_corners(mesh: &TriMesh) -> Option<Vec<[u32; 3]>> {
    let count = mesh.position_count();
    let mut numbers: BTreeMap<[u64; 3], u32> = BTreeMap::new();
    let mut vertex = Vec::with_capacity(count);
    for index in 0..count {
        if !mesh.position(index).is_finite() {
            return None;
        }
        let next = u32::try_from(numbers.len()).ok()?;
        vertex.push(*numbers.entry(key(mesh, index)).or_insert(next));
    }
    (0..TriangleMeshView::triangle_count(mesh))
        .map(|triangle| {
            let corners = TriangleMeshView::triangle(mesh, triangle);
            let mut welded = [0; 3];
            for (slot, corner) in welded.iter_mut().zip(corners) {
                *slot = *vertex.get(usize::try_from(corner).ok()?)?;
            }
            Some(welded)
        })
        .collect()
}

/// Whether the triangles listed form a balanced chain, triangles repeating
/// a vertex left out (they bound nothing): every edge used as often one way
/// as the other, so it may hold edges where several closed sheets meet. Its
/// boundary is empty, so its winding number is a whole number off the
/// surface. A chain with no edge is not balanced.
fn balanced(corners: &[[u32; 3]], triangles: impl Iterator<Item = usize>) -> bool {
    let mut edges: Vec<(u32, u32, i8)> = Vec::new();
    for triangle in triangles {
        let [a, b, c] = corners[triangle];
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
    edges
        .chunk_by(|first, second| (first.0, first.1) == (second.0, second.1))
        .all(|run| run.iter().map(|edge| i64::from(edge.2)).sum::<i64>() == 0)
}

/// Whether the triangles form a closed two-manifold chain: every edge used
/// by exactly two triangles, once each way. Triangles repeating a vertex
/// bound nothing and are left out; zero-area ones count.
fn two_manifold(corners: &[[u32; 3]]) -> bool {
    let mut edges: Vec<(u32, u32, i8)> = Vec::new();
    for [a, b, c] in corners {
        if a == b || b == c || c == a {
            continue;
        }
        for (from, to) in [(a, b), (b, c), (c, a)] {
            edges.push((*from.min(to), *from.max(to), if from < to { 1 } else { -1 }));
        }
    }
    if edges.is_empty() {
        return false;
    }
    edges.sort_unstable();
    edges
        .chunk_by(|first, second| (first.0, first.1) == (second.0, second.1))
        .all(|run| run.len() == 2 && run[0].2 != run[1].2)
}

/// Whether the triangles around every vertex form one fan: each vertex's
/// triangles are joined through the edges they share at it. Two closed
/// shells touching at a corner share that vertex with two fans, so the
/// mesh holding both is no single surface there. Triangles repeating a
/// vertex are left out.
fn single_fans(corners: &[[u32; 3]]) -> bool {
    // (vertex, triangle, the two other corners), grouped by vertex.
    let mut around: Vec<(u32, usize, [u32; 2])> = Vec::new();
    for (triangle, [a, b, c]) in corners.iter().enumerate() {
        if a == b || b == c || c == a {
            continue;
        }
        around.extend([
            (*a, triangle, [*b, *c]),
            (*b, triangle, [*c, *a]),
            (*c, triangle, [*a, *b]),
        ]);
    }
    around.sort_unstable();
    around
        .chunk_by(|first, second| first.0 == second.0)
        .all(|fan| {
            // Join the triangles of one vertex through their other corners.
            let mut parent: Vec<usize> = (0..fan.len()).collect();
            let mut by_corner: BTreeMap<u32, usize> = BTreeMap::new();
            for (slot, (_, _, others)) in fan.iter().enumerate() {
                for other in others {
                    if let Some(held) = by_corner.insert(*other, slot) {
                        let (a, b) = (root(&mut parent, held), root(&mut parent, slot));
                        parent[a] = b;
                    }
                }
            }
            let first = root(&mut parent, 0);
            (1..fan.len()).all(|slot| root(&mut parent, slot) == first)
        })
}

/// `mesh` with every set of exactly equal positions addressed by one index,
/// when that makes it a closed two-manifold chain it was not, with one fan
/// of triangles round every vertex; otherwise `mesh` as it is.
///
/// The positions stay as they are (unused duplicates included), so every
/// coordinate and every triangle is unchanged: only the indices show that
/// faces meeting at equal points share them. A mesh whose items are each
/// closed by their indices keeps them apart, and so does a weld that would
/// join shells along an edge (four triangles on it) or at a corner (two
/// fans round it): shells measured one by one stay apart.
#[must_use]
pub(crate) fn welded_where_closed(mesh: TriMesh) -> TriMesh {
    let as_given: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|corners| [corners[0], corners[1], corners[2]])
        .collect();
    if two_manifold(&as_given) {
        return mesh;
    }
    let Some(welded) = welded_corners(&mesh) else {
        return mesh;
    };
    if welded == as_given || !two_manifold(&welded) || !single_fans(&welded) {
        return mesh;
    }
    // Number each welded vertex by the first position holding it, so the
    // new indices address positions the mesh already has.
    let mut first: BTreeMap<u32, u32> = BTreeMap::new();
    for (corners, original) in welded.iter().zip(&as_given) {
        for (vertex, index) in corners.iter().zip(original) {
            first
                .entry(*vertex)
                .and_modify(|held| *held = (*held).min(*index))
                .or_insert(*index);
        }
    }
    let indices = welded
        .iter()
        .flat_map(|corners| corners.map(|vertex| first[&vertex]))
        .collect();
    TriMesh::new(mesh.positions, indices)
}

/// A connected piece of a mesh's surface.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    /// Its triangles, as indices into the mesh's triangle list.
    pub(crate) triangles: Vec<usize>,
    /// Whether it bounds a solid: its triangles form a balanced chain
    /// (every edge used as often one way as the other), zero-area ones
    /// included and equal positions taken as one vertex.
    pub(crate) closed: bool,
    /// Its lowest and highest elevation.
    pub(crate) z: (f64, f64),
}

/// The representative of `vertex`'s set in a union-find forest.
fn root(parent: &mut [usize], mut vertex: usize) -> usize {
    while parent[vertex] != vertex {
        parent[vertex] = parent[parent[vertex]];
        vertex = parent[vertex];
    }
    vertex
}

/// The pieces of `mesh`'s surface: triangles sharing a vertex (equal
/// positions taken as one) are in one piece. `None` when an index
/// addresses no position or a position is not finite.
///
/// The pieces are ordered by their first triangle, so the split is
/// deterministic.
pub(crate) fn pieces(mesh: &TriMesh) -> Option<Vec<Piece>> {
    let corners = welded_corners(mesh)?;
    let vertices = corners
        .iter()
        .flatten()
        .max()
        .map_or(0, |highest| *highest as usize + 1);
    let mut parent: Vec<usize> = (0..vertices).collect();
    for [a, b, c] in &corners {
        let first = root(&mut parent, *a as usize);
        for other in [*b, *c] {
            let other = root(&mut parent, other as usize);
            parent[other] = first;
        }
    }
    let mut by_root: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pieces: Vec<Piece> = Vec::new();
    for (triangle, [a, _, _]) in corners.iter().enumerate() {
        let group = root(&mut parent, *a as usize);
        let slot = *by_root.entry(group).or_insert_with(|| {
            pieces.push(Piece {
                triangles: Vec::new(),
                closed: false,
                z: (f64::INFINITY, f64::NEG_INFINITY),
            });
            pieces.len() - 1
        });
        let piece = &mut pieces[slot];
        piece.triangles.push(triangle);
        for corner in &mesh.indices[3 * triangle..3 * triangle + 3] {
            let z = mesh.position(*corner as usize).z;
            piece.z = (piece.z.0.min(z), piece.z.1.max(z));
        }
    }
    for piece in &mut pieces {
        piece.closed = balanced(&corners, piece.triangles.iter().copied());
    }
    Some(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiolid_core::Point3;
    use axiolid_mesh::audit_mesh;

    fn tolerance() -> axiolid_core::Tolerance {
        crate::walkable::tolerance().unwrap()
    }

    /// An outward box whose six faces each carry their own four corners,
    /// as a face set written for flat shading does.
    fn box_per_face(min: [f64; 3], max: [f64; 3]) -> TriMesh {
        let corner = |i: usize| {
            Point3::new(
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            )
        };
        // Outward quads by corner number (bit 0 x, bit 1 y, bit 2 z).
        let faces = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for face in faces {
            let base = u32::try_from(positions.len()).unwrap();
            positions.extend(face.map(corner));
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        TriMesh::new(positions, indices)
    }

    #[test]
    fn a_box_repeating_its_corners_per_face_is_one_closed_piece() {
        let mesh = box_per_face([0.0; 3], [1.0, 2.0, 3.0]);
        assert!(!audit_mesh(&mesh, tolerance()).is_closed_two_manifold());
        let pieces = pieces(&mesh).unwrap();
        assert_eq!(pieces.len(), 1);
        assert!(pieces[0].closed);
        assert_eq!(pieces[0].z, (0.0, 3.0));
    }

    #[test]
    fn a_negative_zero_is_the_same_point() {
        let mut mesh = box_per_face([0.0; 3], [1.0; 3]);
        for point in mesh.positions.iter_mut().step_by(5) {
            if point.x == 0.0 {
                point.x = -0.0;
            }
        }
        assert!(pieces(&mesh).unwrap()[0].closed);
    }

    #[test]
    fn a_box_short_of_closing_by_a_rounding_error_stays_open() {
        let mut mesh = box_per_face([0.0; 3], [1.0; 3]);
        mesh.positions[0].x += 1e-12;
        assert!(!pieces(&mesh).unwrap().iter().any(|piece| piece.closed));
    }

    #[test]
    fn an_open_box_is_an_open_piece() {
        let mut mesh = box_per_face([0.0; 3], [1.0; 3]);
        mesh.indices.truncate(30);
        let pieces = pieces(&mesh).unwrap();
        assert_eq!(pieces.len(), 1);
        assert!(!pieces[0].closed);
    }

    #[test]
    fn closed_items_touching_along_an_edge_are_one_closed_piece() {
        // Two boxes sharing the edge x = 1, y = 1, which four triangles use,
        // two each way: one piece, still balanced.
        let first = box_per_face([0.0; 3], [1.0; 3]);
        let second = box_per_face([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]);
        let offset = u32::try_from(first.positions.len()).unwrap();
        let mut positions = first.positions.clone();
        positions.extend(second.positions.iter().copied());
        let mut indices = first.indices.clone();
        indices.extend(second.indices.iter().map(|index| index + offset));
        let pieces = pieces(&TriMesh::new(positions, indices)).unwrap();
        assert_eq!(pieces.len(), 1);
        assert!(pieces[0].closed);
    }

    #[test]
    fn a_closed_item_beside_an_open_one_is_its_own_closed_piece() {
        let closed = box_per_face([0.0; 3], [1.0; 3]);
        let mut open = box_per_face([3.0, 0.0, 0.0], [4.0, 1.0, 5.0]);
        open.indices.truncate(24);
        let offset = u32::try_from(closed.positions.len()).unwrap();
        let mut positions = closed.positions.clone();
        positions.extend(open.positions.iter().copied());
        let mut indices = closed.indices.clone();
        indices.extend(open.indices.iter().map(|index| index + offset));
        let body = TriMesh::new(positions, indices);
        let pieces = pieces(&body).unwrap();
        assert_eq!(pieces.len(), 2);
        assert!(pieces[0].closed);
        assert_eq!(pieces[0].triangles, (0..12).collect::<Vec<_>>());
        assert_eq!(pieces[0].z, (0.0, 1.0));
        assert!(!pieces[1].closed);
        assert_eq!(pieces[1].z, (0.0, 5.0));
    }

    #[test]
    fn a_face_wound_against_its_neighbours_is_not_closed() {
        let mut mesh = box_per_face([0.0; 3], [1.0; 3]);
        mesh.indices[..6].copy_from_slice(&[0, 2, 1, 0, 3, 2]);
        assert!(!pieces(&mesh).unwrap()[0].closed);
    }

    #[test]
    fn a_zero_area_triangle_closing_a_t_junction_counts() {
        // A tetrahedron whose edge 0-1 is split at its midpoint 4 on one
        // face only: a T-junction, which the zero-area triangle 0-1-4 along
        // the edge closes into a balanced chain.
        let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
        let positions = vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(0.5, 0.0, 0.0),
        ];
        let indices = vec![0, 2, 1, 0, 4, 3, 4, 1, 3, 1, 2, 3, 0, 3, 2, 0, 1, 4];
        let mesh = TriMesh::new(positions, indices);
        assert!(!audit_mesh(&mesh, tolerance()).is_closed_two_manifold());
        assert!(pieces(&mesh).unwrap()[0].closed);
    }

    #[test]
    fn a_bad_index_bounds_nothing() {
        let mesh = TriMesh::new(vec![Point3::new(0.0, 0.0, 0.0)], vec![0, 0, 7]);
        assert!(pieces(&mesh).is_none());
        assert_eq!(welded_where_closed(mesh.clone()).indices, mesh.indices);
    }

    fn triangle_points(mesh: &TriMesh) -> Vec<Point3> {
        mesh.indices
            .iter()
            .map(|index| mesh.positions[*index as usize])
            .collect()
    }

    #[test]
    fn a_box_repeating_its_corners_per_face_is_welded_closed() {
        let mesh = box_per_face([0.0; 3], [1.0, 2.0, 3.0]);
        assert!(!audit_mesh(&mesh, tolerance()).is_closed_two_manifold());
        let welded = welded_where_closed(mesh.clone());
        assert!(audit_mesh(&welded, tolerance()).is_closed_two_manifold());
        assert_eq!(welded.positions, mesh.positions);
        assert_eq!(triangle_points(&welded), triangle_points(&mesh));
        // A negative zero is the same coordinate.
        let mut signed = box_per_face([0.0; 3], [1.0; 3]);
        for point in signed.positions.iter_mut().step_by(5) {
            if point.x == 0.0 {
                point.x = -0.0;
            }
        }
        let welded = welded_where_closed(signed);
        assert!(audit_mesh(&welded, tolerance()).is_closed_two_manifold());
    }

    /// Only a weld that closes the mesh is kept: a corner off by a rounding
    /// error, a missing face or a face wound against its neighbours leaves
    /// the mesh as given.
    #[test]
    fn a_weld_that_does_not_close_the_mesh_is_not_kept() {
        let mut rounded = box_per_face([0.0; 3], [1.0; 3]);
        rounded.positions[0].x += 1e-12;
        let mut open = box_per_face([0.0; 3], [1.0; 3]);
        open.indices.truncate(30);
        let mut inverted = box_per_face([0.0; 3], [1.0; 3]);
        inverted.indices[..6].copy_from_slice(&[0, 2, 1, 0, 3, 2]);
        for mesh in [rounded, open, inverted] {
            assert_eq!(welded_where_closed(mesh.clone()).indices, mesh.indices);
        }
    }

    /// Boxes that repeat their corners per face and touch another box at a
    /// face or a corner are not welded: that would join two shells into one
    /// with an edge four triangles use, or a corner two fans meet at. A box
    /// without its top stays open.
    #[test]
    fn a_weld_joining_shells_or_leaving_a_hole_is_not_kept() {
        let joined = |second: TriMesh| {
            let first = box_per_face([0.0; 3], [1.0; 3]);
            let offset = u32::try_from(first.positions.len()).unwrap();
            let mut positions = first.positions.clone();
            positions.extend(second.positions.iter().copied());
            let mut indices = first.indices.clone();
            indices.extend(second.indices.iter().map(|index| index + offset));
            TriMesh::new(positions, indices)
        };
        let sharing_a_face = joined(box_per_face([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]));
        let sharing_a_corner = joined(box_per_face([1.0; 3], [2.0; 3]));
        let mut without_top = box_per_face([0.0; 3], [1.0; 3]);
        without_top.indices.drain(6..12);
        for mesh in [sharing_a_face, sharing_a_corner, without_top] {
            assert_eq!(welded_where_closed(mesh.clone()).indices, mesh.indices);
        }
        // Apart, the two boxes are welded, each into its own shell.
        let apart = joined(box_per_face([3.0; 3], [4.0; 3]));
        let welded = welded_where_closed(apart.clone());
        assert_ne!(welded.indices, apart.indices);
        assert!(audit_mesh(&welded, tolerance()).is_closed_two_manifold());
    }

    /// Two boxes sharing an edge, each closed by its own indices, stay two
    /// closed shells: welded, that edge would carry four triangles.
    #[test]
    fn closed_items_touching_along_an_edge_are_not_welded() {
        let first = welded_where_closed(box_per_face([0.0; 3], [1.0; 3]));
        let second = welded_where_closed(box_per_face([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]));
        let offset = u32::try_from(first.positions.len()).unwrap();
        let mut positions = first.positions.clone();
        positions.extend(second.positions.iter().copied());
        let mut indices = first.indices.clone();
        indices.extend(second.indices.iter().map(|index| index + offset));
        let both = TriMesh::new(positions, indices);
        assert!(audit_mesh(&both, tolerance()).is_closed_two_manifold());
        assert_eq!(welded_where_closed(both.clone()).indices, both.indices);
    }
}
