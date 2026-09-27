//! Metric routes over real Axiolid geometry.
//!
//! Two rooms, `a` (x 0..4) and `b` (x 4.2..8), 3 m high, share a 0.2 m wall
//! with a doorway at y 1.0 up to the door's far jamb; the wall is split into
//! a south piece, a north piece and a lintel above 2.1 m.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidMetricRoutingService};
use axioval_engine::{
    MetricPoint, MetricRouteOutcome, MetricRouteRequest, MetricRoutingError,
    MetricRoutingServiceHandle, MobilityProfile,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let points = vec![
        Point3::new(x0, y0, z0),
        Point3::new(x1, y0, z0),
        Point3::new(x1, y1, z0),
        Point3::new(x0, y1, z0),
        Point3::new(x0, y0, z1),
        Point3::new(x1, y0, z1),
        Point3::new(x1, y1, z1),
        Point3::new(x0, y1, z1),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // floor, facing down
        4, 5, 6, 4, 6, 7, // ceiling, facing up
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn model(door_width: f64) -> AxiolidGeometry {
    let jamb = 1.0 + door_width;
    AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("b"), cuboid([4.2, 0.0, 0.0], [8.0, 4.0, 3.0]))
        .with_mesh(id("wall-s"), cuboid([4.0, 0.0, 0.0], [4.2, 1.0, 3.0]))
        .with_mesh(id("wall-n"), cuboid([4.0, jamb, 0.0], [4.2, 4.0, 3.0]))
        .with_mesh(id("lintel"), cuboid([4.0, 1.0, 2.1], [4.2, jamb, 3.0]))
        .with_mesh(id("door"), cuboid([4.05, 1.0, 0.0], [4.15, jamb, 2.1]))
        // A floor slab under both rooms and a ceiling slab above them.
        .with_mesh(id("floor"), cuboid([-0.2, -0.2, -0.3], [8.2, 4.2, 0.0]))
        .with_mesh(id("ceiling"), cuboid([-0.2, -0.2, 3.0], [8.2, 4.2, 3.3]))
}

fn service(geometry: AxiolidGeometry) -> AxiolidMetricRoutingService {
    AxiolidMetricRoutingService::new(geometry, source())
        .with_surface(id("a"))
        .with_surface(id("b"))
        .with_portal(id("door"))
}

fn request(radius: f64) -> MetricRouteRequest {
    MetricRouteRequest::new(
        MetricPoint::try_new(id("a"), [1.0, 3.0, 0.0]).unwrap(),
        MetricPoint::try_new(id("b"), [7.0, 3.0, 0.0]).unwrap(),
        MobilityProfile::try_new(radius, 2.0, 0.02, 0.06).unwrap(),
    )
}

fn route(
    service: AxiolidMetricRoutingService,
    request: &MetricRouteRequest,
) -> Result<MetricRouteOutcome, MetricRoutingError> {
    MetricRoutingServiceHandle::new(Arc::new(service)).route(request)
}

#[test]
fn a_route_between_two_rooms_through_a_door_is_found_and_bounded() {
    let outcome = route(
        service(model(0.9)).with_clear_width(id("door"), 0.85),
        &request(0.4),
    )
    .unwrap();
    let MetricRouteOutcome::Reachable(found) = outcome else {
        panic!("expected a route, got {outcome:?}");
    };
    let distance = found.shortest_distance();
    // The straight line is 6 m; the doorway forces a detour.
    assert!(distance.lower_metres() > 6.0, "{distance:?}");
    assert!(distance.upper_metres() >= distance.lower_metres());
    assert!(distance.upper_metres() < 8.0, "{distance:?}");
    assert!(!distance.is_exact());
    assert_eq!(found.traversed_objects(), &[id("a"), id("door"), id("b")]);
    assert!(found.evidence().exact);
    assert!(found.evidence().locator.contains("sweep=proven"));
    assert!(found.evidence().locator.contains("point-path="));
}

#[test]
fn a_door_narrower_than_the_body_blocks_the_route_with_complete_evidence() {
    let request = request(0.45);
    let outcome = route(service(model(0.7)), &request).unwrap();
    let MetricRouteOutcome::Blocked(blocked) = outcome else {
        panic!("expected blocked, got {outcome:?}");
    };
    assert_eq!(blocked.request(), &request);
    let locator = &blocked.completeness().evidence().locator;
    assert!(
        locator.contains("narrow-portals=[cad:model/door]"),
        "{locator}"
    );
}

#[test]
fn a_door_without_a_stated_clear_width_refuses_rather_than_passes() {
    let outcome = route(service(model(0.9)), &request(0.4));
    match outcome {
        Err(MetricRoutingError::Unavailable(reason)) => {
            assert!(reason.contains("clear width"), "{reason}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_point_route_is_tightly_bounded() {
    // A point passes a door whatever its leaf and lining.
    let outcome = route(service(model(0.9)), &request(0.0)).unwrap();
    let MetricRouteOutcome::Reachable(found) = outcome else {
        panic!("expected a route, got {outcome:?}");
    };
    let distance = found.shortest_distance();
    // Through the doorway's corners: 6.40 m.
    assert!(distance.lower_metres() <= 6.41, "{distance:?}");
    assert!(distance.upper_metres() >= 6.40, "{distance:?}");
    assert!(distance.upper_metres() < 6.45, "{distance:?}");
}

#[test]
fn a_room_split_by_an_obstacle_is_blocked() {
    // The partition runs into the walls, as modelled partitions do.
    let geometry = model(0.9).with_mesh(id("partition"), cuboid([6.0, -0.1, 0.0], [6.1, 4.1, 2.5]));
    let outcome = route(service(geometry), &request(0.4)).unwrap();
    assert!(
        matches!(outcome, MetricRouteOutcome::Blocked(_)),
        "{outcome:?}"
    );
}

#[test]
fn a_gap_narrower_than_the_body_inside_a_room_refuses_until_one_sided_erosion() {
    // A partition leaves a 0.5 m gap a 0.8 m body cannot pass; without
    // one-sided erosion the gap cannot be proven blocking.
    let geometry = model(0.9).with_mesh(id("partition"), cuboid([6.0, 0.5, 0.0], [6.1, 4.0, 2.5]));
    let outcome = route(
        service(geometry).with_clear_width(id("door"), 0.85),
        &request(0.4),
    );
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref reason)) if reason.contains("one-sided erosion")),
        "{outcome:?}"
    );
}

#[test]
fn a_vertical_connector_leaves_the_level_open() {
    let geometry = model(0.7).with_mesh(id("stair"), cuboid([0.5, 0.5, 0.0], [1.5, 2.5, 3.3]));
    let outcome = route(
        service(geometry).with_connector(id("stair")),
        &request(0.45),
    );
    // The stair could lead round the narrow door through another level.
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref reason)) if reason.contains("leaves the level")),
        "{outcome:?}"
    );
}

#[test]
fn an_unmeasured_body_refuses() {
    let geometry = model(0.9).with_unmeasured(id("pillar"), "no body representation");
    let outcome = route(
        service(geometry).with_clear_width(id("door"), 0.85),
        &request(0.4),
    );
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref reason)) if reason.contains("pillar")),
        "{outcome:?}"
    );
}
