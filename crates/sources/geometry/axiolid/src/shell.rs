//! Which parts of a mesh bound a solid, read from its positions rather than
//! its indices.
//!
//! The kernel's mesh audit reads a mesh's topology from its indices and
//! counts any zero-area triangle as a defect. Positions that are equal are
//! one point, and a zero-area triangle closing a T-junction leaves the
//! surface closed (#221), so closure is read here from the positions.
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
    }
}
