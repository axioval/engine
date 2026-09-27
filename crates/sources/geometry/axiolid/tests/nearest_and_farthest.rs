//! Nearest targets and farthest points over real Axiolid geometry.
//!
//! A hall (x 0..20, y 0..10, 3 m high) has a north wall (y 10..10.2) with
//! exit door `west` at x 1..2, leading outside. With `east`, the east wall
//! (x 20..20.2) has a second exit at y 1..2. Floor and ceiling slabs lie
//! under and over it.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidMetricRoutingService};
use axioval_engine::{
    FarthestPointOutcome, FarthestPointRequest, MetricPoint, MetricRoutingError,
    MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome, NearestTargetRequest,
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

fn hall(east: bool) -> AxiolidGeometry {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("hall"), cuboid([0.0, 0.0, 0.0], [20.0, 10.0, 3.0]))
        .with_mesh(id("wall-nw"), cuboid([0.0, 10.0, 0.0], [1.0, 10.2, 3.0]))
        .with_mesh(id("wall-ne"), cuboid([2.0, 10.0, 0.0], [20.2, 10.2, 3.0]))
        .with_mesh(id("lintel-w"), cuboid([1.0, 10.0, 2.1], [2.0, 10.2, 3.0]))
        .with_mesh(id("west"), cuboid([1.0, 10.05, 0.0], [2.0, 10.15, 2.1]))
        .with_mesh(id("floor"), cuboid([-0.2, -0.2, -0.3], [20.4, 10.4, 0.0]))
        .with_mesh(id("ceiling"), cuboid([-0.2, -0.2, 3.0], [20.4, 10.4, 3.3]));
    if east {
        geometry
            .with_mesh(id("wall-es"), cuboid([20.0, 0.0, 0.0], [20.2, 1.0, 3.0]))
            .with_mesh(id("wall-en"), cuboid([20.0, 2.0, 0.0], [20.2, 10.0, 3.0]))
            .with_mesh(id("lintel-e"), cuboid([20.0, 1.0, 2.1], [20.2, 2.0, 3.0]))
            .with_mesh(id("east"), cuboid([20.05, 1.0, 0.0], [20.15, 2.0, 2.1]))
    } else {
        geometry.with_mesh(id("wall-e"), cuboid([20.0, 0.0, 0.0], [20.2, 10.0, 3.0]))
    }
}

fn service(geometry: AxiolidGeometry, east: bool) -> MetricRoutingServiceHandle {
    let mut service = AxiolidMetricRoutingService::new(geometry, source())
        .with_surface(id("hall"))
        .with_portal(id("west"));
    if east {
        service = service.with_portal(id("east"));
    }
    MetricRoutingServiceHandle::new(Arc::new(service))
}

fn exit(name: &str) -> MetricPoint {
    let at = if name == "west" {
        [1.5, 10.1, 0.0]
    } else {
        [20.1, 1.5, 0.0]
    };
    MetricPoint::try_new(id(name), at).unwrap()
}

fn walking(radius: f64) -> MobilityProfile {
    MobilityProfile::try_new(radius, 2.0, 0.02, 0.0).unwrap()
}

fn farthest(
    service: &MetricRoutingServiceHandle,
    targets: Vec<MetricPoint>,
) -> Result<FarthestPointOutcome, MetricRoutingError> {
    service.farthest_point(
        &FarthestPointRequest::try_new(id("hall"), targets, walking(0.0), 0.01).unwrap(),
    )
}

