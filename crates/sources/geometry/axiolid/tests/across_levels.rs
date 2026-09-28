//! Metric routes across levels through a request's vertical connectors.
//!
//! A ground-floor room (x 0..10, y 0..4, 2.7 m high) holds a straight stair
//! of twelve 0.25 m risers, 0.3 m goings and 1.2 m width climbing along +x
//! from x = 2 (y 1..2.2) to x = 5.6 and z = 3, where an upper room (same
//! plan, floor at 3 m) begins. The stair obstructs the ground room; the
//! upper room stands on its top.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidMetricRoutingService};
use axioval_engine::{
    ClimbLength, ConnectorRouting, FarthestPointOutcome, FarthestPointRequest, MetricPoint,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingError, MetricRoutingServiceHandle,
    MobilityProfile, NearestTargetEvidence, NearestTargetOutcome, NearestTargetRequest,
    StairLength, TravelCost, VerticalConnector, VerticalConnectorKind,
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

/// A closed, outward-facing prism: a side `profile` (counter-clockwise
/// `(x, z)` points, convex pieces fanned from its first point being enough
/// for the shapes here) swept from `y0` to `y1`.
fn prism(profile: &[[f64; 2]], y0: f64, y1: f64) -> TriMesh {
    let n = profile.len();
    let mut points: Vec<Point3> = profile
        .iter()
        .map(|p| Point3::new(p[0], y0, p[1]))
        .collect();
    points.extend(profile.iter().map(|p| Point3::new(p[0], y1, p[1])));
    let mut indices: Vec<u32> = Vec::new();
    for [a, b, c] in ears(profile) {
        // Seen from y0 (outside, looking along +y) the profile runs
        // clockwise, so its own winding faces -y.
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

/// The stair's side profile: from x = 2, twelve 0.25 m risers and 0.3 m
/// goings, the last tread its top at z = 3.
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
    prism(&profile, 1.0, 2.2)
}

fn building() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("ground"), cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 2.7]))
        .with_mesh(id("upper"), cuboid([0.0, 0.0, 3.0], [10.0, 4.0, 5.7]))
        .with_mesh(id("stair"), stair())
}

fn service(geometry: AxiolidGeometry) -> MetricRoutingServiceHandle {
    MetricRoutingServiceHandle::new(Arc::new(
        AxiolidMetricRoutingService::new(geometry, source())
            .with_surface(id("ground"))
            .with_surface(id("upper")),
    ))
}

fn walking(radius: f64) -> MobilityProfile {
    MobilityProfile::try_new(radius, 2.0, 0.02, 0.0).unwrap()
}

fn routing(kind: VerticalConnectorKind, climb: ClimbLength) -> ConnectorRouting {
    ConnectorRouting::try_new(vec![VerticalConnector::new(id("stair"), kind)], climb).unwrap()
}

fn stairs(climb: ClimbLength) -> ConnectorRouting {
    routing(VerticalConnectorKind::Stair, climb)
}

fn upstairs() -> MetricPoint {
    MetricPoint::try_new(id("upper"), [0.5, 0.5, 3.0]).unwrap()
}

fn exit() -> MetricPoint {
    MetricPoint::try_new(id("ground"), [9.5, 3.5, 0.0]).unwrap()
}

/// The landings stand 1 mm outside the walking line's ends, on its
/// centre line y = 1.6.
const LOWER: [f64; 2] = [1.999, 1.6];
const UPPER: [f64; 2] = [5.601, 1.6];

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The ground walk from the lower landing to the exit, round the stair's
/// north-west corner.
fn downstairs() -> f64 {
    distance(LOWER, [2.0, 2.2]) + distance([2.0, 2.2], [9.5, 3.5])
}

fn nearest(
    service: &MetricRoutingServiceHandle,
    connectors: ConnectorRouting,
    radius: f64,
) -> Result<NearestTargetOutcome, MetricRoutingError> {
    service.nearest_target(
        &NearestTargetRequest::try_new(upstairs(), vec![exit()], walking(radius))
            .unwrap()
            .with_connectors(connectors),
    )
}

/// How far the overlay's grid snapping (axiolid/kernel#173) may move a
/// corner of the free region, and so a walk round it.
const SNAP: f64 = 1e-7;

fn holds(lower: f64, upper: f64, value: f64) {
    assert!(
        lower <= value + SNAP && value - SNAP <= upper,
        "[{lower}, {upper}] misses {value}"
    );
    assert!(upper - lower < 1e-6, "[{lower}, {upper}] is loose");
}

