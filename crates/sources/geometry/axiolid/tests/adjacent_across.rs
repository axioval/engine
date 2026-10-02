//! `axioval:derived.adjacent-across` over real Axiolid geometry.
//!
//! Rooms `a` (x 0..4) and `b` (x 4.2..8) stand either side of wall `w`
//! (x 4..4.2); wall `e` (x -0.2..0) has `a` on one side only. A slab `s`
//! (z 3..3.2) covers both, with room `up` above it. Everything below the
//! slab is 3 m high.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidDerivedRelationshipService, AxiolidGeometry};
use axioval_engine::{
    AdjacentSide, Derivation, DerivedRelationshipServiceHandle, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, SemanticRelationship,
    TraversalDirection, across_side,
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
    let (points, indices) = parts(min, max);
    TriMesh::new(points, indices)
}

/// A box's points and triangles.
fn parts(min: [f64; 3], max: [f64; 3]) -> (Vec<Point3>, Vec<u32>) {
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
    (points, indices)
}

const SPACES: &[&str] = &["a", "b", "up"];

fn geometry() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [4.0, 4.0, 3.0]))
        .with_mesh(id("b"), cuboid([4.2, 0.0, 0.0], [8.0, 4.0, 3.0]))
        .with_mesh(id("up"), cuboid([0.0, 0.0, 3.2], [8.0, 4.0, 6.0]))
        .with_mesh(id("w"), cuboid([4.0, 0.0, 0.0], [4.2, 4.0, 3.0]))
        .with_mesh(id("e"), cuboid([-0.2, 0.0, 0.0], [0.0, 4.0, 3.0]))
        .with_mesh(id("s"), cuboid([0.0, 0.0, 3.0], [8.0, 4.0, 3.2]))
}

fn service(geometry: AxiolidGeometry, spaces: &[&str]) -> DerivedRelationshipServiceHandle {
    let mut service = AxiolidDerivedRelationshipService::new(geometry);
    for space in spaces {
        service = service.with_space(id(space));
    }
    for element in ["w", "e", "s", "bent"] {
        service = service.with_separating_element(id(element));
    }
    DerivedRelationshipServiceHandle::new(Arc::new(service))
}

const ACROSS: &str = "axioval:derived.adjacent-across";

fn request(
    anchor: &str,
    universe: &[&str],
    direction: TraversalDirection,
) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        id(anchor),
        universe.iter().map(|local| id(local)).collect(),
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new(ACROSS).unwrap(),
            direction,
            follow_chain: false,
        },
    )
    .unwrap()
}

