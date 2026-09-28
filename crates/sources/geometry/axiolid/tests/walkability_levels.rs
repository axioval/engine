//! Walkable passages between levels through stairs, ramps and lifts.
//!
//! `a` is a ground-floor room (x 0..10, y 0..4, 2.7 m high) and `c` the room
//! above it (floor at 3 m). A straight stair of twelve 0.25 m risers and
//! 0.3 m goings, 1.2 m wide, climbs along +x from x = 2 to 5.6 against the
//! south wall (y 0..1.2). Beside them, lobbies `l0` and `l1` (x 0..4) open
//! onto lift cars `car0` and `car1` (x 4..6, y 1..3) inside the shaft
//! `lift`.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidWalkabilityService};
use axioval_engine::{
    VerticalConnector, VerticalConnectorKind, WalkabilityRequest, WalkabilityRouteOutcome,
    WalkabilityServiceHandle, WalkabilitySnapshot,
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

/// The stair: a side profile swept from y 0 to 1.2, its winding facing -y.
fn stair() -> TriMesh {
    let (steps, rise, going, start) = (12_u32, 0.25, 0.3, 2.0);
    let length = going * f64::from(steps);
    let mut profile = vec![[start, 0.0], [start + length, 0.0], [start + length, 3.0]];
    let mut elevation = 3.0;
    for step in (0..steps).rev() {
        let front = start + going * f64::from(step);
        profile.push([front, elevation]);
        elevation -= rise;
        if step > 0 {
            profile.push([front, elevation]);
        }
    }
    let n = profile.len();
    let mut points: Vec<Point3> = profile
        .iter()
        .map(|p| Point3::new(p[0], 0.0, p[1]))
        .collect();
    points.extend(profile.iter().map(|p| Point3::new(p[0], 1.2, p[1])));
    let mut indices: Vec<u32> = Vec::new();
    // The profile is a staircase over a flat base: fan each step's
    // rectangle and the base triangle-free by ear clipping.
    for [a, b, c] in ears(&profile) {
        indices.extend([a, b, c].map(|i| u32::try_from(i).unwrap()));
        indices.extend([a + n, c + n, b + n].map(|i| u32::try_from(i).unwrap()));
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j + n, j, i, i + n, j + n].map(|k| u32::try_from(k).unwrap()));
    }
    TriMesh::new(points, indices)
}

/// Ear-clipping triangulation of a simple counter-clockwise polygon.
fn ears(polygon: &[[f64; 2]]) -> Vec<[usize; 3]> {
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut remaining: Vec<usize> = (0..polygon.len()).collect();
    let mut found = Vec::new();
    while remaining.len() > 3 {
        let count = remaining.len();
        let ear = (0..count)
            .find(|&i| {
                let (p, c, n) = (
                    remaining[(i + count - 1) % count],
                    remaining[i],
                    remaining[(i + 1) % count],
                );
                if cross(polygon[p], polygon[c], polygon[n]) <= 0.0 {
                    return false;
                }
                remaining.iter().all(|&other| {
                    other == p
                        || other == c
                        || other == n
                        || cross(polygon[p], polygon[c], polygon[other]) < 0.0
                        || cross(polygon[c], polygon[n], polygon[other]) < 0.0
                        || cross(polygon[n], polygon[p], polygon[other]) < 0.0
                })
            })
            .expect("a simple polygon has an ear");
        found.push([
            remaining[(ear + count - 1) % count],
            remaining[ear],
            remaining[(ear + 1) % count],
        ]);
        remaining.remove(ear);
    }
    found.push([remaining[0], remaining[1], remaining[2]]);
    found
}

fn snapshot(geometry: AxiolidGeometry, request: &WalkabilityRequest) -> WalkabilitySnapshot {
    WalkabilityServiceHandle::new(Arc::new(AxiolidWalkabilityService::new(geometry, source())))
        .snapshot(request)
        .unwrap()
}

