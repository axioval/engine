//! Signed distances from a body to a class of another body's faces, over
//! real Axiolid geometry: the covers and protrusions containment checks
//! judge.

use std::f64::consts::TAU;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    FaceClass, FaceDistanceError, FaceDistanceRequest, ProximityService, SignedDistanceInterval,
};
use axioval_ir::{ObjectId, SourceId};

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

/// A closed axis-aligned box, outward wound unless `inward`.
fn cuboid_wound(min: [f64; 3], max: [f64; 3], inward: bool) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let mut indices = vec![
        0, 2, 1, 0, 3, 2, // bottom
        4, 5, 6, 4, 6, 7, // top
        0, 1, 5, 0, 5, 4, // front
        3, 7, 6, 3, 6, 2, // back
        0, 4, 7, 0, 7, 3, // left
        1, 2, 6, 1, 6, 5, // right
    ];
    if inward {
        for triangle in indices.chunks_mut(3) {
            triangle.swap(1, 2);
        }
    }
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z0),
            Point3::new(x1, y0, z0),
            Point3::new(x1, y1, z0),
            Point3::new(x0, y1, z0),
            Point3::new(x0, y0, z1),
            Point3::new(x1, y0, z1),
            Point3::new(x1, y1, z1),
            Point3::new(x0, y1, z1),
        ],
        indices,
    )
}

fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    cuboid_wound(min, max, false)
}

/// A closed vertical prism approximating a cylinder with `sides` chords.
fn round_column(centre: [f64; 2], radius: f64, z: [f64; 2], sides: u32) -> TriMesh {
    let mut positions = Vec::new();
    for level in z {
        for side in 0..sides {
            let angle = TAU * f64::from(side) / f64::from(sides);
            positions.push(Point3::new(
                centre[0] + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
                level,
            ));
        }
    }
    positions.push(Point3::new(centre[0], centre[1], z[0]));
    positions.push(Point3::new(centre[0], centre[1], z[1]));
    let (bottom_centre, top_centre) = (2 * sides, 2 * sides + 1);
    let mut indices = Vec::new();
    for side in 0..sides {
        let next = (side + 1) % sides;
        let (b0, b1, t0, t1) = (side, next, side + sides, next + sides);
        indices.extend([b0, b1, t1, b0, t1, t0]);
        indices.extend([bottom_centre, b1, b0]);
        indices.extend([top_centre, t0, t1]);
    }
    TriMesh::new(positions, indices)
}

/// A 4 m long, 0.3 m thick, 3 m high wall.
fn wall() -> TriMesh {
    cuboid([0.0, 0.0, 0.0], [4.0, 0.3, 3.0])
}

fn measure(
    geometry: AxiolidGeometry,
    faces: FaceClass,
) -> Result<SignedDistanceInterval, FaceDistanceError> {
    AxiolidProximityService::new(geometry)
        .measure_face_distance(
            &FaceDistanceRequest::try_new(id("column"), id("wall"), faces).unwrap(),
        )
        .map(|measured| {
            assert_eq!(measured.request().faces(), faces);
            measured.signed()
        })
}

fn assert_exact(interval: SignedDistanceInterval, expected: f64) {
    assert!(
        (interval.lower_metres() - expected).abs() < 1e-9
            && (interval.upper_metres() - expected).abs() < 1e-9,
        "{interval:?}, expected {expected}"
    );
}

/// A 0.2 m square column in the wall, 0.02 m from the front face, 0.08 m
/// from the back, 0.5 m under the top and 0.4 m over the bottom.
fn column_in_wall() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("column"), cuboid([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]))
}

/// A column wholly inside its host has its cover exactly, per face class.
#[test]
fn a_column_inside_a_wall_has_its_cover_to_each_face_class() {
    assert_exact(measure(column_in_wall(), FaceClass::Side).unwrap(), 0.02);
    assert_exact(measure(column_in_wall(), FaceClass::Top).unwrap(), 0.5);
    assert_exact(measure(column_in_wall(), FaceClass::Bottom).unwrap(), 0.4);
    assert_exact(measure(column_in_wall(), FaceClass::Any).unwrap(), 0.02);
}

/// The end faces are side faces too: a column near the wall's end has its
/// cover to the end.
#[test]
fn an_end_face_is_a_side_face() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("column"), cuboid([0.01, 0.05, 0.4], [0.21, 0.25, 2.5]));
    assert_exact(measure(geometry, FaceClass::Side).unwrap(), 0.01);
}

/// A host wound inward has the same outside: classes follow the solid, not
/// the winding.
#[test]
fn an_inward_wound_host_has_the_same_faces() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(
            id("wall"),
            cuboid_wound([0.0, 0.0, 0.0], [4.0, 0.3, 3.0], true),
        )
        .with_mesh(id("column"), cuboid([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]));
    assert_exact(measure(geometry, FaceClass::Top).unwrap(), 0.5);
}