#[test]
fn the_farthest_point_of_a_hall_from_its_only_exit_is_bracketed() {
    let outcome = farthest(&service(hall(false), false), vec![exit("west")]).unwrap();
    let FarthestPointOutcome::Bounded(bounded) = outcome else {
        panic!("expected a bracket, got {outcome:?}");
    };
    // The south-east corner sees the doorway: sqrt(18.5² + 10.1²).
    let expected = (18.5_f64.powi(2) + 10.1_f64.powi(2)).sqrt();
    let distance = bounded.distance();
    assert!(
        distance.lower_metres() <= expected && distance.upper_metres() >= expected,
        "{distance:?} misses {expected}"
    );
    assert!(bounded.converged(), "{bounded:?}");
    assert!(distance.upper_metres() - distance.lower_metres() <= 0.01);
    let [x, y, z] = bounded.witness().coordinates_metres();
    assert!(x > 19.9 && y < 0.1 && z == 0.0, "{bounded:?}");
    assert_eq!(bounded.witness().subject(), &id("hall"));
    assert!(bounded.evidence().exact);
    assert!(
        bounded.evidence().locator.contains("bracket="),
        "{}",
        bounded.evidence().locator
    );
}

#[test]
fn a_second_exit_shortens_the_farthest_distance() {
    let outcome = farthest(&service(hall(true), true), vec![exit("west"), exit("east")]).unwrap();
    let FarthestPointOutcome::Bounded(bounded) = outcome else {
        panic!("expected a bracket, got {outcome:?}");
    };
    // With the east exit, no point lies more than about 12 m from an exit.
    assert!(bounded.distance().upper_metres() < 13.0, "{bounded:?}");
    assert!(bounded.distance().lower_metres() > 9.0, "{bounded:?}");
}

#[test]
fn part_of_a_hall_cut_off_from_every_exit_is_unreachable_with_a_witness() {
    // A partition runs wall to wall at x 10.
    let geometry = hall(false).with_mesh(
        id("partition"),
        cuboid([10.0, -0.1, 0.0], [10.1, 10.1, 3.0]),
    );
    let outcome = farthest(&service(geometry, false), vec![exit("west")]).unwrap();
    let FarthestPointOutcome::Unreachable(cut_off) = outcome else {
        panic!("expected an unreachable part, got {outcome:?}");
    };
    let [x, _, _] = cut_off.witness().coordinates_metres();
    assert!(x > 10.1, "{cut_off:?}");
    assert!(cut_off.completeness().evidence().exact);
}

#[test]
fn an_unclosed_level_keeps_the_upper_bound_and_falls_back_to_a_straight_line() {
    // A stair touching the hall could lead to a shortcut through another
    // level, so the bracket's lower bound no longer holds.
    let geometry = hall(false).with_mesh(id("stair"), cuboid([5.0, 4.0, 0.0], [6.0, 6.0, 3.3]));
    let service = MetricRoutingServiceHandle::new(Arc::new(
        AxiolidMetricRoutingService::new(geometry, source())
            .with_surface(id("hall"))
            .with_portal(id("west"))
            .with_connector(id("stair")),
    ));
    let outcome = farthest(&service, vec![exit("west")]).unwrap();
    let FarthestPointOutcome::Bounded(bounded) = outcome else {
        panic!("expected a bracket, got {outcome:?}");
    };
    assert!(
        bounded.evidence().locator.contains("straight-line="),
        "{bounded:?}"
    );
    assert!(bounded.distance().upper_metres() >= 21.0, "{bounded:?}");
}

#[test]
fn the_farthest_point_of_a_body_with_a_radius_is_refused() {
    let service = service(hall(false), false);
    let outcome = service.farthest_point(
        &FarthestPointRequest::try_new(id("hall"), vec![exit("west")], walking(0.3), 0.01).unwrap(),
    );
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref reason)) if reason.contains("one-sided erosion")),
        "{outcome:?}"
    );
}

#[test]
fn a_region_that_is_no_walkable_surface_is_refused() {
    let service = service(hall(false), false);
    let outcome = service.farthest_point(
        &FarthestPointRequest::try_new(id("wall-e"), vec![exit("west")], walking(0.0), 0.01)
            .unwrap(),
    );
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref reason)) if reason.contains("not a declared walkable surface")),
        "{outcome:?}"
    );
}