#[test]
fn a_walk_down_the_stair_counts_its_slope_or_its_weighted_rise() {
    let service = service(building());
    let horizontal = UPPER[0] - LOWER[0];
    let before = distance([0.5, 0.5], UPPER);
    for (climb, counted) in [
        (ClimbLength::slope(), horizontal.hypot(3.0)),
        (
            ClimbLength::try_new(StairLength::HorizontalPlusVertical, 2.0).unwrap(),
            horizontal + 6.0,
        ),
    ] {
        let outcome = nearest(&service, stairs(climb), 0.0).unwrap();
        let NearestTargetOutcome::Reached(reached) = outcome else {
            panic!("expected a walk, got {outcome:?}");
        };
        let walked = reached.shortest_distance();
        holds(
            walked.lower_metres(),
            walked.upper_metres(),
            before + counted + downstairs(),
        );
        // The walk lands on the stair at both ends.
        let on_stair: Vec<[f64; 3]> = reached
            .waypoints()
            .iter()
            .filter(|point| point.subject() == &id("stair"))
            .map(MetricPoint::coordinates_metres)
            .collect();
        assert_eq!(on_stair.len(), 2, "{reached:?}");
        assert!((on_stair[0][2] - 3.0).abs() < 1e-12 && on_stair[1][2].abs() < 1e-12);
        assert!(reached.evidence().locator.contains("climbs=[stair"));
    }
}

#[test]
fn the_farthest_point_upstairs_walks_down_the_stair() {
    let service = service(building());
    let request = FarthestPointRequest::try_new(id("upper"), vec![exit()], walking(0.0), 0.01)
        .unwrap()
        .with_connectors(stairs(ClimbLength::slope()));
    let outcome = service.farthest_point(&request).unwrap();
    let FarthestPointOutcome::Bounded(bounded) = outcome else {
        panic!("expected a bracket, got {outcome:?}");
    };
    // The north-west corner is farthest from the stair's head.
    let beyond = (UPPER[0] - LOWER[0]).hypot(3.0) + downstairs();
    let expected = distance([0.0, 4.0], UPPER) + beyond;
    let bracket = bounded.distance();
    assert!(
        bracket.lower_metres() <= expected + SNAP && expected <= bracket.upper_metres() + SNAP,
        "{bracket:?} misses {expected}"
    );
    assert!(bounded.converged(), "{bounded:?}");
    let [x, y, z] = bounded.witness().coordinates_metres();
    assert!(x < 0.1 && y > 3.9 && (z - 3.0).abs() < 1e-12, "{bounded:?}");
}

#[test]
fn a_route_across_levels_names_the_stair_and_no_connector_is_blocked() {
    let service = service(building());
    let request = MetricRouteRequest::new(upstairs(), exit(), walking(0.0));
    let outcome = service
        .route(
            &request
                .clone()
                .with_connectors(stairs(ClimbLength::slope())),
        )
        .unwrap();
    let MetricRouteOutcome::Reachable(route) = outcome else {
        panic!("expected a route, got {outcome:?}");
    };
    assert_eq!(
        route.traversed_objects(),
        [id("upper"), id("stair"), id("ground")]
    );
    // With no connector to climb, the levels are closed and apart.
    let closed = ConnectorRouting::try_new(Vec::new(), ClimbLength::slope()).unwrap();
    assert!(matches!(
        service.route(&request.with_connectors(closed.clone())),
        Ok(MetricRouteOutcome::Blocked(_))
    ));
    assert!(matches!(
        nearest(&service, closed, 0.0),
        Ok(NearestTargetOutcome::Unreachable(_))
    ));
}

#[test]
fn a_body_wider_than_the_stair_or_taller_than_its_headroom_cannot_climb_it() {
    // 1.4 m across a 1.2 m flight: the stair is no way, and nothing else
    // joins the levels.
    assert!(matches!(
        nearest(&service(building()), stairs(ClimbLength::slope()), 0.7),
        Ok(NearestTargetOutcome::Unreachable(_))
    ));
    // A beam 0.9 m above the lowest treads.
    let beamed = building().with_mesh(id("beam"), cuboid([2.0, 0.5, 1.4], [2.6, 2.7, 1.6]));
    assert!(matches!(
        nearest(&service(beamed), stairs(ClimbLength::slope()), 0.0),
        Ok(NearestTargetOutcome::Unreachable(_))
    ));
}

