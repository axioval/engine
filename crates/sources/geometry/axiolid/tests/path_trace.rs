//! Tracing a walk over objects, and walking around objects, over real
//! Axiolid geometry.
//!
//! Room `room` (x 0..4, y 0..4) opens east onto corridor `corridor`
//! (x 4..6, y 0..10), which leads north to lobby `lobby` (x 4..6, y 10..12),
//! where the exit stands at (5, 11). With `bypass`, a strip south of both
//! (x 0..8, y -2..0) and one east of the corridor (x 6..8, y 0..12) lead
//! round it. Surfaces sharing a boundary join, so no doors are needed.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidMetricRoutingService};
use axioval_engine::{
    MetricPoint, MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome,
    NearestTargetRequest, PathTraceRequest,
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

const SURFACES: [(&str, [f64; 2], [f64; 2]); 5] = [
    ("room", [0.0, 0.0], [4.0, 4.0]),
    ("corridor", [4.0, 0.0], [6.0, 10.0]),
    ("lobby", [4.0, 10.0], [6.0, 12.0]),
    ("south", [0.0, -2.0], [8.0, 0.0]),
    ("east", [6.0, 0.0], [8.0, 12.0]),
];

/// The surfaces (with or without the bypass), a stair `stair` standing
/// far east, and `far`, a box far west.
fn service(bypass: bool, stair: bool) -> MetricRoutingServiceHandle {
    let used = if bypass { 5 } else { 3 };
    let mut geometry = AxiolidGeometry::new()
        .with_mesh(id("far"), cuboid([-20.0, -20.0, 0.0], [-19.0, -19.0, 1.0]));
    for (name, [x0, y0], [x1, y1]) in &SURFACES[..used] {
        geometry = geometry.with_mesh(id(name), cuboid([*x0, *y0, 0.0], [*x1, *y1, 3.0]));
    }
    if stair {
        geometry = geometry.with_mesh(id("stair"), cuboid([6.5, 0.0, 0.0], [7.5, 4.0, 3.0]));
    }
    let mut service = AxiolidMetricRoutingService::new(geometry, source());
    for (name, _, _) in &SURFACES[..used] {
        service = service.with_surface(id(name));
    }
    if stair {
        service = service.with_connector(id("stair"));
    }
    MetricRoutingServiceHandle::new(Arc::new(service))
}

fn walk(
    service: &MetricRoutingServiceHandle,
    avoided: &[&str],
) -> Result<NearestTargetOutcome, axioval_engine::MetricRoutingError> {
    let request = NearestTargetRequest::try_new(
        MetricPoint::try_new(id("room"), [2.0, 2.0, 0.0]).unwrap(),
        vec![MetricPoint::try_new(id("exit"), [5.0, 11.0, 0.0]).unwrap()],
        MobilityProfile::try_new(0.0, 2.0, 0.02, 0.0).unwrap(),
    )
    .unwrap()
    .with_avoided(avoided.iter().map(|name| id(name)).collect());
    service.nearest_target(&request)
}

/// Round the room's corner (4, 4), then straight to the exit.
fn shortest() -> f64 {
    8.0_f64.sqrt() + 50.0_f64.sqrt()
}

#[test]
fn a_walk_is_traced_over_the_surfaces_it_crosses() {
    let service = service(false, false);
    let NearestTargetOutcome::Reached(reached) = walk(&service, &[]).unwrap() else {
        panic!("the exit is reachable");
    };
    let upper = reached.shortest_distance().upper_metres();
    assert!((upper - shortest()).abs() < 1e-6, "{reached:?}");
    let trace = service
        .trace_path(
            &PathTraceRequest::try_new(
                reached.waypoints().to_vec(),
                vec![
                    id("room"),
                    id("corridor"),
                    id("lobby"),
                    id("far"),
                    id("nothing"),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    assert!(trace.evidence().exact);
    // Sorted: corridor, far, lobby, nothing, room. The second leg is 6/7
    // in the corridor and 1/7 in the lobby.
    let second = 50.0_f64.sqrt();
    let expected = [
        Some(second * 6.0 / 7.0),
        Some(0.0),
        Some(second / 7.0),
        None,
        Some(8.0_f64.sqrt()),
    ];
    for (length, expected) in trace.lengths().iter().zip(expected) {
        match (length, expected) {
            (Ok(length), Some(metres)) => assert!(
                length.lower_metres() <= metres
                    && length.upper_metres() >= metres
                    && length.upper_metres() - length.lower_metres() < 1e-3,
                "{length:?} misses {metres}"
            ),
            (Err(reason), None) => {
                assert!(reason.contains("no described geometry"), "{reason}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    // A walk along the corridor's west side counts as over it only in the
    // upper bound.
    let along = service
        .trace_path(
            &PathTraceRequest::try_new(
                vec![
                    MetricPoint::try_new(id("corridor"), [4.0, 5.0, 0.0]).unwrap(),
                    MetricPoint::try_new(id("corridor"), [4.0, 9.0, 0.0]).unwrap(),
                ],
                vec![id("corridor")],
            )
            .unwrap(),
        )
        .unwrap();
    let length = along.lengths()[0].clone().unwrap();
    assert!(
        length.lower_metres() == 0.0 && length.upper_metres() >= 4.0,
        "{length:?}"
    );
}

#[test]
fn a_walk_around_the_only_way_out_is_unreachable() {
    let service = service(false, false);
    let outcome = walk(&service, &["corridor"]).unwrap();
    let NearestTargetOutcome::Unreachable(cut_off) = outcome else {
        panic!("expected no way round, got {outcome:?}");
    };
    assert_eq!(cut_off.request().avoided(), [id("corridor")]);
    assert!(
        cut_off
            .completeness()
            .evidence()
            .locator
            .contains(":avoided=[cad:model/corridor]:"),
        "{cut_off:?}"
    );
    // Avoiding what no walk comes near changes nothing.
    let NearestTargetOutcome::Reached(reached) = walk(&service, &["far"]).unwrap() else {
        panic!("the exit is reachable");
    };
    assert!((reached.shortest_distance().upper_metres() - shortest()).abs() < 1e-6);
}

#[test]
fn a_detour_round_the_corridor_is_longer_than_the_shortest_walk() {
    let service = service(true, false);
    let NearestTargetOutcome::Reached(plain) = walk(&service, &[]).unwrap() else {
        panic!("the exit is reachable");
    };
    let NearestTargetOutcome::Reached(detour) = walk(&service, &["corridor"]).unwrap() else {
        panic!("the exit is reachable round the corridor");
    };
    // South to (4, 0), east to (6, 0), north to (6, 10), then to the exit.
    let round = 8.0_f64.sqrt() + 2.0 + 10.0 + 2.0_f64.sqrt();
    let distance = detour.shortest_distance();
    assert!(
        distance.lower_metres() <= round && distance.upper_metres() >= round,
        "{distance:?} misses {round}"
    );
    assert!(distance.lower_metres() > plain.shortest_distance().upper_metres());
}

#[test]
fn an_open_level_bounds_the_nearest_target_only_by_a_straight_line() {
    // The stair leaves the level: a shortcut over another level cannot be
    // ruled out, so the lower bound is the straight line.
    let service = service(true, true);
    let NearestTargetOutcome::Reached(reached) = walk(&service, &[]).unwrap() else {
        panic!("the exit is reachable");
    };
    let straight = 90.0_f64.sqrt();
    let distance = reached.shortest_distance();
    assert!(
        (distance.lower_metres() - straight).abs() < 1e-9,
        "{distance:?}"
    );
    assert!(
        (distance.upper_metres() - shortest()).abs() < 1e-6,
        "{distance:?}"
    );
    assert!(
        reached.evidence().locator.contains(":lower=straight-line="),
        "{}",
        reached.evidence().locator
    );
}
