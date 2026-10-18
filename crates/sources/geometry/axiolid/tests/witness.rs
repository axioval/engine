//! Where a proximity measurement happens (#388): its witness points and
//! the bracket of an overlap's box, on bodies whose answer is known.

#![allow(clippy::float_cmp)] // Box corners are exact: witnesses are compared exactly.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    LocalOrigin, LocalOrigins, ProximityProjection, ProximityRequest, ProximityService,
};
use axioval_ir::{ObjectId, SourceId, WitnessKind};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
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
        vec![
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            3, 7, 6, 3, 6, 2, // back
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ],
    )
}

fn measure(geometry: AxiolidGeometry) -> axioval_engine::ProximityEvidence {
    AxiolidProximityService::new(geometry)
        .measure_proximity(&ProximityRequest::try_new(id("a"), id("b")).unwrap())
        .expect("measurable")
}

#[test]
fn two_boxes_apart_are_witnessed_by_their_nearest_corners() {
    // The nearest points are unique: corner (1, 1, 1) of `a` and corner
    // (3, 2, 2) of `b`, √6 m apart.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0; 3], [1.0; 3]))
        .with_mesh(id("b"), cuboid([3.0, 2.0, 2.0], [4.0, 3.0, 3.0]));
    let measured = measure(geometry);
    assert_eq!(measured.separation_metres(), 6.0_f64.sqrt());
    let witness = measured.witness().expect("the closest pair is found");
    assert_eq!(witness.kind(), WitnessKind::Separation);
    assert_eq!(witness.point_a().metres(), [1.0, 1.0, 1.0]);
    assert_eq!(witness.point_b().metres(), [3.0, 2.0, 2.0]);
    assert!(witness.is_exact());
    // Bodies apart share no region.
    assert_eq!(witness.extent(), None);
    // The evidence a finding cites carries it.
    assert_eq!(measured.evidence().witness.as_deref(), Some(witness));
}

#[test]
fn two_overlapping_boxes_are_witnessed_by_the_deepest_corner_and_the_shared_box() {
    // The boxes share the unit cube [1, 2]³. The deepest point of `a`
    // inside `b` is its corner (2, 2, 2), a metre from each of `b`'s faces
    // through (1, 1, 1); the first of them in `b`'s faces is its bottom.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0; 3], [2.0; 3]))
        .with_mesh(id("b"), cuboid([1.0; 3], [3.0; 3]));
    let measured = measure(geometry);
    assert_eq!(measured.penetration_metres(), Some(1.0));
    let witness = measured.witness().expect("the penetration is witnessed");
    assert_eq!(witness.kind(), WitnessKind::PenetrationIntoCounterpart);
    assert_eq!(witness.point_a().metres(), [2.0, 2.0, 2.0]);
    assert_eq!(witness.point_b().metres(), [2.0, 2.0, 1.0]);
    let extent = witness.extent().expect("the shared region is measured");
    let outer = extent.outer();
    assert_eq!((outer.min(), outer.max()), ([1.0; 3], [2.0; 3]));
    // Its corners are witnessed, so the bracket is closed.
    assert_eq!(extent.inner(), Some(outer));
    assert!(extent.is_exact() && witness.is_exact());
}

#[test]
fn a_box_inside_another_is_witnessed_by_its_deepest_point() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([1.0; 3], [2.0; 3]))
        .with_mesh(id("b"), cuboid([0.0; 3], [4.0; 3]));
    let measured = measure(geometry);
    let witness = measured.witness().expect("the containment is witnessed");
    assert_eq!(witness.kind(), WitnessKind::PenetrationIntoCounterpart);
    let (a, b) = (witness.point_a(), witness.point_b());
    assert!((a.distance(&b) - measured.penetration_metres().unwrap()).abs() < 1e-12);
    let outer = witness.extent().expect("the shared region").outer();
    assert_eq!((outer.min(), outer.max()), ([1.0; 3], [2.0; 3]));
}

#[test]
fn tessellated_witnesses_carry_each_bodys_deviation() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("a"), cuboid([0.0; 3], [1.0; 3]), 0.002)
        .with_mesh(id("b"), cuboid([3.0, 2.0, 2.0], [4.0, 3.0, 3.0]));
    let measured = measure(geometry);
    assert!(!measured.evidence().exact);
    let witness = measured.witness().expect("the closest pair is found");
    assert_eq!(witness.point_a().deviation_metres(), 0.002);
    assert_eq!(witness.point_b().deviation_metres(), 0.0);
    assert!(!witness.is_exact());
}

#[test]
fn witnesses_are_reported_in_model_coordinates() {
    let origin = LocalOrigin::new([1000.0, 2000.0]).unwrap();
    let geometry = AxiolidGeometry::new()
        .with_local_origins(LocalOrigins::new([(source(), origin)]))
        .with_mesh(id("a"), cuboid([0.0; 3], [2.0; 3]))
        .with_mesh(id("b"), cuboid([1.0; 3], [3.0; 3]));
    let witness = measure(geometry).witness().copied().expect("witnessed");
    assert_eq!(witness.point_a().metres(), [1002.0, 2002.0, 2.0]);
    let outer = witness.extent().unwrap().outer();
    assert_eq!(outer.min(), [1001.0, 2001.0, 1.0]);
    assert_eq!(outer.max(), [1002.0, 2002.0, 2.0]);
}

#[test]
fn a_distance_in_space_carries_its_closest_pair_and_no_other_projection_does() {
    let service = AxiolidProximityService::new(
        AxiolidGeometry::new()
            .with_mesh(id("a"), cuboid([0.0; 3], [1.0; 3]))
            .with_mesh(id("b"), cuboid([3.0, 2.0, 2.0], [4.0, 3.0, 3.0])),
    );
    let space = service
        .measure_distance(&ProximityRequest::try_new(id("a"), id("b")).unwrap())
        .unwrap();
    let witness = space.witness().expect("the closest pair");
    assert_eq!(witness.point_a().metres(), [1.0, 1.0, 1.0]);
    assert_eq!(witness.point_b().metres(), [3.0, 2.0, 2.0]);
    let plan = service
        .measure_distance(
            &ProximityRequest::projected(id("a"), id("b"), ProximityProjection::Horizontal)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(plan.witness(), None);
}
