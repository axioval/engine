//! Leaf glazing measured from a door's body items (#205).

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{
    AxiolidLeafGlazingService, GlazingItem, ItemMaterial, RegisteredGlazingItems,
};
use axioval_engine::{
    DoorLeaf, DoorLeaves, HingeSide, LeafGlazing, LeafGlazingError, LeafGlazingService,
    LeafGlazingServiceHandle, LeafMotion, LeafPosition, MetricDirection, SourceSnapshot,
    SwingSector,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    turned_cuboid(min, max, |p| Point3::new(p[0], p[1], p[2]))
}

/// A closed, outward-oriented box, its corners moved by `place` (a rigid
/// motion keeping orientation).
fn turned_cuboid(min: [f64; 3], max: [f64; 3], place: impl Fn([f64; 3]) -> Point3) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let points = vec![
        place([x0, y0, z0]),
        place([x1, y0, z0]),
        place([x1, y1, z0]),
        place([x0, y1, z0]),
        place([x0, y0, z1]),
        place([x1, y0, z1]),
        place([x1, y1, z1]),
        place([x0, y1, z1]),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn item(label: &str, min: [f64; 3], max: [f64; 3], material: ItemMaterial) -> GlazingItem {
    GlazingItem {
        label: label.into(),
        mesh: cuboid(min, max),
        deviation: 0.0,
        material,
    }
}

fn direction(vector: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(vector).unwrap()
}

/// A swinging leaf `width` wide hinged at `hinge`, closed along `closed`,
/// opening towards +y.
fn leaf(hinge: [f64; 3], closed: [f64; 3], width: f64) -> DoorLeaf {
    let open = [0.0, 1.0, 0.0];
    let side = if closed[0] * open[1] - closed[1] * open[0] > 0.0 {
        HingeSide::Left
    } else {
        HingeSide::Right
    };
    DoorLeaf::try_new(
        LeafPosition::NotDefined,
        LeafMotion::Swing,
        hinge,
        direction(closed),
        direction(open),
        direction([0.0, 0.0, 1.0]),
        width,
        Some(0.04),
        Some(side),
        Some(
            SwingSector::try_new(hinge, width, direction(closed), direction(open), false).unwrap(),
        ),
    )
    .unwrap()
}

/// A double door 1.8 m wide: one leaf hinged at x = 0 closing along +x,
/// one hinged at x = 1.8 closing along -x, each 0.9 m.
fn double_door() -> DoorLeaves {
    DoorLeaves::try_new(
        id("door"),
        "DOUBLE_DOOR_SINGLE_SWING",
        1.8,
        None,
        vec![
            leaf([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.9),
            leaf([1.8, 0.0, 0.0], [-1.0, 0.0, 0.0], 0.9),
        ],
        Evidence::exact(source(), "door-leaves:door"),
    )
    .unwrap()
}

/// The door's frame and two solid leaf panels, 2.1 m high in all.
fn body() -> Vec<GlazingItem> {
    vec![
        item(
            "#frame",
            [-0.05, 0.0, 0.0],
            [1.85, 0.04, 2.1],
            ItemMaterial::Opaque,
        ),
        item(
            "#panel-1",
            [0.0, 0.0, 0.0],
            [0.9, 0.04, 2.0],
            ItemMaterial::Opaque,
        ),
        item(
            "#panel-2",
            [0.9, 0.0, 0.0],
            [1.8, 0.04, 2.0],
            ItemMaterial::Opaque,
        ),
    ]
}

/// A pane 0.5 m wide and 0.8 m high in the first leaf.
fn pane() -> GlazingItem {
    item(
        "#pane",
        [0.2, 0.01, 1.0],
        [0.7, 0.03, 1.8],
        ItemMaterial::Glazed,
    )
}

fn service(items: RegisteredGlazingItems) -> LeafGlazingServiceHandle {
    LeafGlazingServiceHandle::new(Arc::new(AxiolidLeafGlazingService::new(
        vec![SourceSnapshot::try_new(source(), "r1", "sha256:1").unwrap()],
        Arc::new(items),
    )))
}

fn close(actual: (f64, f64), expected: f64) {
    assert!(
        (actual.0 - expected).abs() < 1e-12 && (actual.1 - expected).abs() < 1e-12,
        "{actual:?} is not {expected}"
    );
}

#[test]
fn a_double_door_with_one_glazed_leaf_is_measured_per_leaf() {
    let mut items = body();
    items.push(pane());
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    assert!(glazing.is_exact());
    assert_eq!(
        glazing.evidence().locator,
        format!("leaf-glazing:{}", id("door"))
    );
    let [first, second] = glazing.leaves() else {
        panic!("two leaves");
    };
    let LeafGlazing::Measured {
        glazed,
        leaf,
        items,
    } = first
    else {
        panic!("the first leaf is measured: {first:?}");
    };
    close(*glazed, 0.4);
    close(*leaf, 0.9 * 2.1);
    assert_eq!(*items, 1);
    let ratio = first.ratio().unwrap();
    close(ratio, 0.4 / (0.9 * 2.1));
    let LeafGlazing::Measured { glazed, items, .. } = second else {
        panic!("the second leaf is measured: {second:?}");
    };
    close(*glazed, 0.0);
    assert_eq!(*items, 0);
    close(second.ratio().unwrap(), 0.0);
}

/// Panes overlapping in the leaf's plane count once.
#[test]
fn overlapping_panes_count_once() {
    let mut items = body();
    items.push(pane());
    items.push(item(
        "#pane-2",
        [0.2, 0.02, 1.4],
        [0.7, 0.03, 1.8],
        ItemMaterial::Glazed,
    ));
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    let LeafGlazing::Measured { glazed, items, .. } = &glazing.leaves()[0] else {
        panic!("measured");
    };
    close(*glazed, 0.4);
    assert_eq!(*items, 2);
}

/// An item whose material is undecided cannot be told apart from the
/// frame in the leaf it reaches into; the other leaf is still measured.
#[test]
fn an_undecided_item_leaves_its_leaf_undecided() {
    let mut items = body();
    items.push(pane());
    items.push(item(
        "#insert",
        [1.1, 0.01, 1.0],
        [1.6, 0.03, 1.8],
        ItemMaterial::Undecided("it has no surface style".into()),
    ));
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    assert!(matches!(glazing.leaves()[0], LeafGlazing::Measured { .. }));
    let LeafGlazing::Undecided(why) = &glazing.leaves()[1] else {
        panic!("undecided");
    };
    assert_eq!(
        why,
        "item #insert reaches into leaf 2 but cannot be told apart from the frame: it has no \
         surface style"
    );
    assert_eq!(glazing.leaves()[1].ratio(), None);
}

/// A frame item spanning both leaves whose material is undecided leaves
/// both undecided: the glazing cannot be told apart from it.
#[test]
fn an_undecided_frame_leaves_every_leaf_undecided() {
    let mut items = body();
    items[0].material = ItemMaterial::Undecided("its styles disagree".into());
    items.push(pane());
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    assert!(
        glazing
            .leaves()
            .iter()
            .all(|leaf| matches!(leaf, LeafGlazing::Undecided(_)))
    );
}

#[test]
fn a_pane_across_the_meeting_edge_cannot_be_assigned() {
    let mut items = body();
    items.push(item(
        "#wide",
        [0.6, 0.01, 1.0],
        [1.2, 0.03, 1.8],
        ItemMaterial::Glazed,
    ));
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    for (index, leaf) in glazing.leaves().iter().enumerate() {
        let LeafGlazing::Undecided(why) = leaf else {
            panic!("undecided");
        };
        assert_eq!(
            why,
            &format!(
                "glazed item #wide reaches past the edge of leaf {}, so it cannot be assigned \
                 to one leaf",
                index + 1
            )
        );
    }
}

/// A glazed side light beside the leaves belongs to neither.
#[test]
fn glazing_beside_the_leaves_belongs_to_none() {
    let mut items = body();
    items.push(item(
        "#side-light",
        [1.85, 0.01, 0.2],
        [2.3, 0.03, 2.0],
        ItemMaterial::Glazed,
    ));
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    for leaf in glazing.leaves() {
        close(leaf.ratio().unwrap(), 0.0);
    }
}

/// A tessellated pane is never exact: its area is an interval.
#[test]
fn a_tessellated_pane_is_an_interval() {
    let mut items = body();
    let mut tessellated = pane();
    tessellated.deviation = 0.001;
    items.push(tessellated);
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&double_door())
        .unwrap();
    assert!(!glazing.is_exact());
    let LeafGlazing::Measured { glazed, leaf, .. } = &glazing.leaves()[0] else {
        panic!("measured");
    };
    // The pane's outline is 2.6 m long: a band of 2·2.6·0.001 + π·0.001².
    let band = 2.0 * 2.6 * 0.001 + std::f64::consts::PI * 1e-6;
    assert!((glazed.0 - (0.4 - band)).abs() < 1e-9 && (glazed.1 - (0.4 + band)).abs() < 1e-9);
    assert!(leaf.0 < 0.9 * 2.1 && leaf.1 > 0.9 * 2.1);
}

/// A door off the coordinate axes rounds its projection: not exact, but
/// the interval holds the glazed area.
#[test]
fn a_door_off_the_axes_is_bracketed() {
    let (c, s) = (0.6_f64, 0.8_f64);
    let turn = |p: [f64; 3]| Point3::new(c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]);
    let rotated = |item: GlazingItem, min: [f64; 3], max: [f64; 3]| GlazingItem {
        mesh: turned_cuboid(min, max, turn),
        ..item
    };
    let mut items = vec![
        rotated(body().remove(1), [0.0, 0.0, 0.0], [0.9, 0.04, 2.1]),
        rotated(pane(), [0.2, 0.01, 1.0], [0.7, 0.03, 1.8]),
    ];
    items[0].label = "#panel".into();
    let turned = |hinge: [f64; 3], closed: [f64; 3]| {
        let open = [-s, c, 0.0];
        let closed = [c * closed[0], s * closed[0], 0.0];
        let hinge = [c * hinge[0], s * hinge[0], 0.0];
        DoorLeaf::try_new(
            LeafPosition::NotDefined,
            LeafMotion::Swing,
            hinge,
            direction(closed),
            direction(open),
            direction([0.0, 0.0, 1.0]),
            0.9,
            None,
            Some(HingeSide::Left),
            Some(
                SwingSector::try_new(hinge, 0.9, direction(closed), direction(open), false)
                    .unwrap(),
            ),
        )
    };
    let first = turned([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]).unwrap();
    let leaves = DoorLeaves::try_new(
        id("door"),
        "SINGLE_SWING_LEFT",
        0.9,
        None,
        vec![first],
        Evidence::exact(source(), "door-leaves:door"),
    )
    .unwrap();
    let glazing = service(RegisteredGlazingItems::new().with_items(id("door"), items))
        .measure_leaf_glazing(&leaves)
        .unwrap();
    assert!(!glazing.is_exact());
    let LeafGlazing::Measured { glazed, .. } = &glazing.leaves()[0] else {
        panic!("measured: {:?}", glazing.leaves());
    };
    assert!(glazed.0 < 0.4 && 0.4 < glazed.1 && glazed.1 - glazed.0 < 1e-6);
}

#[test]
fn a_door_without_a_body_or_items_is_refused() {
    let leaves = double_door();
    let handle = service(
        RegisteredGlazingItems::new()
            .with_no_body(id("door"))
            .with_unmeasured(id("other"), "its body could not be meshed"),
    );
    assert_eq!(
        handle.measure_leaf_glazing(&leaves),
        Err(LeafGlazingError::NoBody(id("door")))
    );
    let unknown = service(RegisteredGlazingItems::new());
    assert!(matches!(
        unknown.measure_leaf_glazing(&leaves),
        Err(LeafGlazingError::Unavailable(_))
    ));
}

#[test]
fn a_door_of_another_source_is_refused() {
    let service = AxiolidLeafGlazingService::new(
        vec![
            SourceSnapshot::try_new(SourceId::new("cad", "other").unwrap(), "r1", "sha256:1")
                .unwrap(),
        ],
        Arc::new(RegisteredGlazingItems::new().with_items(id("door"), body())),
    );
    assert!(service.measure_leaf_glazing(&double_door()).is_ok());
    assert_eq!(
        LeafGlazingServiceHandle::new(Arc::new(service)).measure_leaf_glazing(&double_door()),
        Err(LeafGlazingError::UncoveredSource(source()))
    );
}