#[test]
fn a_connector_that_cannot_be_measured_leaves_the_walk_undecided() {
    // Declared a ramp, the stepped flight has no sloped run: it is not
    // climbed, and the levels it joins are open.
    let service = service(building());
    let outcome = nearest(
        &service,
        routing(VerticalConnectorKind::Ramp, ClimbLength::slope()),
        0.0,
    );
    assert!(
        matches!(&outcome, Err(MetricRoutingError::Unavailable(reason)) if reason.contains("not climbed")),
        "{outcome:?}"
    );
    // A lift is ridden, not walked: no length is measured for it.
    let outcome = nearest(
        &service,
        routing(VerticalConnectorKind::Lift, ClimbLength::slope()),
        0.0,
    );
    assert!(
        matches!(&outcome, Err(MetricRoutingError::Unavailable(reason)) if reason.contains("lift")),
        "{outcome:?}"
    );
}

#[test]
fn a_ramp_is_walked_along_its_slope() {
    // A wedge climbing 0.5 m along x from 2 to 8 (y 1..2.5), onto a
    // mezzanine from x = 8 whose floor is at 0.5 m.
    let wedge = prism(&[[2.0, 0.0], [8.0, 0.0], [8.0, 0.5]], 1.0, 2.5);
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("ground"), cuboid([0.0, 0.0, 0.0], [8.0, 4.0, 2.7]))
        .with_mesh(id("mezzanine"), cuboid([8.0, 0.0, 0.5], [12.0, 4.0, 3.2]))
        .with_mesh(id("stair"), wedge);
    let service = MetricRoutingServiceHandle::new(Arc::new(
        AxiolidMetricRoutingService::new(geometry, source())
            .with_surface(id("ground"))
            .with_surface(id("mezzanine")),
    ));
    let origin = MetricPoint::try_new(id("ground"), [1.0, 1.75, 0.0]).unwrap();
    let target = MetricPoint::try_new(id("mezzanine"), [11.0, 1.75, 0.5]).unwrap();
    let outcome = service
        .nearest_target(
            &NearestTargetRequest::try_new(origin, vec![target], walking(0.0))
                .unwrap()
                .with_connectors(routing(VerticalConnectorKind::Ramp, ClimbLength::slope())),
        )
        .unwrap();
    let NearestTargetOutcome::Reached(reached) = outcome else {
        panic!("expected a walk, got {outcome:?}");
    };
    let walked = reached.shortest_distance();
    let expected = 0.999 + 6.002_f64.hypot(0.5) + 2.999;
    holds(walked.lower_metres(), walked.upper_metres(), expected);
}

#[test]
fn the_farthest_point_between_an_exit_and_a_stair_converges() {
    // Upstairs, an exit stands at the far end (9.5, 3.5); downstairs one
    // stands just west of the stair's foot (1.5, 1.6). With the rise
    // counting nothing, the stair costs its 3.602 m in plan. From the
    // south-west corner (0, 0) the walk down the stair and on to the lower
    // exit is shorter than the one to the upper exit, and the largest over
    // the room.
    let service = service(building());
    let upper_exit = MetricPoint::try_new(id("upper"), [9.5, 3.5, 3.0]).unwrap();
    let ground_exit = MetricPoint::try_new(id("ground"), [1.5, 1.6, 0.0]).unwrap();
    let climb = ClimbLength::try_new(StairLength::Slope, 0.0).unwrap();
    let request = FarthestPointRequest::try_new(
        id("upper"),
        vec![ground_exit, upper_exit],
        walking(0.0),
        0.01,
    )
    .unwrap()
    .with_connectors(stairs(climb));
    let outcome = service.farthest_point(&request).unwrap();
    let FarthestPointOutcome::Bounded(bounded) = outcome else {
        panic!("expected a bracket, got {outcome:?}");
    };
    let beyond = (UPPER[0] - LOWER[0]) + distance(LOWER, [1.5, 1.6]);
    let expected = distance([0.0, 0.0], UPPER) + beyond;
    assert!(expected < distance([0.0, 0.0], [9.5, 3.5]));
    let bracket = bounded.distance();
    assert!(
        bracket.lower_metres() <= expected + SNAP && expected <= bracket.upper_metres() + SNAP,
        "{bracket:?} misses {expected}"
    );
    // Two sources on the room's level, weighted apart by the walk beyond
    // the stair: one weighted map brackets them to the tolerance.
    assert!(bounded.converged(), "{bounded:?}");
    let [x, y, _] = bounded.witness().coordinates_metres();
    assert!(x < 0.1 && y < 0.1, "{bounded:?}");
}

