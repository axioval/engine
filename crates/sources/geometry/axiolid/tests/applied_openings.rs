//! A host whose body already carries its openings' voids states them in
//! the evidence of every measurement of it.
#![allow(missing_docs)]

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService, AxiolidTriangleCountService};
use axioval_engine::{ProximityRequest, ProximityService, TriangleCountService};
use axioval_ir::{ObjectId, SourceId};

fn id(local: &str) -> ObjectId {
    ObjectId::new(SourceId::new("cad", "model").unwrap(), local).unwrap()
}

/// A closed, outward-oriented axis-aligned box.
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
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front
            3, 7, 6, 3, 6, 2, // back
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ],
    )
}

#[test]
fn applied_openings_are_named_in_the_evidence_of_their_host() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("wall"), cuboid([0.0, 0.0, 0.0], [4.0, 0.2, 3.0]))
        .with_mesh(id("slab"), cuboid([0.0, 0.0, -0.2], [4.0, 4.0, 0.0]))
        .with_applied_openings(id("wall"), vec![id("o2"), id("o1"), id("o2")]);
    assert_eq!(
        geometry.applied_openings(&id("wall")),
        Some([id("o1"), id("o2")].as_slice())
    );
    assert_eq!(geometry.applied_openings(&id("slab")), None);

    let count = AxiolidTriangleCountService::new(geometry.clone())
        .count_triangles(&id("wall"))
        .unwrap();
    assert!(
        count
            .evidence()
            .locator
            .ends_with(";applied-openings:cad:model/wall=cad:model/o1+cad:model/o2"),
        "{}",
        count.evidence().locator
    );
    let slab = AxiolidTriangleCountService::new(geometry.clone())
        .count_triangles(&id("slab"))
        .unwrap();
    assert!(
        !slab.evidence().locator.contains("applied-openings"),
        "{}",
        slab.evidence().locator
    );

    let proximity = AxiolidProximityService::new(geometry)
        .measure_proximity(&ProximityRequest::try_new(id("slab"), id("wall")).unwrap())
        .unwrap();
    let locator = &proximity.evidence().locator;
    assert!(
        locator.contains(";applied-openings:cad:model/wall=cad:model/o1+cad:model/o2"),
        "{locator}"
    );

    // An empty list states nothing.
    let none = AxiolidGeometry::new()
        .with_mesh(id("wall"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_applied_openings(id("wall"), Vec::new());
    assert_eq!(none.applied_openings(&id("wall")), None);
}
