//! Wholes with no body of their own, measured as the union of their parts.
#![allow(missing_docs, clippy::float_cmp)]

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidProximityService, AxiolidTriangleCountService};
use axioval_engine::{GeometryFidelity, ProximityRequest, ProximityService, TriangleCountService};
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

/// A stair of a flight (`x` 0..2) and a landing (`x` 2..3) meeting face
/// to face, both 1 m deep and 1 m high.
fn stair_parts() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("flight"), cuboid([0.0, 0.0, 0.0], [2.0, 1.0, 1.0]))
        .with_mesh(id("landing"), cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]))
}

fn stair() -> AxiolidGeometry {
    let geometry = stair_parts();
    let body = geometry
        .compose(&[id("landing"), id("flight")])
        .expect("both parts are measured");
    geometry.with_composed_body(id("stair"), body)
}

#[test]
fn a_whole_of_exact_parts_is_their_exact_union() {
    let geometry = stair_parts();
    let body = geometry.compose(&[id("landing"), id("flight")]).unwrap();
    assert!(body.is_exact());
    assert_eq!(body.deviation_metres(), 0.0);
    assert_eq!(body.parts(), [id("flight"), id("landing")]);
    assert!(!body.has_exact_body(), "no part has an exact body");
    let geometry = geometry.with_composed_body(id("stair"), body);

    assert_eq!(
        geometry.fidelity(&id("stair")).unwrap(),
        GeometryFidelity::Exact
    );
    assert_eq!(
        geometry.parts_of(&id("stair")),
        Some([id("flight"), id("landing")].as_slice())
    );
    assert_eq!(geometry.parts_of(&id("flight")), None);
    let count = AxiolidTriangleCountService::new(geometry.clone())
        .count_triangles(&id("stair"))
        .unwrap();
    assert_eq!(count.triangles(), 24);
    assert!(
        count
            .evidence()
            .locator
            .ends_with(";union:cad:model/stair=2-parts"),
        "{}",
        count.evidence().locator
    );

    // The whole's identity is its own: it shares material with its parts,
    // and only with them.
    assert!(geometry.shares_body(&id("stair"), &id("flight")));
    assert!(geometry.shares_body(&id("landing"), &id("stair")));
    assert!(!geometry.shares_body(&id("flight"), &id("landing")));
    assert!(!geometry.shares_body(&id("stair"), &id("stair")));
    let service = AxiolidProximityService::new(geometry);
    assert!(service.shares_body(&id("stair"), &id("flight")));
    assert!(!service.shares_body(&id("stair"), &id("wall")));
}

#[test]
fn a_tessellated_part_makes_the_whole_tessellated_within_its_largest_deviation() {
    let geometry = AxiolidGeometry::new()
        .with_tessellated_mesh(id("a"), cuboid([0.0; 3], [1.0; 3]), 1e-3)
        .with_tessellated_mesh(id("b"), cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]), 4e-4)
        .with_mesh(id("c"), cuboid([4.0, 0.0, 0.0], [5.0, 1.0, 1.0]));
    let body = geometry.compose(&[id("a"), id("b"), id("c")]).unwrap();
    assert!(!body.is_exact());
    assert_eq!(body.deviation_metres(), 1e-3);
    let geometry = geometry.with_composed_body(id("whole"), body);
    assert_eq!(
        geometry.fidelity(&id("whole")).unwrap().deviation_metres(),
        1e-3
    );
}

#[test]
fn a_whole_with_an_unmeasured_part_stays_unmeasured_naming_the_first() {
    let geometry = stair_parts()
        .with_unmeasured(id("rail"), "mesh compilation refused: no triangles")
        .with_unmeasured(id("post"), "no body representation");
    let error = geometry
        .compose(&[id("rail"), id("flight"), id("post"), id("landing")])
        .unwrap_err();
    assert_eq!(
        error,
        "its body is the union of its 4 parts, and part cad:model/post is unmeasured: \
         no body representation"
    );

    // A part with no state at all is not taken as bodiless.
    let error = stair_parts()
        .compose(&[id("flight"), id("ghost")])
        .unwrap_err();
    assert_eq!(
        error,
        "its body is the union of its 2 parts, and part cad:model/ghost has no measured body"
    );
}

