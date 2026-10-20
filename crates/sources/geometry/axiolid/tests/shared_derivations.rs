//! Placement scenes and indexed bodies are derivations of the check's
//! store (ADR 0006, #408): circle and rectangle searches over one space
//! build its scene once, pairs sharing a body index it once, and every
//! answer is the one built afresh.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry, AxiolidProximityService};
use axioval_engine::{
    BoxClearance, CylinderClearance, DerivationStatistics, DerivationStore, FreeSpaceService,
    PlacementOrientation, PlacementRequest, PlacementShape, ProximityRequest, ProximityService,
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
    let positions = vec![
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
        0, 2, 1, 0, 3, 2, // bottom, facing down
        4, 5, 6, 4, 6, 7, // top, facing up
        0, 1, 5, 0, 5, 4, // y0
        1, 2, 6, 1, 6, 5, // x1
        2, 3, 7, 2, 7, 6, // y1
        3, 0, 4, 3, 4, 7, // x0
    ];
    TriMesh::new(positions, indices)
}

/// A 5 m by 4 m room with a table in one corner and a cupboard in another.
fn furnished() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [5.0, 4.0, 3.0]))
        .with_mesh(id("table"), cuboid([0.0, 0.0, 0.0], [1.2, 0.8, 0.75]))
        .with_mesh(id("cupboard"), cuboid([4.4, 3.4, 0.0], [5.0, 4.0, 2.2]))
}

fn circle(diameter: f64, height: f64) -> PlacementShape {
    PlacementShape::Cylinder(CylinderClearance::try_new(diameter / 2.0, height).unwrap())
}

fn rectangle(width: f64, depth: f64) -> PlacementShape {
    PlacementShape::Box {
        shape: BoxClearance::try_new(width, depth, 2.0).unwrap(),
        orientation: PlacementOrientation::Any,
    }
}

fn request(shape: PlacementShape, obstacles: &[&str]) -> PlacementRequest {
    PlacementRequest::new(
        id("room"),
        shape,
        obstacles.iter().map(|object| id(object)).collect(),
    )
}

fn kind(store: &DerivationStore, name: &str) -> DerivationStatistics {
    store
        .statistics()
        .into_iter()
        .find(|kind| kind.kind == name)
        .unwrap_or_default()
}

/// Every search of `requests`, as the service answers it.
fn searched(service: &AxiolidFreeSpaceService, requests: &[PlacementRequest]) -> Vec<String> {
    requests
        .iter()
        .flat_map(|request| {
            [
                format!("{:?}", service.find_placement(request)),
                format!("{:?}", service.estimate_largest_disc(request)),
            ]
        })
        .collect()
}

#[test]
fn circle_and_rectangle_searches_over_one_space_build_its_scene_once() {
    let service = AxiolidFreeSpaceService::new(furnished(), source());
    let obstacles = ["table", "cupboard"];
    let requests = [
        request(circle(1.5, 2.0), &obstacles),
        request(rectangle(1.8, 1.5), &obstacles),
        request(circle(1.2, 2.0), &obstacles),
        request(rectangle(1.2, 0.9), &obstacles),
    ];
    let store = DerivationStore::new();
    let kept = {
        let _entered = store.enter();
        searched(&service, &requests)
    };
    let scenes = kind(&store, "placement scene");
    // One scene: every request shares the space, obstacles and band.
    assert_eq!((scenes.builds, scenes.hits), (1, 7));
    assert_eq!(scenes.entries, 1);
    assert!(scenes.bytes > 0);

    // Another obstacle set, or another band, is another scene.
    {
        let _entered = store.enter();
        let _ = service.find_placement(&request(rectangle(1.8, 1.5), &["table"]));
        let _ = service.find_placement(&request(circle(1.5, 2.5), &obstacles));
    }
    assert_eq!(kind(&store, "placement scene").builds, 3);

    // The answers are the ones a forgetful store, no store and a service
    // keeping nothing give.
    let forgetful = DerivationStore::forgetful();
    let forgotten = {
        let _entered = forgetful.enter();
        searched(&service, &requests)
    };
    assert_eq!(kind(&forgetful, "placement scene").builds, 8);
    let alone = searched(&service, &requests);
    let unkept = searched(
        &AxiolidFreeSpaceService::new(furnished(), source()).with_reuse(false),
        &requests,
    );
    assert_eq!(kept, forgotten);
    assert_eq!(kept, alone);
    assert_eq!(kept, unkept);
}

#[test]
fn pairs_sharing_a_body_index_it_once() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_mesh(id("b"), cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]))
        .with_mesh(id("c"), cuboid([0.5, 0.5, 0.5], [1.5, 1.5, 1.5]));
    let service = AxiolidProximityService::new(geometry);
    let pairs = [("a", "b"), ("a", "c"), ("b", "c")];
    let measure = || -> Vec<String> {
        pairs
            .iter()
            .map(|(subject, counterpart)| {
                let request = ProximityRequest::try_new(id(subject), id(counterpart)).unwrap();
                format!("{:?}", service.measure_proximity(&request))
            })
            .collect()
    };
    let store = DerivationStore::new();
    let kept = {
        let _entered = store.enter();
        measure()
    };
    let bodies = kind(&store, "indexed body");
    assert_eq!(bodies.builds, 3);
    assert!(bodies.hits >= 3, "{bodies:?}");
    // The material and void views share every body of its own.
    {
        let _entered = store.enter();
        let voids = service.measuring_voids();
        let _ = voids.measure_proximity(&ProximityRequest::try_new(id("a"), id("b")).unwrap());
    }
    assert_eq!(kind(&store, "indexed body").builds, 3);
    assert_eq!(kept, measure());
}

/// Services over clones of one geometry share its meshes and the bodies
/// indexed from them (#450); a clone changed after cloning is a set of its
/// own, its bodies indexed afresh, and the others keep what they had.
#[test]
fn services_over_clones_of_one_geometry_share_its_bodies() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_mesh(id("b"), cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]));
    let clone = geometry.clone();
    assert!(std::ptr::eq(
        geometry.mesh(&id("a")).unwrap(),
        clone.mesh(&id("a")).unwrap()
    ));
    let changed = geometry
        .clone()
        .with_mesh(id("b"), cuboid([4.0, 0.0, 0.0], [5.0, 1.0, 1.0]));
    assert!(!std::ptr::eq(
        geometry.mesh(&id("a")).unwrap(),
        changed.mesh(&id("a")).unwrap()
    ));
    let request = ProximityRequest::try_new(id("a"), id("b")).unwrap();
    let measure = |geometry: AxiolidGeometry| {
        format!(
            "{:?}",
            AxiolidProximityService::new(geometry).measure_proximity(&request)
        )
    };
    let store = DerivationStore::new();
    let (first, second, third) = {
        let _entered = store.enter();
        (
            measure(geometry.clone()),
            measure(clone),
            measure(changed.clone()),
        )
    };
    // Two bodies for the shared set, two more for the changed one.
    let bodies = kind(&store, "indexed body");
    assert_eq!(bodies.builds, 4, "{bodies:?}");
    assert_eq!(first, second);
    assert_ne!(first, third);
    assert_eq!(first, measure(geometry.clone()));
    assert_eq!(third, measure(changed));
    // The original still holds its own second body.
    assert_eq!(
        geometry.mesh(&id("b")).unwrap().positions[0],
        Point3::new(2.0, 0.0, 0.0)
    );
}
