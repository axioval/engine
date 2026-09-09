//! Free space measured from real geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, ClearanceOutcome, ClearanceRequest, ClearanceShape, FreeAreaRequest,
    FreeSpaceError, FreeSpaceService, MetricDirection, MetricFrame, MetricPoint, MobilityProfile,
    PlacementRequest,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

fn body(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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
        vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
    )
}

/// A clearance volume centred at `(x, y, z)`.
fn frame_at(x: f64, y: f64, z: f64) -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(id("scope"), [x, y, z]).expect("valid point"),
        MetricDirection::try_new([1.0, 0.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 1.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 0.0, 1.0]).expect("valid direction"),
    )
    .expect("valid frame")
}

/// A walking profile: 0.3 m radius, 1.8 m tall, modest step and slope.
fn profile() -> MobilityProfile {
    MobilityProfile::try_new(0.3, 1.8, 0.15, 0.08).expect("valid profile")
}

fn box_shape(w: f64, d: f64, h: f64) -> ClearanceShape {
    ClearanceShape::Box(BoxClearance::try_new(w, d, h).expect("valid box"))
}

/// A volume with nothing in it is clear, and the claim is earned: every named
/// obstacle was measured.
#[test]
fn an_empty_volume_is_clear() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("column"), body(8.0, 9.0, 8.0, 9.0, 0.0, 3.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("column")],
    );
    assert!(matches!(
        service.assess_clearance(&request).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// A column inside the volume obstructs it and is named as the blocker.
#[test]
fn an_intersecting_body_obstructs_and_is_named() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("column"), body(0.9, 1.1, 0.9, 1.1, 0.0, 3.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("column")],
    );
    match service.assess_clearance(&request).expect("measurable") {
        ClearanceOutcome::Obstructed(evidence) => {
            assert_eq!(evidence.blockers(), &[id("column")]);
        }
        ClearanceOutcome::Clear(_) => panic!("a column in the volume obstructs it"),
    }
}

/// A body sharing plan area but sitting above the volume does not obstruct it.
#[test]
fn a_body_above_the_volume_does_not_obstruct_it() {
    let geometry = AxiolidGeometry::new().with_mesh(id("beam"), body(0.0, 2.0, 0.0, 2.0, 5.0, 5.4));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("beam")],
    );
    assert!(matches!(
        service.assess_clearance(&request).expect("measurable"),
        ClearanceOutcome::Clear(_)
    ));
}

/// Free floor is the scope minus what obstructs it.
#[test]
fn free_area_subtracts_obstacles_from_the_scope() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        .with_mesh(id("island"), body(0.0, 2.0, 0.0, 10.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("island")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    // 100 m2 of room, 20 m2 taken by the island.
    assert!(
        (evidence.available_area().upper_square_metres() - 80.0).abs() < 1e-6,
        "got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// Two overlapping obstacles do not subtract their shared area twice.
#[test]
fn overlapping_obstacles_are_not_double_counted() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        .with_mesh(id("crate-a"), body(0.0, 4.0, 0.0, 10.0, 0.0, 1.0))
        .with_mesh(id("crate-b"), body(2.0, 6.0, 0.0, 10.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("crate-a"), id("crate-b")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    // Union of the two crates spans x 0..6, so 60 m2 obstructed, 40 free.
    assert!(
        (evidence.available_area().upper_square_metres() - 40.0).abs() < 1e-6,
        "overlapping obstacles must union, got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// An obstacle extending beyond the room does not consume floor the room
/// never had.
#[test]
fn an_obstacle_is_clipped_to_the_scope() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1))
        // Twice the room's width, half its depth.
        .with_mesh(id("overhang"), body(-10.0, 20.0, 0.0, 5.0, 0.0, 1.0));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), vec![id("overhang")]);
    let evidence = service.measure_free_area(&request).expect("measurable");
    assert!(
        (evidence.available_area().upper_square_metres() - 50.0).abs() < 1e-6,
        "only the overlap inside the room counts, got {}",
        evidence.available_area().upper_square_metres()
    );
}

/// Placement refuses rather than guessing.
///
/// `NoPlacement` asserts an EXHAUSTIVE search found nowhere the shape fits. A
/// sampled sweep can only fail to find a witness, which is a weaker statement.
/// Returning `NoPlacement` from a sampled search would launder "did not find"
/// into "does not exist" -- the exact failure mode the outcome type exists to
/// prevent.
#[test]
fn placement_refuses_rather_than_claiming_an_exhaustive_search() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = PlacementRequest::new(id("room"), box_shape(0.8, 0.8, 2.0), Vec::new());
    assert!(
        matches!(
            service.find_placement(&request),
            Err(FreeSpaceError::Unavailable(_))
        ),
        "an unimplementable completeness claim must be refused, not faked"
    );
}

/// An obstacle without geometry cannot be cleared.
///
/// `Clear` asserts nothing obstructs the volume. An obstacle the adapter
/// cannot see has not been shown to be elsewhere, so reporting `Clear` would
/// assert more than was measured. The error names the object to look at.
#[test]
fn an_obstacle_without_geometry_is_not_silently_clear() {
    let service = AxiolidFreeSpaceService::new(AxiolidGeometry::new(), source());
    let request = ClearanceRequest::new(
        frame_at(1.0, 1.0, 0.0),
        box_shape(0.8, 0.8, 2.0),
        vec![id("unknown-column")],
    );
    match service.assess_clearance(&request) {
        Err(FreeSpaceError::MissingGeometry(object)) => {
            assert_eq!(*object, id("unknown-column"), "the error names the object");
        }
        other => panic!("expected MissingGeometry, got {other:?}"),
    }
}

/// Free area is reported as an upper bound, never as exact.
///
/// Plan area counts floor a mobility profile cannot occupy, such as a strip
/// too narrow to turn in. Claiming exactness would assert usable space that
/// was never measured.
#[test]
fn free_area_is_an_upper_bound_not_an_exact_value() {
    let geometry =
        AxiolidGeometry::new().with_mesh(id("room"), body(0.0, 10.0, 0.0, 10.0, 0.0, 0.1));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    let request = FreeAreaRequest::new(id("room"), profile(), Vec::new());
    let evidence = service.measure_free_area(&request).expect("measurable");
    let area = evidence.available_area();
    assert!(
        area.lower_square_metres() < area.upper_square_metres(),
        "an unrefined plan measurement is a bound, not a value"
    );
}
