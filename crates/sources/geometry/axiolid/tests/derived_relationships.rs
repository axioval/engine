//! Relationships derived from real Axiolid geometry.
//!
//! Two rooms, `a` (x 0..4) and `b` (x 4.2..8), share a 0.2 m wall; an
//! external wall runs along x -0.2..0. Everything is 3 m high.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidDerivedRelationshipService, AxiolidGeometry};
use axioval_engine::{
    DerivedRelationshipServiceHandle, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, SemanticRelationship, TraversalDirection,
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

fn rooms() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("b"), cuboid([4.2, 0.0, 0.0], [8.0, 4.0, 3.0]))
        // A chair in a, a cabinet in b, a lamp in the shared wall nearer a.
        .with_mesh(id("chair"), cuboid([0.5, 0.5, 0.0], [1.0, 1.0, 0.9]))
        .with_mesh(id("cabinet"), cuboid([6.0, 1.0, 0.0], [7.0, 1.5, 2.0]))
        .with_mesh(id("lamp"), cuboid([4.02, 2.0, 1.8], [4.08, 2.2, 2.0]))
        // An internal door in the shared wall, an external one in the facade.
        .with_mesh(id("door"), cuboid([4.05, 1.0, 0.0], [4.15, 1.9, 2.1]))
        .with_mesh(id("exit"), cuboid([-0.15, 1.0, 0.0], [-0.05, 1.9, 2.1]))
        .with_no_body(id("storey"))
}

fn service(geometry: AxiolidGeometry) -> AxiolidDerivedRelationshipService {
    AxiolidDerivedRelationshipService::new(geometry)
        .with_space(id("a"))
        .with_space(id("b"))
        .with_opening(id("door"))
        .with_opening(id("exit"))
}

fn handle(service: AxiolidDerivedRelationshipService) -> DerivedRelationshipServiceHandle {
    DerivedRelationshipServiceHandle::new(Arc::new(service))
}

fn request(
    anchor: &str,
    universe: &[&str],
    relationship: &str,
    direction: TraversalDirection,
    follow_chain: bool,
) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        id(anchor),
        universe.iter().map(|local| id(local)).collect(),
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new(relationship).unwrap(),
            direction,
            follow_chain,
        },
    )
    .unwrap()
}

fn select(
    handle: &DerivedRelationshipServiceHandle,
    anchor: &str,
    universe: &[&str],
    relationship: &str,
    direction: TraversalDirection,
) -> Result<(Vec<String>, Vec<String>), RelationshipSelectionError> {
    let selection = handle.select(&request(anchor, universe, relationship, direction, false))?;
    Ok((
        selection
            .candidates()
            .iter()
            .map(|candidate| candidate.local_id.clone())
            .collect(),
        selection
            .evidence()
            .iter()
            .map(|item| item.locator.clone())
            .collect(),
    ))
}

const CONTAINED: &str = "axioval:derived.contained-in-space";
const ADJACENT: &str = "axioval:derived.adjacent-space";
const GROUP: &str = "axioval:derived.overlapping-group-space";
const COMPONENTS: &[&str] = &["chair", "cabinet", "lamp", "storey"];

