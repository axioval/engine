//! The edges where a body's pieces meet (#329): a gable roof with a
//! dormer, a ridge split into collinear segments, an open roof surface's
//! border and a tessellated roof.
#![allow(clippy::float_cmp)]

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidVerticalExtentService};
use axioval_engine::{FaceEdge, VerticalExtentError, VerticalExtentService};
use axioval_ir::{ObjectId, SourceId};

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

type P = [f64; 3];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: P, b: P) -> P {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A mesh of the triangles `faces`, each wound so its normal leans to the
/// face's `outward` direction (or `None` to keep it as written), corners
/// with equal coordinates shared.
fn mesh(faces: &[(P, &[[P; 3]])], keep: bool) -> TriMesh {
    let mut points: Vec<P> = Vec::new();
    let mut indices = Vec::new();
    for (outward, triangles) in faces {
        for triangle in *triangles {
            let [a, b, c] = *triangle;
            let normal = cross(sub(b, a), sub(c, a));
            let ordered = if keep || dot(normal, *outward) > 0.0 {
                [a, b, c]
            } else {
                [a, c, b]
            };
            for corner in ordered {
                let at = points.iter().position(|p| *p == corner).unwrap_or_else(|| {
                    points.push(corner);
                    points.len() - 1
                });
                indices.push(u32::try_from(at).unwrap());
            }
        }
    }
    TriMesh::new(
        points
            .iter()
            .map(|p| Point3::new(p[0], p[1], p[2]))
            .collect(),
        indices,
    )
}

/// A closed gable roof 10 m along x over y in `[0, 8]`, eaves at 3 m and
/// the ridge at 7 m along y = 4, with a gable dormer 2 m wide (x in
/// `[4, 6]`) rising from the eaves at y = 0: its eaves at 5 m, its ridge
/// at 6 m running back to where its roof meets the main roof in two
/// valleys, (4, 2, 5) to (5, 3, 6) and (6, 2, 5) to (5, 3, 6).
pub fn dormer_roof() -> TriMesh {
    mesh(
        &[
            (
                [0.0, 0.0, -1.0],
                &[
                    [[0.0, 0.0, 3.0], [4.0, 0.0, 3.0], [0.0, 8.0, 3.0]],
                    [[4.0, 0.0, 3.0], [10.0, 8.0, 3.0], [0.0, 8.0, 3.0]],
                    [[4.0, 0.0, 3.0], [6.0, 0.0, 3.0], [10.0, 8.0, 3.0]],
                    [[6.0, 0.0, 3.0], [10.0, 0.0, 3.0], [10.0, 8.0, 3.0]],
                ],
            ),
            (
                [0.0, -1.0, 1.0],
                &[
                    [[0.0, 0.0, 3.0], [4.0, 0.0, 3.0], [4.0, 2.0, 5.0]],
                    [[0.0, 0.0, 3.0], [4.0, 2.0, 5.0], [0.0, 4.0, 7.0]],
                    [[0.0, 4.0, 7.0], [4.0, 2.0, 5.0], [5.0, 3.0, 6.0]],
                    [[0.0, 4.0, 7.0], [5.0, 3.0, 6.0], [10.0, 4.0, 7.0]],
                    [[5.0, 3.0, 6.0], [6.0, 2.0, 5.0], [10.0, 4.0, 7.0]],
                    [[6.0, 2.0, 5.0], [10.0, 0.0, 3.0], [10.0, 4.0, 7.0]],
                    [[6.0, 2.0, 5.0], [6.0, 0.0, 3.0], [10.0, 0.0, 3.0]],
                ],
            ),
            (
                [0.0, 1.0, 1.0],
                &[
                    [[0.0, 4.0, 7.0], [10.0, 4.0, 7.0], [10.0, 8.0, 3.0]],
                    [[0.0, 4.0, 7.0], [10.0, 8.0, 3.0], [0.0, 8.0, 3.0]],
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                &[[[0.0, 0.0, 3.0], [0.0, 4.0, 7.0], [0.0, 8.0, 3.0]]],
            ),
            (
                [1.0, 0.0, 0.0],
                &[[[10.0, 0.0, 3.0], [10.0, 8.0, 3.0], [10.0, 4.0, 7.0]]],
            ),
            (
                [0.0, -1.0, 0.0],
                &[
                    [[4.0, 0.0, 3.0], [6.0, 0.0, 3.0], [6.0, 0.0, 5.0]],
                    [[4.0, 0.0, 3.0], [6.0, 0.0, 5.0], [5.0, 0.0, 6.0]],
                    [[4.0, 0.0, 3.0], [5.0, 0.0, 6.0], [4.0, 0.0, 5.0]],
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                &[[[4.0, 0.0, 3.0], [4.0, 0.0, 5.0], [4.0, 2.0, 5.0]]],
            ),
            (
                [1.0, 0.0, 0.0],
                &[[[6.0, 0.0, 3.0], [6.0, 2.0, 5.0], [6.0, 0.0, 5.0]]],
            ),
            (
                [-1.0, 0.0, 1.0],
                &[
                    [[4.0, 0.0, 5.0], [5.0, 0.0, 6.0], [5.0, 3.0, 6.0]],
                    [[4.0, 0.0, 5.0], [5.0, 3.0, 6.0], [4.0, 2.0, 5.0]],
                ],
            ),
            (
                [1.0, 0.0, 1.0],
                &[
                    [[6.0, 0.0, 5.0], [6.0, 2.0, 5.0], [5.0, 3.0, 6.0]],
                    [[6.0, 0.0, 5.0], [5.0, 3.0, 6.0], [5.0, 0.0, 6.0]],
                ],
            ),
        ],
        false,
    )
}

/// The line's ends, sorted, as exact points.
fn ends(edge: &FaceEdge) -> [P; 2] {
    let (start, end) = (edge.start(), edge.end());
    assert!(start.is_exact() && end.is_exact());
    let mut ends = [start.lower(), end.lower()];
    ends.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ends
}

/// The direction from the line into each face beside it, from its normal
/// and side.
fn into_faces(edge: &FaceEdge) -> Vec<P> {
    let along = sub(edge.end().lower(), edge.start().lower());
    edge.sides()
        .iter()
        .map(|side| {
            let normal = side.normal().lower();
            if side.left() {
                cross(normal, along)
            } else {
                cross(along, normal)
            }
        })
        .collect()
}

fn find(edges: &[FaceEdge], a: P, b: P) -> &FaceEdge {
    let mut wanted = [a, b];
    wanted.sort_by(|p, q| p.partial_cmp(q).unwrap());
    edges
        .iter()
        .find(|edge| ends(edge) == wanted)
        .unwrap_or_else(|| panic!("no line {a:?} - {b:?}"))
}

#[test]
fn a_gable_roof_with_a_dormer_lists_every_line_its_pieces_meet_at() {
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new().with_mesh(id("roof"), dormer_roof()),
    );
    let edges = service.measure_face_edges(&id("roof")).unwrap();
    assert!(edges.evidence().exact);
    assert_eq!(edges.evidence().locator, "face-edges:cad:model/roof");
    // Ten pieces meet along 22 lines, every one with two sides.
    assert_eq!(edges.edges().len(), 22, "{:#?}", edges.edges());
    assert!(edges.edges().iter().all(|edge| edge.sides().len() == 2));
    // The ridge: both faces fall away from it, one each way.
    let ridge = find(edges.edges(), [0.0, 4.0, 7.0], [10.0, 4.0, 7.0]);
    let into = into_faces(ridge);
    assert!(into.iter().all(|direction| direction[2] < 0.0), "{into:?}");
    assert!(into[0][1] * into[1][1] < 0.0, "{into:?}");
    // The valley: the dormer's roof rises away from it, the main roof
    // too, and each face lies above the other's plane (concave).
    let valley = find(edges.edges(), [4.0, 2.0, 5.0], [5.0, 3.0, 6.0]);
    let into = into_faces(valley);
    let normals: Vec<P> = valley.sides().iter().map(|s| s.normal().lower()).collect();
    assert!(dot(normals[0], into[1]) > 0.0, "{into:?} {normals:?}");
    // The eaves: the main roof rises away from it; below lies the bottom.
    let eaves = find(edges.edges(), [0.0, 0.0, 3.0], [4.0, 0.0, 3.0]);
    let into = into_faces(eaves);
    assert!(into.iter().any(|direction| direction[2] > 0.0), "{into:?}");
    // Every face lies on the side its normal and winding say: into each
    // face is square to the line and to its normal.
    for edge in edges.edges() {
        let along = sub(edge.end().lower(), edge.start().lower());
        for (side, into) in edge.sides().iter().zip(into_faces(edge)) {
            assert_eq!(dot(into, along), 0.0);
            assert_eq!(dot(into, side.normal().lower()), 0.0);
        }
    }
}

/// A gable roof whose ridge vertex at x = 5 splits its ridge and eaves
/// into collinear segments: each is listed as one line.
fn split_gable() -> TriMesh {
    mesh(
        &[
            (
                [0.0, 0.0, -1.0],
                &[
                    [[0.0, 0.0, 3.0], [5.0, 0.0, 3.0], [0.0, 8.0, 3.0]],
                    [[5.0, 0.0, 3.0], [5.0, 8.0, 3.0], [0.0, 8.0, 3.0]],
                    [[5.0, 0.0, 3.0], [10.0, 0.0, 3.0], [5.0, 8.0, 3.0]],
                    [[10.0, 0.0, 3.0], [10.0, 8.0, 3.0], [5.0, 8.0, 3.0]],
                ],
            ),
            (
                [0.0, -1.0, 1.0],
                &[
                    [[0.0, 0.0, 3.0], [5.0, 0.0, 3.0], [5.0, 4.0, 7.0]],
                    [[0.0, 0.0, 3.0], [5.0, 4.0, 7.0], [0.0, 4.0, 7.0]],
                    [[5.0, 0.0, 3.0], [10.0, 0.0, 3.0], [5.0, 4.0, 7.0]],
                    [[10.0, 0.0, 3.0], [10.0, 4.0, 7.0], [5.0, 4.0, 7.0]],
                ],
            ),
            (
                [0.0, 1.0, 1.0],
                &[
                    [[0.0, 4.0, 7.0], [5.0, 4.0, 7.0], [0.0, 8.0, 3.0]],
                    [[5.0, 4.0, 7.0], [5.0, 8.0, 3.0], [0.0, 8.0, 3.0]],
                    [[5.0, 4.0, 7.0], [10.0, 4.0, 7.0], [5.0, 8.0, 3.0]],
                    [[10.0, 4.0, 7.0], [10.0, 8.0, 3.0], [5.0, 8.0, 3.0]],
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                &[[[0.0, 0.0, 3.0], [0.0, 4.0, 7.0], [0.0, 8.0, 3.0]]],
            ),
            (
                [1.0, 0.0, 0.0],
                &[[[10.0, 0.0, 3.0], [10.0, 8.0, 3.0], [10.0, 4.0, 7.0]]],
            ),
        ],
        false,
    )
}

#[test]
fn collinear_segments_between_the_same_faces_are_one_line() {
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new().with_mesh(id("roof"), split_gable()),
    );
    let edges = service.measure_face_edges(&id("roof")).unwrap();
    // Ridge, two eaves, four verges, and the bottom's two gable edges.
    assert_eq!(edges.edges().len(), 9, "{:#?}", edges.edges());
    find(edges.edges(), [0.0, 4.0, 7.0], [10.0, 4.0, 7.0]);
    find(edges.edges(), [0.0, 0.0, 3.0], [10.0, 0.0, 3.0]);
    find(edges.edges(), [0.0, 8.0, 3.0], [10.0, 8.0, 3.0]);
}

#[test]
fn an_open_roof_surface_lists_its_border_with_one_side_looking_up() {
    // The two slopes alone, wound either way.
    let open = mesh(
        &[
            (
                [0.0, -1.0, 1.0],
                &[
                    [[0.0, 0.0, 3.0], [10.0, 0.0, 3.0], [10.0, 4.0, 7.0]],
                    [[0.0, 0.0, 3.0], [0.0, 4.0, 7.0], [10.0, 4.0, 7.0]],
                ],
            ),
            (
                [0.0, 1.0, 1.0],
                &[
                    [[0.0, 4.0, 7.0], [10.0, 4.0, 7.0], [10.0, 8.0, 3.0]],
                    [[0.0, 4.0, 7.0], [10.0, 8.0, 3.0], [0.0, 8.0, 3.0]],
                ],
            ),
        ],
        true,
    );
    let service =
        AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_mesh(id("roof"), open));
    let edges = service.measure_face_edges(&id("roof")).unwrap();
    assert_eq!(edges.edges().len(), 7, "{:#?}", edges.edges());
    let ridge = find(edges.edges(), [0.0, 4.0, 7.0], [10.0, 4.0, 7.0]);
    assert_eq!(ridge.sides().len(), 2);
    let eaves = find(edges.edges(), [0.0, 0.0, 3.0], [10.0, 0.0, 3.0]);
    assert_eq!(eaves.sides().len(), 1);
    assert!(eaves.sides()[0].normal().lower()[2] > 0.0);
    // The slope rises away from its eaves.
    assert!(into_faces(eaves)[0][2] > 0.0);
    assert!(edges.edges().iter().all(|edge| {
        edge.sides()
            .iter()
            .all(|side| side.normal().lower()[2] > 0.0)
    }));
    // An open surface standing vertical somewhere has no upper side there.
    let wall = mesh(
        &[(
            [0.0, -1.0, 0.0],
            &[[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]]],
        )],
        true,
    );
    let service =
        AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_mesh(id("wall"), wall));
    assert!(matches!(
        service.measure_face_edges(&id("wall")),
        Err(VerticalExtentError::Unavailable(_))
    ));
}

#[test]
fn a_tessellated_roof_widens_its_lines_and_is_never_exact() {
    let service = AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_tessellated_mesh(
        id("roof"),
        dormer_roof(),
        0.001,
    ));
    let edges = service.measure_face_edges(&id("roof")).unwrap();
    assert!(!edges.evidence().exact);
    assert_eq!(edges.edges().len(), 22);
    for edge in edges.edges() {
        for end in [edge.start(), edge.end()] {
            assert!(!end.is_exact());
            for axis in 0..3 {
                assert!(end.upper()[axis] - end.lower()[axis] >= 0.002);
            }
        }
        assert!(edge.sides().iter().all(|side| !side.normal().is_exact()));
    }
}
