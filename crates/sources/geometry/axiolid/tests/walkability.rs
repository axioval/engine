//! Walkable regions and passages from real Axiolid geometry.
//!
//! Two rooms, `a` (x 0..4) and `b` (x 4.2..8), 3 m high, share a 0.2 m wall
//! with a doorway at y 1.0 up to the door's far jamb; the wall is split into
//! a south piece, a north piece and a lintel above 2.1 m. A room `c` sits
//! above `a` on the next level, reached by a stair.
#![allow(clippy::float_cmp)] // bounds are copied, never computed

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidWalkabilityService};
use axioval_engine::{
    LengthInterval, MetricDirection, PassageAdmission, SweptDoor, SwingSector, VerticalConnector,
    VerticalConnectorKind, WalkabilityError, WalkabilityRegionId, WalkabilityRequest,
    WalkabilityRouteOutcome, WalkabilityServiceHandle, WalkabilitySnapshot,
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

/// The two rooms with a doorway `door_width` wide starting at y 1.0.
fn model(door_width: f64) -> AxiolidGeometry {
    let jamb = 1.0 + door_width;
    AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("b"), cuboid([4.2, 0.0, 0.0], [8.0, 4.0, 3.0]))
        .with_mesh(id("c"), cuboid([0.0, 0.0, 3.3], [4.0, 4.0, 6.3]))
        .with_mesh(id("wall-s"), cuboid([4.0, 0.0, 0.0], [4.2, 1.0, 3.0]))
        .with_mesh(id("wall-n"), cuboid([4.0, jamb, 0.0], [4.2, 4.0, 3.0]))
        .with_mesh(id("lintel"), cuboid([4.0, 1.0, 2.1], [4.2, jamb, 3.0]))
        .with_mesh(id("door"), cuboid([4.05, 1.0, 0.0], [4.15, jamb, 2.1]))
        .with_mesh(id("stair"), cuboid([0.5, 0.5, 0.0], [1.5, 3.0, 3.3]))
}

const WALLS: &[&str] = &["wall-s", "wall-n", "lintel"];

fn request(width: f64) -> WalkabilityRequest {
    WalkabilityRequest::try_new(
        vec![id("a"), id("b"), id("c")],
        vec![id("door")],
        WALLS.iter().map(|local| id(local)).collect(),
        width,
        None,
        true,
        false,
    )
    .unwrap()
}

fn snapshot(
    service: AxiolidWalkabilityService,
    request: &WalkabilityRequest,
) -> Result<WalkabilitySnapshot, WalkabilityError> {
    WalkabilityServiceHandle::new(Arc::new(service)).snapshot(request)
}

fn rid(value: &str) -> WalkabilityRegionId {
    WalkabilityRegionId::new(value).unwrap()
}