#[test]
fn the_nearest_of_two_exits_is_found_for_a_point_and_for_a_body() {
    let service = service(hall(true), true);
    let origin = MetricPoint::try_new(id("hall"), [18.0, 3.0, 0.0]).unwrap();
    for radius in [0.0, 0.3] {
        let request = NearestTargetRequest::try_new(
            origin.clone(),
            vec![exit("west"), exit("east")],
            walking(radius),
        )
        .unwrap();
        let outcome = service.nearest_target(&request);
        let outcome = match (radius, outcome) {
            (_, Ok(outcome)) => outcome,
            // A door's leaf and lining admit a body only by a stated width.
            (r, Err(MetricRoutingError::Unavailable(reason))) if r > 0.0 => {
                assert!(reason.contains("no route wide enough"), "{reason}");
                continue;
            }
            (_, Err(error)) => panic!("{error}"),
        };
        let NearestTargetOutcome::Reached(reached) = outcome else {
            panic!("expected a target, got {outcome:?}");
        };
        assert_eq!(reached.target(), 1, "{reached:?}");
        let straight = (2.1_f64.powi(2) + 1.5_f64.powi(2)).sqrt();
        let distance = reached.shortest_distance();
        assert!(distance.lower_metres() >= straight - 1e-9, "{distance:?}");
        assert!(distance.upper_metres() < straight + 0.1, "{distance:?}");
    }
}

#[test]
fn a_body_reaches_the_nearest_landing_by_a_proven_sweep() {
    // A body cannot stand in a doorway that leads outside, so the targets
    // here are the landings in front of the exits.
    let service = service(hall(true), true);
    let landings = vec![
        MetricPoint::try_new(id("west"), [1.5, 9.6, 0.0]).unwrap(),
        MetricPoint::try_new(id("east"), [19.6, 1.5, 0.0]).unwrap(),
    ];
    let origin = MetricPoint::try_new(id("hall"), [15.0, 5.0, 0.0]).unwrap();
    let request = NearestTargetRequest::try_new(origin, landings, walking(0.3)).unwrap();
    let NearestTargetOutcome::Reached(reached) = service.nearest_target(&request).unwrap() else {
        panic!("expected a target");
    };
    assert_eq!(reached.target(), 1);
    assert!(reached.evidence().locator.contains("sweep=proven"));
    let straight = (4.6_f64.powi(2) + 3.5_f64.powi(2)).sqrt();
    let distance = reached.shortest_distance();
    assert!(distance.lower_metres() >= straight - 1e-9, "{distance:?}");
    assert!(distance.upper_metres() < straight + 1e-6, "{distance:?}");
}

#[test]
fn exits_behind_a_partition_are_unreachable() {
    let geometry = hall(false).with_mesh(
        id("partition"),
        cuboid([10.0, -0.1, 0.0], [10.1, 10.1, 3.0]),
    );
    let service = service(geometry, false);
    let request = NearestTargetRequest::try_new(
        MetricPoint::try_new(id("hall"), [15.0, 5.0, 0.0]).unwrap(),
        vec![exit("west")],
        walking(0.0),
    )
    .unwrap();
    let outcome = service.nearest_target(&request).unwrap();
    assert!(
        matches!(outcome, NearestTargetOutcome::Unreachable(_)),
        "{outcome:?}"
    );
}

#[test]
fn an_unplaced_target_bounds_only_from_below() {
    // A target on no surface counts by its straight line; the placed one
    // bounds the distance from above.
    let service = service(hall(false), false);
    let nowhere = MetricPoint::try_new(id("nowhere"), [40.0, 5.0, 0.0]).unwrap();
    let request = NearestTargetRequest::try_new(
        MetricPoint::try_new(id("hall"), [19.0, 5.0, 0.0]).unwrap(),
        vec![exit("west"), nowhere],
        walking(0.0),
    )
    .unwrap();
    let NearestTargetOutcome::Reached(reached) = service.nearest_target(&request).unwrap() else {
        panic!("expected a target");
    };
    assert_eq!(reached.target(), 0);
    let distance = reached.shortest_distance();
    // The unplaced target lies 21 m away in a straight line, the exit about
    // 18.2 m: the nearest stays below the exit's route.
    assert!(distance.upper_metres() > 17.0, "{distance:?}");
    assert!(distance.lower_metres() <= distance.upper_metres());
}