#[test]
fn a_bodiless_part_adds_nothing_and_bodiless_parts_alone_are_no_body() {
    let geometry = stair_parts().with_no_body(id("opening"));
    let body = geometry.compose(&[id("flight"), id("opening")]).unwrap();
    assert_eq!(body.parts().len(), 2);
    assert_eq!(body.mesh().triangle_count(), 12);

    let error = geometry.compose(&[id("opening")]).unwrap_err();
    assert_eq!(
        error,
        "its body is the union of its 1 part, and none of them occupies material"
    );
    assert!(geometry.compose(&[]).is_err());
}

#[test]
fn an_inner_whole_shares_material_with_the_outer_one() {
    let geometry = stair();
    let geometry = geometry
        .clone()
        .with_mesh(id("rail"), cuboid([0.0, -0.1, 0.0], [3.0, 0.0, 2.0]));
    let outer = geometry.compose(&[id("stair"), id("rail")]).unwrap();
    assert_eq!(outer.mesh().triangle_count(), 36);
    let geometry = geometry.with_composed_body(id("stairway"), outer);
    for part in ["stair", "flight", "landing", "rail"] {
        assert!(geometry.shares_body(&id("stairway"), &id(part)), "{part}");
    }
    assert!(!geometry.shares_body(&id("stair"), &id("rail")));
}

#[test]
fn a_whole_measures_against_others_as_the_union_of_its_parts() {
    let geometry = stair().with_mesh(id("column"), cuboid([2.5, 2.0, 0.0], [2.7, 2.2, 3.0]));
    let service = AxiolidProximityService::new(geometry);
    let request = ProximityRequest::try_new(id("stair"), id("column")).unwrap();
    let measured = service.measure_proximity(&request).unwrap();
    // The column stands 1 m beyond the landing.
    assert!((measured.separation_metres() - 1.0).abs() < 1e-9);
    assert!(
        measured
            .evidence()
            .locator
            .ends_with(";union:cad:model/stair=2-parts"),
        "{}",
        measured.evidence().locator
    );

    // Touching parts share nothing: the union encloses their sum.
    let volume = service.measure_body_volume(&id("stair")).unwrap();
    assert!((volume.volume().lower_cubic_metres() - 3.0).abs() < 1e-9);
    assert!((volume.volume().upper_cubic_metres() - 3.0).abs() < 1e-9);
}

#[test]
fn overlapping_parts_bound_the_union_volume_and_a_shared_volume() {
    // Two unit cubes overlapping by half: the union encloses 1.5 m³, and a
    // probe covering the overlap shares 0.5 m³ with it, not 1.0.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("a"), cuboid([0.0; 3], [1.0; 3]))
        .with_mesh(id("b"), cuboid([0.5, 0.0, 0.0], [1.5, 1.0, 1.0]))
        .with_mesh(id("probe"), cuboid([0.5, 0.0, 0.0], [1.0, 1.0, 1.0]));
    let body = geometry.compose(&[id("a"), id("b")]).unwrap();
    let geometry = geometry.with_composed_body(id("whole"), body);
    let service = AxiolidProximityService::new(geometry);

    let volume = service.measure_body_volume(&id("whole")).unwrap().volume();
    assert!(volume.lower_cubic_metres() <= 1.5 + 1e-9, "{volume:?}");
    assert!(volume.upper_cubic_metres() >= 1.5 - 1e-9, "{volume:?}");
    assert!(volume.upper_cubic_metres() <= 2.0 + 1e-9, "{volume:?}");

    let request = ProximityRequest::try_new(id("whole"), id("probe")).unwrap();
    let shared = service
        .measure_proximity(&request)
        .unwrap()
        .intersection_volume()
        .expect("closed solids share a measured volume")
        .shared();
    assert!(shared.lower_cubic_metres() <= 0.5 + 1e-9, "{shared:?}");
    assert!(shared.upper_cubic_metres() >= 0.5 - 1e-9, "{shared:?}");
}
