//! Circulation maps: pieces of the free area a path fits through, their
//! skeleton, and which entrances and components they come near.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    CirculationMap, CirculationNodeKind, CirculationRequest, FreeSpaceError, FreeSpaceService,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-wound box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
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
        vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7,
            6, 3, 0, 4, 3, 4, 7,
        ],
    )
}

/// A room 6 m by 4 m with a door in its south wall at x 0.5..1.5, a
/// partition at x 3.0..3.1 from the south wall up to `partition` and a WC
/// in the south-east corner.
fn room(partition: f64) -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [6.0, 4.0, 3.0]))
        .with_mesh(id("door"), cuboid([0.5, -0.2, 0.0], [1.5, 0.0, 2.1]))
        .with_mesh(
            id("partition"),
            cuboid([3.0, 0.0, 0.0], [3.1, partition, 3.0]),
        )
        .with_mesh(id("wc"), cuboid([5.3, 0.2, 0.0], [5.9, 0.9, 0.8]))
}

fn map(geometry: AxiolidGeometry, width: f64) -> Result<CirculationMap, FreeSpaceError> {
    let request = CirculationRequest::try_new(
        id("room"),
        vec![id("door")],
        vec![id("wc")],
        vec![id("partition"), id("wc"), id("door")],
        width,
        2.0,
        0.05,
    )?;
    AxiolidFreeSpaceService::new(geometry, source()).map_circulation(&request)
}

#[test]
fn a_wide_gap_joins_the_door_to_the_wc() {
    let map = map(room(2.5), 0.9).expect("mapped");
    assert_eq!(map.pieces(), 1, "{map:#?}");
    let door = map.contact(&id("door")).unwrap();
    let wc = map.contact(&id("wc")).unwrap();
    assert_eq!(door.reached().len(), 1, "{map:#?}");
    assert!(wc.reaches(door.reached()[0].0), "{map:#?}");
    // The door and the WC's nodes are in the piece.
    assert!(door.reached()[0].1.is_some() && wc.reached()[0].1.is_some());
    // Ends of the skeleton, each with a half width around it.
    let ends: Vec<_> = map
        .nodes()
        .iter()
        .filter(|node| node.kind() == CirculationNodeKind::End)
        .collect();
    assert!(ends.len() >= 2, "{ends:#?}");
    for node in map.nodes() {
        let half = node.half_width();
        assert!(half.lower_metres() >= 0.45 - 1.0e-6, "{node:?}");
        assert!(
            half.upper_metres() - half.lower_metres() < 1.0e-5,
            "{node:?}"
        );
    }
    assert_eq!(map.evidence().locator, "axiolid:circulation:room");
}

#[test]
fn a_gap_narrower_than_the_path_cuts_the_wc_off() {
    // 0.7 m are left north of the partition.
    let map = map(room(3.3), 0.9).expect("mapped");
    assert_eq!(map.pieces(), 2, "{map:#?}");
    let door = map.contact(&id("door")).unwrap();
    let wc = map.contact(&id("wc")).unwrap();
    assert_eq!(door.reached().len(), 1);
    assert_eq!(wc.reached().len(), 1);
    assert_ne!(door.reached()[0].0, wc.reached()[0].0);
    // Even from outside the erosion the two stay apart.
    assert!(
        door.possible()
            .iter()
            .all(|piece| !wc.possible().contains(piece)),
        "{map:#?}"
    );
    // A narrower path passes.
    let map = self::map(room(3.3), 0.6).expect("mapped");
    let door = map.contact(&id("door")).unwrap();
    let wc = map.contact(&id("wc")).unwrap();
    assert!(wc.reaches(door.reached()[0].0), "{map:#?}");
}

#[test]
fn a_gap_as_wide_as_the_path_is_neither_proven_nor_ruled_out() {
    // 0.9 m are left north of the partition, exactly the path's width.
    let map = map(room(3.1), 0.9).expect("mapped");
    let door = map.contact(&id("door")).unwrap();
    let wc = map.contact(&id("wc")).unwrap();
    let joined = door.reached().iter().any(|(piece, _)| wc.reaches(*piece));
    let apart = door
        .possible()
        .iter()
        .all(|piece| !wc.possible().contains(piece));
    assert!(!joined && !apart, "{map:#?}");
}

#[test]
fn subjects_without_a_body_and_services_without_maps_refuse() {
    let geometry = room(2.5).with_no_body(id("door"));
    assert!(matches!(
        map(geometry, 0.9),
        Err(FreeSpaceError::Unavailable(message)) if message.contains("has no body")
    ));
    assert!(matches!(
        map(AxiolidGeometry::new(), 0.9),
        Err(FreeSpaceError::MissingGeometry(_))
    ));
    assert_eq!(
        CirculationRequest::try_new(id("room"), vec![], vec![], vec![], 0.0, 2.0, 0.0),
        Err(FreeSpaceError::InvalidClearanceShape)
    );
}

#[test]
fn a_dead_end_corridor_ends_where_it_meets_its_end_wall() {
    // A block fills the room east of x 2 and north of y 1.2, leaving a
    // corridor 1.2 m wide along the south wall and a bay 2 m wide west of
    // the block.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [6.0, 4.0, 3.0]))
        .with_mesh(id("door"), cuboid([0.5, -0.2, 0.0], [1.5, 0.0, 2.1]))
        .with_mesh(id("block"), cuboid([2.0, 1.2, 0.0], [6.0, 4.0, 2.0]));
    let request = CirculationRequest::try_new(
        id("room"),
        vec![id("door")],
        vec![],
        vec![id("block")],
        0.9,
        2.0,
        0.05,
    )
    .unwrap();
    let map = AxiolidFreeSpaceService::new(geometry, source())
        .map_circulation(&request)
        .expect("mapped");
    assert!(map.unmapped().is_empty(), "{map:#?}");
    let mut ends: Vec<([f64; 3], f64)> = map
        .nodes()
        .iter()
        .filter(|node| node.kind() == CirculationNodeKind::End)
        .map(|node| (node.point(), node.half_width().lower_metres()))
        .collect();
    ends.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    assert_eq!(ends.len(), 2, "{ends:?}");
    // The bay's end, 1 m from its walls, and the corridor's, 0.6 m.
    assert!((ends[0].1 - 1.0).abs() < 0.01, "{ends:?}");
    assert!((ends[1].1 - 0.6).abs() < 0.01, "{ends:?}");
    assert!(
        ends[1].0[0] > 5.0 && (ends[1].0[1] - 0.6).abs() < 0.05,
        "{ends:?}"
    );
}