#[test]
fn components_are_counted_in_the_space_that_contains_them() {
    let handle = handle(service(rooms()));
    let (in_a, evidence) = select(
        &handle,
        "a",
        COMPONENTS,
        CONTAINED,
        TraversalDirection::Backward,
    )
    .unwrap();
    assert_eq!(in_a, ["chair"]);
    assert!(
        evidence
            .iter()
            .all(|locator| locator.starts_with(CONTAINED)),
        "{evidence:?}"
    );
    assert!(
        evidence
            .iter()
            .any(|locator| locator.contains("/chair->cad:model/a:contains-point")),
        "{evidence:?}"
    );
    let (in_b, _) = select(
        &handle,
        "b",
        COMPONENTS,
        CONTAINED,
        TraversalDirection::Backward,
    )
    .unwrap();
    assert_eq!(in_b, ["cabinet"]);
    let (spaces, _) = select(
        &handle,
        "chair",
        &["a", "b"],
        CONTAINED,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a"]);
}

#[test]
fn an_element_outside_every_space_goes_to_the_nearest_within_the_tolerances() {
    let handle = handle(service(rooms()));
    let (strict, evidence) = select(
        &handle,
        "lamp",
        &["a", "b"],
        CONTAINED,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert!(strict.is_empty());
    assert!(
        evidence
            .iter()
            .any(|locator| locator.contains("in-no-space")),
        "{evidence:?}"
    );
    // 0.05 m from a in plan, 0.15 m from b.
    let near = "axioval:derived.contained-in-space;horizontal=0.1";
    let (spaces, evidence) = select(
        &handle,
        "lamp",
        &["a", "b"],
        near,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a"]);
    assert!(
        evidence.iter().any(|locator| locator
            .starts_with("axioval:derived.contained-in-space;horizontal=0.1;vertical=0:")
            && locator.contains(":nearest")),
        "{evidence:?}"
    );
    let (in_a, _) = select(&handle, "a", COMPONENTS, near, TraversalDirection::Backward).unwrap();
    assert_eq!(in_a, ["chair", "lamp"]);
    let too_far = "axioval:derived.contained-in-space;horizontal=0.04";
    let (none, _) = select(
        &handle,
        "lamp",
        &["a", "b"],
        too_far,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert!(none.is_empty());
}

#[test]
fn a_door_relates_to_the_two_spaces_it_connects_and_an_exit_to_one() {
    let handle = handle(service(rooms()));
    let (spaces, evidence) = select(
        &handle,
        "door",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a", "b"]);
    let sides: Vec<&String> = evidence
        .iter()
        .filter(|locator| locator.contains(":side="))
        .collect();
    assert_eq!(sides.len(), 2, "{evidence:?}");
    assert!(
        sides
            .iter()
            .any(|locator| locator.contains("/a:side=-(1.000000,0.000000)")),
        "{evidence:?}"
    );
    assert!(
        sides
            .iter()
            .any(|locator| locator.contains("/b:side=+(1.000000,0.000000)")),
        "{evidence:?}"
    );

    let (spaces, evidence) = select(
        &handle,
        "exit",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a"]);
    assert!(
        evidence
            .iter()
            .any(|locator| locator.contains("exit:side=-") && locator.contains(":outside")),
        "{evidence:?}"
    );

    let (doors_of_a, _) = select(
        &handle,
        "a",
        &["door", "exit", "chair"],
        ADJACENT,
        TraversalDirection::Backward,
    )
    .unwrap();
    assert_eq!(doors_of_a, ["door", "exit"]);
    // Too short a reach to cross from the leaf to either room.
    let short = "axioval:derived.adjacent-space;reach=0.01";
    let (none, _) = select(
        &handle,
        "door",
        &["a", "b"],
        short,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert!(none.is_empty());
}

#[test]
fn a_space_belongs_to_the_larger_space_covering_it() {
    let geometry = rooms()
        .with_mesh(id("gross"), cuboid([0.0, 0.0, 0.0], [8.2, 4.0, 3.0]))
        .with_mesh(id("wing"), cuboid([-1.0, -1.0, 0.0], [20.0, 10.0, 3.0]))
        .with_mesh(id("upstairs"), cuboid([0.0, 0.0, 3.5], [4.0, 4.0, 6.0]));
    let handle = handle(
        service(geometry)
            .with_space(id("gross"))
            .with_space(id("wing"))
            .with_space(id("upstairs")),
    );
    let all = &["a", "b", "gross", "wing", "upstairs"];
    let ratio = "axioval:derived.overlapping-group-space;ratio=0.9";
    let (groups, evidence) = select(&handle, "a", all, ratio, TraversalDirection::Forward).unwrap();
    assert_eq!(groups, ["gross", "wing"]);
    assert!(
        evidence
            .iter()
            .any(|locator| locator.contains("covers=1.000000")),
        "{evidence:?}"
    );
    let (members, _) = select(&handle, "gross", all, GROUP, TraversalDirection::Backward).unwrap();
    assert_eq!(members, ["a", "b"]);
    // Upstairs lies 0.5 m above the gross space: close only with a tolerance.
    let (groups, _) = select(&handle, "upstairs", all, GROUP, TraversalDirection::Forward).unwrap();
    assert!(groups.is_empty());
    let (groups, _) = select(
        &handle,
        "upstairs",
        all,
        "axioval:derived.overlapping-group-space;vertical=0.5",
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(groups, ["gross", "wing"]);
    // Transitively, a room reaches the wing through the gross space too.
    let chained = handle
        .select(&request(
            "wing",
            &["a", "b", "gross"],
            GROUP,
            TraversalDirection::Backward,
            true,
        ))
        .unwrap();
    assert_eq!(chained.candidates(), [id("a"), id("b"), id("gross")]);

    // A space enclosing both faces of a door is on neither side of it.
    let (spaces, _) = select(
        &handle,
        "door",
        &["a", "b", "gross", "wing"],
        ADJACENT,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a", "b"]);
}

fn refusal(result: Result<(Vec<String>, Vec<String>), RelationshipSelectionError>) -> String {
    match result {
        Err(RelationshipSelectionError::Unavailable(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn unmeasured_bodies_boundaries_and_ties_refuse() {
    // An unmeasured or bodiless space could hold anything.
    let unmeasured =
        handle(service(rooms().with_unmeasured(id("c"), "no mesh")).with_space(id("c")));
    let message = refusal(select(
        &unmeasured,
        "a",
        COMPONENTS,
        CONTAINED,
        TraversalDirection::Backward,
    ));
    assert!(message.contains("was not measured"), "{message}");
    let bodiless = handle(service(rooms().with_no_body(id("c"))).with_space(id("c")));
    let message = refusal(select(
        &bodiless,
        "door",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    ));
    assert!(message.contains("has no body"), "{message}");

    // An unmeasured component in the universe might be in the space.
    let component = handle(service(rooms().with_unmeasured(id("sofa"), "no mesh")));
    let message = refusal(select(
        &component,
        "a",
        &["chair", "sofa"],
        CONTAINED,
        TraversalDirection::Backward,
    ));
    assert!(message.contains("sofa"), "{message}");

    // A reference point on a space's boundary is neither in nor out.
    let on_boundary = handle(service(
        rooms().with_mesh(id("shelf"), cuboid([3.8, 3.0, 0.0], [4.2, 3.5, 2.0])),
    ));
    let message = refusal(select(
        &on_boundary,
        "shelf",
        &["a", "b"],
        CONTAINED,
        TraversalDirection::Forward,
    ));
    assert!(message.contains("boundary"), "{message}");

    // Equally near two spaces.
    let middle = handle(service(
        rooms().with_mesh(id("pipe"), cuboid([4.05, 3.0, 1.0], [4.15, 3.1, 1.2])),
    ));
    let message = refusal(select(
        &middle,
        "pipe",
        &["a", "b"],
        "axioval:derived.contained-in-space;horizontal=0.2",
        TraversalDirection::Forward,
    ));
    assert!(message.contains("equally near"), "{message}");
}

#[test]
fn tessellations_near_the_decision_and_shapeless_openings_refuse() {
    let curved_room = handle(service(rooms().with_tessellated_mesh(
        id("b"),
        cuboid([4.2, 0.0, 0.0], [8.0, 4.0, 3.0]),
        0.001,
    )));
    let message = refusal(select(
        &curved_room,
        "door",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    ));
    assert!(message.contains("tessellation"), "{message}");
    let curved_door = handle(service(rooms().with_tessellated_mesh(
        id("door"),
        cuboid([4.05, 1.0, 0.0], [4.15, 1.9, 2.1]),
        0.001,
    )));
    let message = refusal(select(
        &curved_door,
        "door",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    ));
    assert!(message.contains("tessellation"), "{message}");
    // A tessellation far from the question blocks nothing.
    let far = handle(
        service(rooms().with_tessellated_mesh(
            id("far"),
            cuboid([40.0, 0.0, 0.0], [44.0, 4.0, 3.0]),
            0.001,
        ))
        .with_space(id("far")),
    );
    let (spaces, _) = select(
        &far,
        "chair",
        &["a", "b"],
        CONTAINED,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a"]);

    // A square opening has no single direction through it.
    let square = handle(service(
        rooms().with_mesh(id("door"), cuboid([4.0, 1.0, 0.0], [4.2, 1.2, 2.1])),
    ));
    let message = refusal(select(
        &square,
        "door",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    ));
    assert!(message.contains("single direction"), "{message}");

    // Every refusal names the derivation it was asked for.
    assert!(
        message.starts_with("axioval:derived.adjacent-space;reach=1:"),
        "{message}"
    );
}

#[test]
fn a_bodiless_opening_is_probed_through_its_void() {
    let geometry = rooms().with_no_body(id("void"));
    let handle = handle(
        service(geometry).with_opening_void(id("void"), cuboid([4.0, 2.5, 0.0], [4.2, 3.4, 2.1])),
    );
    let (spaces, _) = select(
        &handle,
        "void",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert_eq!(spaces, ["a", "b"]);
    // Non-openings have no adjacent spaces, exactly.
    let (none, _) = select(
        &handle,
        "chair",
        &["a", "b"],
        ADJACENT,
        TraversalDirection::Forward,
    )
    .unwrap();
    assert!(none.is_empty());
}