/// The spaces beside `element` and the face each lies on, from the
/// evidence.
fn beside(
    handle: &DerivedRelationshipServiceHandle,
    element: &str,
) -> Result<Vec<(String, AdjacentSide)>, RelationshipSelectionError> {
    let universe = ["a", "b", "up", "w", "e", "s"];
    let selection = handle.select(&request(element, &universe, TraversalDirection::Forward))?;
    let mut found = Vec::new();
    for space in selection.candidates() {
        for item in selection.evidence() {
            if let Some(side) = across_side(&item.locator, &id(element), space) {
                found.push((space.local_id.clone(), side));
            }
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn a_wall_between_two_rooms_relates_to_both_one_on_each_face() {
    let handle = service(geometry(), SPACES);
    assert_eq!(
        beside(&handle, "w").unwrap(),
        [
            ("a".to_owned(), AdjacentSide::Negative),
            ("b".to_owned(), AdjacentSide::Positive)
        ]
    );
    // The canonical identity fixes the defaults, and every locator names it.
    let derivation = Derivation::parse(&SemanticRelationship::try_new(ACROSS).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        derivation.to_string(),
        "axioval:derived.adjacent-across;tolerance=0.05;overlap=0.3"
    );
}

#[test]
fn a_wall_with_a_room_on_one_side_relates_to_that_room_and_notes_the_other_face() {
    let handle = service(geometry(), SPACES);
    assert_eq!(
        beside(&handle, "e").unwrap(),
        [("a".to_owned(), AdjacentSide::Positive)]
    );
    let selection = handle
        .select(&request(
            "e",
            &["a", "b", "up"],
            TraversalDirection::Forward,
        ))
        .unwrap();
    assert!(
        selection
            .evidence()
            .iter()
            .any(|item| item.locator.contains("model/e:side=-") && item.locator.contains(":none")),
        "{:?}",
        selection.evidence()
    );
}

#[test]
fn a_slab_relates_to_the_rooms_above_and_below_it() {
    let handle = service(geometry(), SPACES);
    assert_eq!(
        beside(&handle, "s").unwrap(),
        [
            ("a".to_owned(), AdjacentSide::Negative),
            ("b".to_owned(), AdjacentSide::Negative),
            ("up".to_owned(), AdjacentSide::Positive)
        ]
    );
    // The room upstairs shares no height with the wall below it.
    assert!(
        !beside(&handle, "w")
            .unwrap()
            .iter()
            .any(|(space, _)| space == "up")
    );
}

#[test]
fn walking_backward_from_a_room_reaches_every_element_beside_it() {
    let handle = service(geometry(), SPACES);
    let selection = handle
        .select(&request(
            "b",
            &["a", "b", "up", "w", "e", "s"],
            TraversalDirection::Backward,
        ))
        .unwrap();
    let reached: Vec<&str> = selection
        .candidates()
        .iter()
        .map(|object| object.local_id.as_str())
        .collect();
    assert_eq!(reached, ["s", "w"]);
}

#[test]
fn a_room_whose_boundary_straddles_the_tolerance_leaves_the_wall_undecided() {
    // Exactly at the tolerance: rounding cannot tell within from beyond.
    let exact = geometry().with_mesh(id("b"), cuboid([4.25, 0.0, 0.0], [8.0, 4.0, 3.0]));
    let refused = beside(&service(exact, SPACES), "w").unwrap_err();
    assert!(
        matches!(&refused, RelationshipSelectionError::Unavailable(why) if why.contains("undecided")),
        "{refused:?}"
    );
    // A tessellated room 0.04 m away, within 0.02 m: it may be 0.06 m away.
    let tessellated =
        geometry().with_tessellated_mesh(id("b"), cuboid([4.24, 0.0, 0.0], [8.0, 4.0, 3.0]), 0.02);
    assert!(matches!(
        beside(&service(tessellated, SPACES), "w"),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
    // Within its deviation of nothing: 0.01 m away it is surely beside.
    let near =
        geometry().with_tessellated_mesh(id("b"), cuboid([4.21, 0.0, 0.0], [8.0, 4.0, 3.0]), 0.01);
    assert_eq!(
        beside(&service(near, SPACES), "w").unwrap(),
        [
            ("a".to_owned(), AdjacentSide::Negative),
            ("b".to_owned(), AdjacentSide::Positive)
        ]
    );
    // Well beyond the tolerance it is surely not.
    let far = geometry().with_mesh(id("b"), cuboid([4.5, 0.0, 0.0], [8.0, 4.0, 3.0]));
    assert_eq!(
        beside(&service(far, SPACES), "w").unwrap(),
        [("a".to_owned(), AdjacentSide::Negative)]
    );
}

#[test]
fn a_room_meeting_the_wall_along_less_than_the_overlap_is_not_beside_it() {
    // `b` meets the wall's face along 0.2 m only (y 3.8..4).
    let short = geometry().with_mesh(id("b"), cuboid([4.2, 3.8, 0.0], [8.0, 8.0, 3.0]));
    assert_eq!(
        beside(&service(short, SPACES), "w").unwrap(),
        [("a".to_owned(), AdjacentSide::Negative)]
    );
}

#[test]
fn an_element_neither_a_straight_wall_nor_a_flat_slab_refuses() {
    // An L-shaped wall: two boxes in one mesh.
    let (mut points, mut indices) = parts([4.0, 0.0, 0.0], [4.2, 4.0, 3.0]);
    let (more, triangles) = parts([4.2, 3.8, 0.0], [6.0, 4.0, 3.0]);
    points.extend(more);
    indices.extend(triangles.iter().map(|index| index + 8));
    let bent = geometry().with_mesh(id("bent"), TriMesh::new(points, indices));
    let handle = service(bent, SPACES);
    let refused = handle
        .select(&request("bent", &["a", "b"], TraversalDirection::Forward))
        .unwrap_err();
    assert!(
        matches!(&refused, RelationshipSelectionError::Unavailable(why) if why.contains("bent")),
        "{refused:?}"
    );
}