fn stairs(width: f64, obstacles: Vec<ObjectId>) -> WalkabilitySnapshot {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 2.7]))
        .with_mesh(id("c"), cuboid([0.0, 0.0, 3.0], [10.0, 4.0, 5.7]))
        .with_mesh(id("stair"), stair())
        .with_mesh(id("beam"), cuboid([2.0, 0.0, 1.4], [2.6, 1.2, 1.6]));
    let request = WalkabilityRequest::try_new(
        vec![id("a"), id("c")],
        Vec::new(),
        obstacles,
        width,
        None,
        true,
        false,
    )
    .unwrap()
    .with_connectors(vec![VerticalConnector::new(
        id("stair"),
        VerticalConnectorKind::Stair,
    )])
    .unwrap();
    snapshot(geometry, &request)
}

#[test]
fn a_measured_stair_wide_enough_is_a_definite_climb() {
    let snapshot = stairs(0.8, Vec::new());
    let climb = snapshot
        .passages()
        .iter()
        .find(|passage| passage.connector().is_some())
        .unwrap();
    // The flight's own width bounds the climb both ways.
    let width = climb.clear_width();
    assert!(
        (width.lower_metres() - 1.2).abs() < 1e-9 && (width.upper_metres() - 1.2).abs() < 1e-9,
        "{climb:?}"
    );
    assert!(
        climb.evidence().locator.contains("climb=proven"),
        "{climb:?}"
    );
    assert!(matches!(
        snapshot.route_between(&id("a"), &id("c")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    // A body wider than the flight cannot climb it.
    assert_eq!(
        stairs(1.4, Vec::new())
            .route_between(&id("a"), &id("c"))
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

#[test]
fn a_beam_over_the_flight_blocks_the_climb_it_stands_in() {
    // 0.9 m above the second tread: far below the room's height.
    let snapshot = stairs(0.8, vec![id("beam")]);
    let climb = snapshot
        .passages()
        .iter()
        .find(|passage| passage.connector().is_some())
        .unwrap();
    assert!(
        climb.evidence().locator.contains("climb=refused"),
        "{climb:?}"
    );
    assert_eq!(
        snapshot.route_between(&id("a"), &id("c")).unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

fn lifts() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("l0"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 2.7]))
        .with_mesh(id("car0"), cuboid([4.0, 1.0, 0.0], [6.0, 3.0, 2.7]))
        .with_mesh(id("l1"), cuboid([0.0, 0.0, 3.0], [4.0, 4.0, 5.7]))
        .with_mesh(id("car1"), cuboid([4.0, 1.0, 3.0], [6.0, 3.0, 5.7]))
        .with_mesh(id("lift"), cuboid([4.0, 1.0, 0.0], [6.0, 3.0, 5.7]))
}

fn lift_request(kind: VerticalConnectorKind) -> WalkabilityRequest {
    WalkabilityRequest::try_new(
        vec![id("l0"), id("car0"), id("l1"), id("car1")],
        Vec::new(),
        Vec::new(),
        0.8,
        None,
        true,
        false,
    )
    .unwrap()
    .with_connectors(vec![VerticalConnector::new(id("lift"), kind)])
    .unwrap()
}

#[test]
fn a_lift_carries_a_body_between_the_cars_it_holds() {
    let snapshot = snapshot(lifts(), &lift_request(VerticalConnectorKind::Lift));
    let ride = snapshot
        .passages()
        .iter()
        .find(|passage| {
            passage.connector().is_some()
                && passage.evidence().locator.contains("car0<->")
                && passage.evidence().locator.contains("car1")
        })
        .unwrap();
    assert!(ride.evidence().locator.contains("ride=proven"), "{ride:?}");
    assert!(ride.clear_width().lower_metres() >= 0.8);
    assert!(matches!(
        snapshot.route_between(&id("l0"), &id("l1")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    // A lobby's hub is outside the car: no ride from it is proven.
    assert!(
        snapshot
            .passages()
            .iter()
            .filter(|passage| passage.connector().is_some())
            .filter(|passage| passage.evidence().locator.contains("ride=proven"))
            .count()
            == 1
    );
    assert_eq!(
        snapshot
            .route_between_avoiding(&id("l0"), &id("l1"), &[VerticalConnectorKind::Lift])
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}