/// A column reaching 0.4 m above the wall protrudes: its distance to the top
/// is negative, at least as deep as its witnessed top corners, and its side
/// cover can no longer be positive.
#[test]
fn a_column_through_the_top_protrudes_by_its_reach() {
    let geometry = || {
        AxiolidGeometry::new()
            .with_mesh(id("wall"), wall())
            .with_mesh(id("column"), cuboid([1.0, 0.05, 0.5], [1.2, 0.25, 3.4]))
    };
    let top = measure(geometry(), FaceClass::Top).unwrap();
    assert!((top.upper_metres() + 0.4).abs() < 1e-9, "{top:?}");
    assert!(
        top.lower_metres() <= -0.4 && top.lower_metres() > -0.5,
        "{top:?}"
    );
    let side = measure(geometry(), FaceClass::Side).unwrap();
    assert!(side.upper_metres() <= 0.05 + 1e-9, "{side:?}");
    assert!(side.lower_metres() < 0.0, "{side:?}");
}

/// A face flush with the host's face has no cover.
#[test]
fn a_flush_column_has_no_cover() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_mesh(id("column"), cuboid([1.0, 0.0, 0.5], [1.2, 0.2, 2.5]));
    let side = measure(geometry, FaceClass::Side).unwrap();
    assert!(side.upper_metres().abs() < 1e-9, "{side:?}");
    assert!(side.lower_metres() <= 0.0, "{side:?}");
}

/// A round column's chords lie inside its true surface: the cover widens by
/// the chord deviation and is not exact.
#[test]
fn a_tessellated_column_widens_by_its_deviation() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wall())
        .with_tessellated_mesh(
            id("column"),
            round_column([2.0, 0.15], 0.1, [0.5, 2.5], 32),
            0.001,
        );
    let measured = AxiolidProximityService::new(geometry)
        .measure_face_distance(
            &FaceDistanceRequest::try_new(id("column"), id("wall"), FaceClass::Side).unwrap(),
        )
        .unwrap();
    assert!(!measured.evidence().exact);
    let signed = measured.signed();
    // The 32-gon touches its circle at the vertices on the y axis.
    assert!((signed.lower_metres() - 0.049).abs() < 1e-9, "{signed:?}");
    assert!((signed.upper_metres() - 0.051).abs() < 1e-9, "{signed:?}");
}

#[test]
fn hosts_that_cannot_state_their_faces_are_refused() {
    let tessellated = AxiolidGeometry::new()
        .with_tessellated_mesh(id("wall"), wall(), 0.001)
        .with_mesh(id("column"), cuboid([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]));
    assert_eq!(
        measure(tessellated, FaceClass::Side),
        Err(FaceDistanceError::InexactHost)
    );
    let open = AxiolidGeometry::new()
        .with_mesh(
            id("wall"),
            TriMesh::new(
                vec![
                    Point3::new(0.0, 0.0, 0.0),
                    Point3::new(4.0, 0.0, 0.0),
                    Point3::new(4.0, 0.0, 3.0),
                ],
                vec![0, 1, 2],
            ),
        )
        .with_mesh(id("column"), cuboid([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]));
    assert_eq!(
        measure(open, FaceClass::Side),
        Err(FaceDistanceError::NotClosed)
    );
    let bodiless = AxiolidGeometry::new()
        .with_mesh(id("column"), cuboid([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]))
        .with_no_body(id("wall"));
    assert_eq!(
        measure(bodiless, FaceClass::Side),
        Err(FaceDistanceError::NoBody)
    );
    let missing =
        AxiolidGeometry::new().with_mesh(id("column"), cuboid([1.0, 0.02, 0.4], [1.2, 0.22, 2.5]));
    assert_eq!(
        measure(missing, FaceClass::Side),
        Err(FaceDistanceError::Unavailable)
    );
}

/// A roof pitched at exactly 45° is neither clearly top nor clearly side.
#[test]
fn a_face_at_45_degrees_leaves_its_class_undecided() {
    // A triangular prism: a wedge whose sloped face rises at 45°.
    let wedge = TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
            Point3::new(4.0, 0.0, 4.0),
            Point3::new(0.0, 2.0, 0.0),
            Point3::new(4.0, 2.0, 0.0),
            Point3::new(4.0, 2.0, 4.0),
        ],
        vec![
            0, 2, 1, 3, 4, 5, // ends
            0, 1, 4, 0, 4, 3, // bottom
            1, 2, 5, 1, 5, 4, // back
            0, 3, 5, 0, 5, 2, // slope
        ],
    );
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), wedge)
        .with_mesh(id("column"), cuboid([3.0, 0.5, 0.5], [3.5, 1.0, 1.0]));
    assert_eq!(
        measure(geometry.clone(), FaceClass::Top),
        Err(FaceDistanceError::AmbiguousFace)
    );
    assert!(measure(geometry, FaceClass::Any).is_ok());
}