/// Travel over `object` counting `factor` metres a metre.
fn cost(object: &str, factor: f64) -> TravelCost {
    TravelCost::try_new(id(object), factor).unwrap()
}

fn weighted_nearest(
    service: &MetricRoutingServiceHandle,
    from: MetricPoint,
    to: MetricPoint,
    costs: Vec<TravelCost>,
) -> Result<NearestTargetOutcome, MetricRoutingError> {
    service.nearest_target(
        &NearestTargetRequest::try_new(from, vec![to], walking(0.0))
            .unwrap()
            .with_connectors(stairs(ClimbLength::slope()))
            .with_costs(costs),
    )
}

fn reached(outcome: Result<NearestTargetOutcome, MetricRoutingError>) -> NearestTargetEvidence {
    match outcome {
        Ok(NearestTargetOutcome::Reached(reached)) => reached,
        other => panic!("expected a walk, got {other:?}"),
    }
}

/// The bracket holds `value` and is at most `width` wide: a weighted one
/// is first order in the spacing of points along cost edges (5 mm), and
/// exact where walks cross them square.
fn brackets(lower: f64, upper: f64, value: f64, width: f64) {
    assert!(
        lower <= value + SNAP && value - SNAP <= upper,
        "[{lower}, {upper}] misses {value}"
    );
    assert!(upper - lower <= width, "[{lower}, {upper}] is loose");
}

#[test]
fn a_costed_slab_weighs_only_the_walks_on_its_own_level() {
    // Two slabs under the east strip (x 6..10): one under the ground
    // floor, one under the upper floor. Upstairs the walk runs straight
    // along y = 0.5 from x = 0.5 to 9.5, 3.5 m of it over the strip.
    let service = service(
        building()
            .with_mesh(
                id("ground-slab"),
                cuboid([6.0, 0.0, -0.25], [10.0, 4.0, -0.05]),
            )
            .with_mesh(
                id("upper-slab"),
                cuboid([6.0, 0.0, 2.75], [10.0, 4.0, 2.95]),
            ),
    );
    let to = MetricPoint::try_new(id("upper"), [9.5, 0.5, 3.0]).unwrap();
    let walk = |costs| {
        let walked = reached(weighted_nearest(&service, upstairs(), to.clone(), costs));
        let distance = walked.shortest_distance();
        (distance.lower_metres(), distance.upper_metres(), walked)
    };
    // The ground floor's slab lies on another level: it weighs the ground
    // level, reached down the stair, and the upper walk is its plain 9 m.
    let (lower, upper, walked) = walk(vec![cost("ground-slab", 3.0)]);
    brackets(lower, upper, 9.0, 1e-6);
    let locator = &walked.evidence().locator;
    assert!(
        locator.contains("costs=[") && locator.contains("weighed-levels=1"),
        "{locator}"
    );
    // The upper floor's own slab weighs it: 5.5 m plain, 3.5 m thrice.
    let (lower, upper, _) = walk(vec![cost("upper-slab", 3.0)]);
    brackets(lower, upper, 5.5 + 3.0 * 3.5, 0.02);
    // Without connectors the walk stays upstairs, weighed by the same rule.
    let outcome = service.nearest_target(
        &NearestTargetRequest::try_new(upstairs(), vec![to.clone()], walking(0.0))
            .unwrap()
            .with_costs(vec![cost("ground-slab", 3.0)]),
    );
    let alone = reached(outcome);
    let upstairs_only = alone.shortest_distance();
    brackets(
        upstairs_only.lower_metres(),
        upstairs_only.upper_metres(),
        9.0,
        1e-6,
    );
    // Both together weigh only the upper one's.
    let (lower, upper, _) = walk(vec![cost("ground-slab", 3.0), cost("upper-slab", 3.0)]);
    brackets(lower, upper, 5.5 + 3.0 * 3.5, 0.02);
}

