//! Weighted travel and walks forced through an object, over real Axiolid
//! geometry.
//!
//! A hall (x 0..20, y 0..10, 3 m high) has one exit door `west` at x 1..2
//! in its north wall, as in `nearest_and_farthest.rs`. A slab `band`
//! under its floor covers the south strip (y 0..4) wall to wall: travel
//! over it counts by a factor.
//!
//! The forced walks use the surfaces of `path_trace.rs`: room `room`
//! (x 0..4, y 0..4) opens east onto corridor `corridor` (x 4..6, y 0..10)
//! and lobby `lobby` (x 4..6, y 10..12), where the exit stands at (5, 11);
//! strips `south` (x 0..8, y -2..0) and `east` (x 6..8, y 0..12) lead
//! round them.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidMetricRoutingService};
use axioval_engine::{
    FarthestPointOutcome, FarthestPointRequest, ForcedWalkOutcome, ForcedWalkRequest, MetricPoint,
    MetricRoutingError, MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome,
    NearestTargetRequest, TravelCost,
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

fn hall() -> MetricRoutingServiceHandle {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("hall"), cuboid([0.0, 0.0, 0.0], [20.0, 10.0, 3.0]))
        .with_mesh(id("wall-nw"), cuboid([0.0, 10.0, 0.0], [1.0, 10.2, 3.0]))
        .with_mesh(id("wall-ne"), cuboid([2.0, 10.0, 0.0], [20.2, 10.2, 3.0]))
        .with_mesh(id("lintel-w"), cuboid([1.0, 10.0, 2.1], [2.0, 10.2, 3.0]))
        .with_mesh(id("west"), cuboid([1.0, 10.05, 0.0], [2.0, 10.15, 2.1]))
        .with_mesh(id("wall-e"), cuboid([20.0, 0.0, 0.0], [20.2, 10.0, 3.0]))
        .with_mesh(id("floor"), cuboid([-0.2, -0.2, -0.3], [20.4, 10.4, 0.0]))
        .with_mesh(id("ceiling"), cuboid([-0.2, -0.2, 3.0], [20.4, 10.4, 3.3]))
        .with_mesh(id("band"), cuboid([0.0, 0.0, -0.25], [20.0, 4.0, -0.05]));
    MetricRoutingServiceHandle::new(Arc::new(
        AxiolidMetricRoutingService::new(geometry, source())
            .with_surface(id("hall"))
            .with_portal(id("west")),
    ))
}

fn exit() -> MetricPoint {
    MetricPoint::try_new(id("west"), [1.5, 10.1, 0.0]).unwrap()
}

fn walking() -> MobilityProfile {
    MobilityProfile::try_new(0.0, 2.0, 0.02, 0.0).unwrap()
}

fn band(factor: f64) -> Vec<TravelCost> {
    vec![TravelCost::try_new(id("band"), factor).unwrap()]
}

#[test]
fn a_walk_across_a_costed_band_counts_its_metres_there_by_the_factor() {
    let service = hall();
    let from = MetricPoint::try_new(id("hall"), [19.0, 1.0, 0.0]).unwrap();
    let request = NearestTargetRequest::try_new(from, vec![exit()], walking()).unwrap();
    let NearestTargetOutcome::Reached(plain) = service.nearest_target(&request).unwrap() else {
        panic!("the exit is reachable");
    };
    let straight = 17.5_f64.hypot(9.1);
    assert!((plain.shortest_distance().upper_metres() - straight).abs() < 1e-6);
    let NearestTargetOutcome::Reached(weighted) = service
        .nearest_target(&request.clone().with_costs(band(2.0)))
        .unwrap()
    else {
        panic!("the exit is reachable");
    };
    let distance = weighted.shortest_distance();
    // Leaving the band from y = 1 walks at least 3 m in it, each counting
    // twice; walking straight north out of it costs 6 m, then straight.
    let north = 6.0 + 17.5_f64.hypot(6.1);
    assert!(
        distance.lower_metres() >= straight + 3.0 - 1e-6,
        "{distance:?}"
    );
    assert!(distance.upper_metres() <= north + 1e-6, "{distance:?}");
    assert!(
        distance.upper_metres() - distance.lower_metres() < 0.2,
        "{distance:?}"
    );
    assert_eq!(weighted.waypoints().last(), Some(&exit()));
    assert!(
        weighted.evidence().locator.contains("costs=[")
            && weighted.evidence().locator.contains("spacing="),
        "{}",
        weighted.evidence().locator
    );
}