#[test]
fn a_route_between_two_rooms_passes_through_a_door_wide_enough() {
    let service =
        AxiolidWalkabilityService::new(model(0.9), source()).with_clear_width(id("door"), 0.85);
    let snapshot = snapshot(service, &request(0.8)).unwrap();
    assert!(snapshot.evidence().exact);
    let route = snapshot.route_between(&id("a"), &id("b")).unwrap();
    assert_eq!(
        route,
        WalkabilityRouteOutcome::Reachable(vec![
            rid("surface:cad:model/a"),
            rid("portal:cad:model/door:-"),
            rid("portal:cad:model/door:+"),
            rid("surface:cad:model/b"),
        ])
    );
    let crossing = snapshot
        .passages()
        .iter()
        .find(|passage| passage.portal() == Some(&id("door")))
        .unwrap();
    assert_eq!(crossing.clear_width().lower_metres(), 0.8);
    assert_eq!(crossing.clear_width().upper_metres(), 0.85);
    assert!(crossing.evidence().exact);
    assert!(crossing.evidence().locator.contains("sweep=proven"));
    // The door is reached from either room.
    assert!(matches!(
        snapshot.route_between(&id("b"), &id("door")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
}

#[test]
fn a_door_narrower_than_the_route_width_blocks_it() {
    // No clear width is stated: the opening alone bounds the width from
    // above, and 0.7 m cannot pass a 0.9 m body.
    let service = AxiolidWalkabilityService::new(model(0.7), source());
    let snapshot = snapshot(service, &request(0.9)).unwrap();
    assert_eq!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    let crossing = snapshot
        .passages()
        .iter()
        .find(|passage| passage.portal() == Some(&id("door")))
        .unwrap();
    assert!(crossing.clear_width().upper_metres() < 0.9);
    assert!(crossing.clear_width().upper_metres() >= 0.7);
    assert!(crossing.evidence().exact);
    assert!(snapshot.evidence().locator.contains("complete"));
}

#[test]
fn a_stated_clear_width_below_the_route_width_blocks_a_wide_opening() {
    let service =
        AxiolidWalkabilityService::new(model(1.2), source()).with_clear_width(id("door"), 0.8);
    let snapshot = snapshot(service, &request(0.9)).unwrap();
    assert_eq!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

#[test]
fn a_clear_width_the_request_states_decides_a_door() {
    // The rule reads the width from its source; the host states none.
    let stated = |metres: f64| {
        request(0.8)
            .with_stated_clear_widths([(id("door"), metres)])
            .unwrap()
    };
    let service = || AxiolidWalkabilityService::new(model(0.9), source());
    let wide = snapshot(service(), &stated(0.85)).unwrap();
    assert!(matches!(
        wide.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    let crossing = wide
        .passages()
        .iter()
        .find(|passage| passage.portal() == Some(&id("door")))
        .unwrap();
    assert_eq!(crossing.clear_width().upper_metres(), 0.85);
    assert!(
        crossing
            .evidence()
            .locator
            .contains("clearance=stated=0.85")
    );
    let narrow = snapshot(service(), &stated(0.75)).unwrap();
    assert_eq!(
        narrow.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    // Where host and request both state one, the narrower counts.
    let both = snapshot(service().with_clear_width(id("door"), 0.85), &stated(0.75)).unwrap();
    assert_eq!(
        both.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    // A stated width bounds an opening's void as well.
    let geometry = model(0.9).with_no_body(id("door"));
    let opening = AxiolidWalkabilityService::new(geometry, source())
        .with_opening_void(id("door"), cuboid([4.0, 1.0, 0.0], [4.2, 1.9, 2.1]));
    assert_eq!(
        snapshot(opening, &stated(0.75))
            .unwrap()
            .route_between(&id("a"), &id("b"))
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

#[test]
fn a_door_without_a_stated_clear_width_is_undecided_when_it_could_pass() {
    // The leaf and lining could narrow the 0.9 m opening below 0.8 m.
    let service = AxiolidWalkabilityService::new(model(0.9), source());
    let snapshot = snapshot(service, &request(0.8)).unwrap();
    assert_eq!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Indeterminate
    );
}

#[test]
fn a_bodiless_opening_is_its_own_clear_passage() {
    let geometry = model(0.9).with_no_body(id("door"));
    let service = AxiolidWalkabilityService::new(geometry, source())
        .with_opening_void(id("door"), cuboid([4.0, 1.0, 0.0], [4.2, 1.9, 2.1]));
    let snapshot = snapshot(service, &request(0.8)).unwrap();
    assert!(matches!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
}

#[test]
fn an_obstacle_in_front_of_the_door_is_not_proven_passable() {
    let geometry = model(0.9).with_mesh(id("cabinet"), cuboid([4.3, 0.5, 0.0], [5.0, 2.5, 2.0]));
    let service =
        AxiolidWalkabilityService::new(geometry, source()).with_clear_width(id("door"), 0.85);
    let mut obstacles: Vec<ObjectId> = WALLS.iter().map(|local| id(local)).collect();
    obstacles.push(id("cabinet"));
    let request = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![id("door")],
        obstacles,
        0.8,
        None,
        true,
        false,
    )
    .unwrap();
    // The cabinet stands 0.1 m in front of the doorway and reaches past it
    // on both sides: no body 0.8 m wide leaves the door into `b`, so every
    // possible piece of `b` lies beyond the door's reach.
    let snapshot = snapshot(service, &request).unwrap();
    let outcome = snapshot.route_between(&id("a"), &id("b")).unwrap();
    assert_eq!(outcome, WalkabilityRouteOutcome::Unreachable);
    let blocking = snapshot
        .blocking_passages(&id("a"), &id("b"), |_| PassageAdmission::Admitted)
        .unwrap();
    assert!(
        blocking
            .iter()
            .all(|passage| passage.evidence().locator.contains(":separated")),
        "{blocking:#?}"
    );
    assert!(!blocking.is_empty());
}

#[test]
fn a_cabinet_beside_the_door_leaves_room_to_pass() {
    // Moved 1 m into `b`, the cabinet leaves a 0.8 m body room between it
    // and the wall; the route is still undecided, as no sweep is proven
    // round it, but it is no longer ruled out.
    let geometry = model(0.9).with_mesh(id("cabinet"), cuboid([5.3, 0.5, 0.0], [6.0, 2.5, 2.0]));
    let service =
        AxiolidWalkabilityService::new(geometry, source()).with_clear_width(id("door"), 0.85);
    let mut obstacles: Vec<ObjectId> = WALLS.iter().map(|local| id(local)).collect();
    obstacles.push(id("cabinet"));
    let request = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![id("door")],
        obstacles,
        0.8,
        None,
        true,
        false,
    )
    .unwrap();
    let outcome = snapshot(service, &request)
        .unwrap()
        .route_between(&id("a"), &id("b"))
        .unwrap();
    assert_ne!(outcome, WalkabilityRouteOutcome::Unreachable);
}

/// A 0.9 m leaf hinged at `(x, y)`, closed along +x and opening towards -y.
fn leaf_swinging_south(door: &str, x: f64, y: f64) -> SweptDoor {
    SweptDoor::try_new(
        id(door),
        vec![
            SwingSector::try_new(
                [x, y, 0.0],
                0.9,
                MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
                MetricDirection::try_new([0.0, -1.0, 0.0]).unwrap(),
                false,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn a_swing_across_a_room_separates_it_but_not_its_own_door() {
    // A hatch hinged on `b`'s north wall at x 5 swings south over y 3.1..4;
    // a leaf hinged at the door's north jamb swings over the door's
    // landing. Neither closes `b`: the door's own swing is walked through.
    let service =
        AxiolidWalkabilityService::new(model(0.9), source()).with_clear_width(id("door"), 0.85);
    let request = request(0.8)
        .with_swept_doors(vec![
            leaf_swinging_south("hatch", 5.0, 4.0),
            SweptDoor::try_new(
                id("door"),
                vec![
                    SwingSector::try_new(
                        [4.2, 1.9, 0.0],
                        0.9,
                        MetricDirection::try_new([0.0, -1.0, 0.0]).unwrap(),
                        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
                        false,
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ])
        .unwrap();
    let snapshot = snapshot(service, &request).unwrap();
    assert!(matches!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    assert!(snapshot.evidence().locator.contains(":swept=2:"));
}

#[test]
fn a_swing_closing_a_corridor_cuts_off_the_room_beyond() {
    // `b` is cut down to a corridor 1.2 m wide (y 1..2.2) by walls north
    // and south of it; a hatch in its north wall at x 6 swings across it.
    let geometry = model(0.9)
        .with_mesh(id("south"), cuboid([4.2, 0.0, 0.0], [8.0, 1.0, 3.0]))
        .with_mesh(id("north"), cuboid([4.2, 2.2, 0.0], [8.0, 4.0, 3.0]));
    let mut obstacles: Vec<ObjectId> = WALLS.iter().map(|local| id(local)).collect();
    obstacles.extend([id("south"), id("north")]);
    let corridor = |swept: Vec<SweptDoor>| {
        let service = AxiolidWalkabilityService::new(geometry.clone(), source())
            .with_clear_width(id("door"), 0.85);
        let request = WalkabilityRequest::try_new(
            vec![id("a"), id("b")],
            vec![id("door")],
            obstacles.clone(),
            0.8,
            None,
            true,
            false,
        )
        .unwrap()
        .with_swept_doors(swept)
        .unwrap();
        snapshot(service, &request).unwrap()
    };
    // The corridor's far end is its own piece of `b` once the swing is
    // subtracted; `b` is still reached, but its east end is not.
    let open = corridor(Vec::new());
    let closed = corridor(vec![leaf_swinging_south("hatch", 6.0, 2.2)]);
    let pieces = |snapshot: &WalkabilitySnapshot| {
        snapshot
            .regions()
            .iter()
            .filter(|region| region.id().as_str().starts_with("surface:cad:model/b"))
            .count()
    };
    assert_eq!(pieces(&open), 1);
    assert_eq!(pieces(&closed), 2);
}

#[test]
fn portals_are_not_crossed_when_the_request_forbids_it() {
    let service =
        AxiolidWalkabilityService::new(model(0.9), source()).with_clear_width(id("door"), 0.85);
    let request = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![id("door")],
        WALLS.iter().map(|local| id(local)).collect(),
        0.8,
        None,
        false,
        false,
    )
    .unwrap();
    let snapshot = snapshot(service, &request).unwrap();
    assert!(snapshot.passages().iter().all(|p| p.portal().is_none()));
    assert_eq!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

#[test]
fn a_stairs_only_connection_is_unreachable_once_stairs_are_forbidden() {
    let service =
        AxiolidWalkabilityService::new(model(0.9), source()).with_clear_width(id("door"), 0.85);
    let request = request(0.8)
        .with_connectors(vec![VerticalConnector::new(
            id("stair"),
            VerticalConnectorKind::Stair,
        )])
        .unwrap();
    let snapshot = snapshot(service, &request).unwrap();
    let climb = snapshot
        .passages()
        .iter()
        .find(|passage| passage.connector().is_some())
        .unwrap();
    assert_eq!(
        climb.endpoints(),
        (&rid("surface:cad:model/a"), &rid("surface:cad:model/c"))
    );
    assert_eq!(climb.clear_width().lower_metres(), 0.0);
    // The climb is not measured, so the stair makes the route possible only.
    assert_eq!(
        snapshot.route_between(&id("b"), &id("c")).unwrap(),
        WalkabilityRouteOutcome::Indeterminate
    );
    assert_eq!(
        snapshot
            .route_between_avoiding(&id("b"), &id("c"), &[VerticalConnectorKind::Stair])
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
    // Forbidding stairs does not touch the level's own route.
    assert!(matches!(
        snapshot
            .route_between_avoiding(&id("a"), &id("b"), &[VerticalConnectorKind::Stair])
            .unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
}

#[test]
fn a_headroom_band_below_the_lintel_keeps_the_door_open() {
    let service =
        AxiolidWalkabilityService::new(model(0.9), source()).with_clear_width(id("door"), 0.85);
    let band = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![id("door")],
        WALLS.iter().map(|local| id(local)).collect(),
        0.8,
        Some(LengthInterval::try_new(0.0, 2.1).unwrap()),
        true,
        false,
    )
    .unwrap();
    assert!(matches!(
        snapshot(service, &band)
            .unwrap()
            .route_between(&id("a"), &id("b"))
            .unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
    // A band reaching into the lintel closes it.
    let service =
        AxiolidWalkabilityService::new(model(0.9), source()).with_clear_width(id("door"), 0.85);
    let tall = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![id("door")],
        WALLS.iter().map(|local| id(local)).collect(),
        0.8,
        Some(LengthInterval::try_new(0.0, 2.3).unwrap()),
        true,
        false,
    )
    .unwrap();
    assert_eq!(
        snapshot(service, &tall)
            .unwrap()
            .route_between(&id("a"), &id("b"))
            .unwrap(),
        WalkabilityRouteOutcome::Unreachable
    );
}

#[test]
fn missing_or_approximate_evidence_refuses() {
    let unmeasured = model(0.9).with_unmeasured(id("pillar"), "no body representation");
    let mut obstacles: Vec<ObjectId> = WALLS.iter().map(|local| id(local)).collect();
    obstacles.push(id("pillar"));
    let request_with_pillar = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![id("door")],
        obstacles,
        0.8,
        None,
        true,
        false,
    )
    .unwrap();
    let refused = |result: Result<WalkabilitySnapshot, WalkabilityError>, cause: &str| match result
    {
        Err(WalkabilityError::Unavailable(reason)) => assert!(reason.contains(cause), "{reason}"),
        other => panic!("expected a refusal naming {cause}, got {other:?}"),
    };
    refused(
        snapshot(
            AxiolidWalkabilityService::new(unmeasured, source()),
            &request_with_pillar,
        ),
        "pillar",
    );

    let curved =
        model(0.9).with_tessellated_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]), 1e-3);
    refused(
        snapshot(
            AxiolidWalkabilityService::new(curved, source()),
            &request(0.8),
        ),
        "tessellation",
    );

    let swings = WalkabilityRequest::try_new(
        vec![id("a")],
        vec![id("door")],
        Vec::new(),
        0.8,
        None,
        true,
        true,
    )
    .unwrap();
    refused(
        snapshot(
            AxiolidWalkabilityService::new(model(0.9), source()),
            &swings,
        ),
        "door swings",
    );
}

#[test]
fn a_passage_cites_the_source_of_the_object_it_measures() {
    // The door comes from a second file of the same federated set.
    let door_file = SourceId::new("cad", "doors").unwrap();
    let door = ObjectId::new(door_file.clone(), "door").unwrap();
    let geometry = model(0.9)
        .with_mesh(door.clone(), cuboid([4.05, 1.0, 0.0], [4.15, 1.9, 2.1]))
        .with_no_body(id("door"));
    let service =
        AxiolidWalkabilityService::new(geometry, source()).with_clear_width(door.clone(), 0.85);
    let request = WalkabilityRequest::try_new(
        vec![id("a"), id("b")],
        vec![door.clone()],
        WALLS.iter().map(|local| id(local)).collect(),
        0.8,
        None,
        true,
        false,
    )
    .unwrap();
    let snapshot = snapshot(service, &request).unwrap();
    let crossing = snapshot
        .passages()
        .iter()
        .find(|passage| passage.portal() == Some(&door))
        .unwrap();
    assert_eq!(crossing.evidence().source, door_file);
    assert_eq!(snapshot.evidence().source, source());
    assert!(matches!(
        snapshot.route_between(&id("a"), &id("b")).unwrap(),
        WalkabilityRouteOutcome::Reachable(_)
    ));
}