#[test]
fn a_weighted_escape_down_the_stair_counts_the_ground_walk_by_its_factor() {
    // A slab under the whole ground floor doubles every ground metre; the
    // upper walk and the climb count once.
    let service = service(building().with_mesh(
        id("ground-slab"),
        cuboid([0.0, 0.0, -0.25], [10.0, 4.0, -0.05]),
    ));
    let slope = (UPPER[0] - LOWER[0]).hypot(3.0);
    let walked = reached(weighted_nearest(
        &service,
        upstairs(),
        exit(),
        vec![cost("ground-slab", 2.0)],
    ));
    let walk = walked.shortest_distance();
    let expected = distance([0.5, 0.5], UPPER) + slope + 2.0 * downstairs();
    brackets(walk.lower_metres(), walk.upper_metres(), expected, 0.02);
    assert_eq!(walked.waypoints().last(), Some(&exit()));

    // From the upper room's farthest point, its north-west corner.
    let request = FarthestPointRequest::try_new(id("upper"), vec![exit()], walking(0.0), 0.01)
        .unwrap()
        .with_connectors(stairs(ClimbLength::slope()))
        .with_costs(vec![cost("ground-slab", 2.0)]);
    let outcome = service.farthest_point(&request).unwrap();
    let FarthestPointOutcome::Bounded(bounded) = outcome else {
        panic!("expected a bracket, got {outcome:?}");
    };
    let expected = distance([0.0, 4.0], UPPER) + slope + 2.0 * downstairs();
    let bracket = bounded.distance();
    brackets(
        bracket.lower_metres(),
        bracket.upper_metres(),
        expected,
        0.03,
    );
    let [x, y, _] = bounded.witness().coordinates_metres();
    assert!(x < 0.1 && y > 3.9, "{bounded:?}");
}

#[test]
fn a_cost_on_the_stair_raises_only_the_climbs_upper_bound() {
    // The upper room begins at the stair's head (x 5.6..10), so no walk on
    // either level passes over the stair: its cost weighs only the climb,
    // at least once and at most twice.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("ground"), cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 2.7]))
        .with_mesh(id("upper"), cuboid([5.6, 0.0, 3.0], [10.0, 4.0, 5.7]))
        .with_mesh(id("stair"), stair());
    let service = service(geometry);
    let from = MetricPoint::try_new(id("upper"), [9.5, 0.5, 3.0]).unwrap();
    let slope = (UPPER[0] - LOWER[0]).hypot(3.0);
    let plain = distance([9.5, 0.5], UPPER) + slope + downstairs();
    let walked = reached(weighted_nearest(
        &service,
        from.clone(),
        exit(),
        vec![cost("stair", 2.0)],
    ));
    let walk = walked.shortest_distance();
    assert!(
        (walk.lower_metres() - plain).abs() < 1e-6,
        "{walk:?} misses {plain}"
    );
    assert!(
        (walk.upper_metres() - (plain + slope)).abs() < 1e-6,
        "{walk:?} misses {}",
        plain + slope
    );
    let locator = &walked.evidence().locator;
    assert!(locator.contains("*<=2"), "{locator}");
    // Unweighted, the climb counts once both ways.
    let walked = reached(weighted_nearest(&service, from, exit(), Vec::new()));
    let walk = walked.shortest_distance();
    holds(walk.lower_metres(), walk.upper_metres(), plain);
}

#[test]
fn a_cost_whose_level_cannot_be_resolved_is_refused() {
    // A canopy high above the upper room lies in no storey, yet over its
    // floor: which level it weighs is unknown.
    let service =
        service(building().with_mesh(id("canopy"), cuboid([6.0, 0.0, 9.0], [10.0, 4.0, 9.2])));
    let to = MetricPoint::try_new(id("upper"), [9.5, 0.5, 3.0]).unwrap();
    let outcome = weighted_nearest(&service, upstairs(), to, vec![cost("canopy", 2.0)]);
    assert!(
        matches!(&outcome, Err(MetricRoutingError::Unavailable(reason)) if reason.contains("cannot be resolved")),
        "{outcome:?}"
    );
    // A body with a radius is never weighed.
    let outcome = service.nearest_target(
        &NearestTargetRequest::try_new(upstairs(), vec![exit()], walking(0.2))
            .unwrap()
            .with_connectors(stairs(ClimbLength::slope()))
            .with_costs(vec![cost("stair", 2.0)]),
    );
    assert!(
        matches!(&outcome, Err(MetricRoutingError::Unavailable(reason)) if reason.contains("point only")),
        "{outcome:?}"
    );
}