#[test]
fn the_weighted_farthest_point_exceeds_the_plain_one_by_the_costed_band() {
    let service = hall();
    let request = FarthestPointRequest::try_new(id("hall"), vec![exit()], walking(), 0.01).unwrap();
    let FarthestPointOutcome::Bounded(plain) = service.farthest_point(&request).unwrap() else {
        panic!("expected a bracket");
    };
    let corner = 18.5_f64.hypot(10.1);
    assert!(plain.distance().upper_metres() < corner + 0.011);
    let FarthestPointOutcome::Bounded(weighted) = service
        .farthest_point(&request.with_costs(band(2.0)))
        .unwrap()
    else {
        panic!("expected a bracket");
    };
    let distance = weighted.distance();
    // From the south-east corner a walk leaves the band after at least
    // 4 m in it; walking straight north out of it costs 8 m, then
    // straight to the door.
    assert!(
        distance.lower_metres() >= corner + 4.0 - 0.2,
        "{distance:?}"
    );
    assert!(
        distance.upper_metres() <= 8.0 + 18.5_f64.hypot(6.1) + 0.2,
        "{distance:?}"
    );
    assert!(weighted.evidence().locator.contains("spacing="));
}

#[test]
fn a_tessellated_costed_object_is_refused() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("hall"), cuboid([0.0, 0.0, 0.0], [20.0, 10.0, 3.0]))
        .with_tessellated_mesh(
            id("band"),
            cuboid([0.0, 0.0, -0.25], [20.0, 4.0, -0.05]),
            0.01,
        );
    let service = MetricRoutingServiceHandle::new(Arc::new(
        AxiolidMetricRoutingService::new(geometry, source()).with_surface(id("hall")),
    ));
    let target = MetricPoint::try_new(id("hall"), [1.0, 9.0, 0.0]).unwrap();
    let request = NearestTargetRequest::try_new(
        MetricPoint::try_new(id("hall"), [19.0, 1.0, 0.0]).unwrap(),
        vec![target],
        walking(),
    )
    .unwrap()
    .with_costs(band(2.0));
    let outcome = service.nearest_target(&request);
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref why)) if why.contains("tessellated")),
        "{outcome:?}"
    );
}

const SURFACES: [(&str, [f64; 2], [f64; 2]); 5] = [
    ("room", [0.0, 0.0], [4.0, 4.0]),
    ("corridor", [4.0, 0.0], [6.0, 10.0]),
    ("lobby", [4.0, 10.0], [6.0, 12.0]),
    ("south", [0.0, -2.0], [8.0, 0.0]),
    ("east", [6.0, 0.0], [8.0, 12.0]),
];

fn rooms(open: bool) -> MetricRoutingServiceHandle {
    let mut geometry = AxiolidGeometry::new();
    for (name, [x0, y0], [x1, y1]) in &SURFACES {
        geometry = geometry.with_mesh(id(name), cuboid([*x0, *y0, 0.0], [*x1, *y1, 3.0]));
    }
    let mut service = AxiolidMetricRoutingService::new(geometry, source());
    for (name, _, _) in &SURFACES {
        service = service.with_surface(id(name));
    }
    if open {
        // A declared surface that cannot be measured leaves the level
        // unclosed.
        service = service.with_surface(id("unmeasured"));
    }
    MetricRoutingServiceHandle::new(Arc::new(service))
}

fn forced(
    service: &MetricRoutingServiceHandle,
    through: &str,
) -> Result<ForcedWalkOutcome, MetricRoutingError> {
    service.forced_walk(
        &ForcedWalkRequest::try_new(
            MetricPoint::try_new(id("room"), [2.0, 2.0, 0.0]).unwrap(),
            vec![MetricPoint::try_new(id("exit"), [5.0, 11.0, 0.0]).unwrap()],
            id(through),
            walking(),
            0.01,
        )
        .unwrap(),
    )
}

#[test]
fn a_strip_off_every_shortest_walk_is_proven_by_the_walk_forced_through_it() {
    let service = rooms(false);
    // Round the room's corner (4, 4), then straight to the exit.
    let shortest = 8.0_f64.sqrt() + 50.0_f64.sqrt();
    // Touching the east strip at x = 6 on the way: the exit mirrored in
    // that line, seen from the corner.
    let east = 8.0_f64.sqrt() + 58.0_f64.sqrt();
    let Ok(ForcedWalkOutcome::Bounded(bounded)) = forced(&service, "east") else {
        panic!("the east strip is reached");
    };
    assert!(
        bounded.lower_metres() <= east && bounded.upper_metres() >= east,
        "{bounded:?} misses {east}"
    );
    assert!(bounded.lower_metres() > shortest + 0.5, "{bounded:?}");
    assert!(bounded.converged(), "{bounded:?}");
    assert!(
        bounded
            .evidence()
            .locator
            .starts_with("axiolid:metric-route:forced:"),
        "{}",
        bounded.evidence().locator
    );
    // The corridor lies on the shortest walk: the bracket holds it.
    let Ok(ForcedWalkOutcome::Bounded(corridor)) = forced(&service, "corridor") else {
        panic!("the corridor is reached");
    };
    assert!(corridor.lower_metres() <= shortest + 1e-6, "{corridor:?}");
    assert!(corridor.upper_metres() >= shortest - 1e-6, "{corridor:?}");
}

#[test]
fn a_forced_walk_on_a_level_that_is_not_closed_is_refused() {
    let outcome = forced(&rooms(true), "east");
    assert!(
        matches!(outcome, Err(MetricRoutingError::Unavailable(ref why)) if why.contains("closed level")),
        "{outcome:?}"
    );
}
