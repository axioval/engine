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

/// A whole's openings subtracted from its parts are named on each part
/// they cut and on the whole, beside the union note.
#[test]
fn whole_openings_are_named_in_the_evidence_of_the_bodies_they_were_subtracted_from() {
    let parts = AxiolidGeometry::new()
        .with_mesh(id("layer"), cuboid([0.0, 0.0, 0.0], [4.0, 0.1, 3.0]))
        .with_mesh(id("slab"), cuboid([0.0, 0.0, -0.2], [4.0, 4.0, 0.0]))
        .with_whole_openings(id("layer"), vec![id("o2"), id("o1"), id("o2")]);
    let body = parts.compose(&[id("layer")]).unwrap();
    let geometry = parts
        .with_composed_body(id("wall"), body)
        .with_whole_openings(id("wall"), vec![id("o1"), id("o2")]);
    assert_eq!(
        geometry.whole_openings(&id("layer")),
        Some([id("o1"), id("o2")].as_slice())
    );
    assert_eq!(geometry.whole_openings(&id("slab")), None);

    let proximity = AxiolidProximityService::new(geometry.clone())
        .measure_proximity(&ProximityRequest::try_new(id("slab"), id("wall")).unwrap())
        .unwrap();
    let locator = &proximity.evidence().locator;
    assert!(
        locator.ends_with(
            ";union:cad:model/wall=1-parts;whole-openings:cad:model/wall=cad:model/o1+cad:model/o2"
        ),
        "{locator}"
    );
    let layer = AxiolidTriangleCountService::new(geometry)
        .count_triangles(&id("layer"))
        .unwrap();
    assert!(
        layer
            .evidence()
            .locator
            .ends_with(";whole-openings:cad:model/layer=cad:model/o1+cad:model/o2"),
        "{}",
        layer.evidence().locator
    );

    // An empty list states nothing.
    let none = AxiolidGeometry::new()
        .with_mesh(id("layer"), cuboid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]))
        .with_whole_openings(id("layer"), Vec::new());
    assert_eq!(none.whole_openings(&id("layer")), None);
}

/// An opening flush with a hole its part already has shares no volume
/// with it, and one reaching into its material shares some: the reading
/// the CLI decides by whether a whole's opening cuts a part.
#[test]
fn an_opening_flush_with_a_cut_shares_no_volume_and_one_into_material_shares_some() {
    let shared = |part: TriMesh, opening: TriMesh| {
        let geometry = AxiolidGeometry::new()
            .with_mesh(id("part"), part)
            .with_mesh(id("opening"), opening);
        let measured = AxiolidProximityService::new(geometry)
            .measure_proximity(&ProximityRequest::try_new(id("part"), id("opening")).unwrap())
            .unwrap();
        let volume = measured.intersection_volume().unwrap().shared();
        (volume.lower_cubic_metres(), volume.upper_cubic_metres())
    };
    // The part left of a hole from x = 1 to 2, the opening filling it.
    let (lower, upper) = shared(
        cuboid([0.0, 0.0, 0.0], [1.0, 0.1, 3.0]),
        cuboid([1.0, -0.1, 0.0], [2.0, 0.2, 2.0]),
    );
    assert!(lower == 0.0 && upper <= 1e-12, "{lower} {upper}");
    // An uncut part through the opening.
    let (lower, upper) = shared(
        cuboid([0.0, 0.0, 0.0], [4.0, 0.1, 3.0]),
        cuboid([1.0, -0.1, 0.0], [2.0, 0.2, 2.0]),
    );
    assert!(
        lower > 1e-12 && lower <= 0.2 && upper >= 0.2,
        "{lower} {upper}"
    );
}
