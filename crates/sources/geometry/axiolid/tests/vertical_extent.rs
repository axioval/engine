//! Bottom and top elevations over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidVerticalExtentService};
use axioval_engine::{VerticalExtentError, VerticalExtentService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented 4 x 3 box from `z0` to `z1`.
fn slab(z0: f64, z1: f64) -> TriMesh {
    let points = vec![
        Point3::new(0.0, 0.0, z0),
        Point3::new(4.0, 0.0, z0),
        Point3::new(4.0, 3.0, z0),
        Point3::new(0.0, 3.0, z0),
        Point3::new(0.0, 0.0, z1),
        Point3::new(4.0, 0.0, z1),
        Point3::new(4.0, 3.0, z1),
        Point3::new(0.0, 3.0, z1),
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

fn service(geometry: AxiolidGeometry) -> AxiolidVerticalExtentService {
    AxiolidVerticalExtentService::new(geometry, source())
}

#[test]
fn a_planar_slab_measures_its_bottom_and_top_exactly() {
    let extents = service(AxiolidGeometry::new().with_mesh(id("slab"), slab(2.8, 3.0)));
    let extent = extents.measure_vertical_extent(&id("slab")).unwrap();
    assert!(extent.is_exact());
    assert_eq!(extent.object(), &id("slab"));
    assert!((extent.bottom().lower_metres() - 2.8).abs() < 1e-12);
    assert!((extent.top().upper_metres() - 3.0).abs() < 1e-12);
    assert!(extent.bottom().is_exact() && extent.top().is_exact());
    assert_eq!(
        extent.evidence().locator,
        format!("vertical-extent:{}", id("slab"))
    );
}

/// A tessellation is never exact: each elevation widens by the chord
/// deviation, and the evidence says it is approximate.
#[test]
fn a_tessellated_slab_measures_intervals_with_approximate_evidence() {
    let extents =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("vault"), slab(2.8, 3.0), 0.01));
    let extent = extents.measure_vertical_extent(&id("vault")).unwrap();
    assert!(!extent.is_exact());
    assert!(!extent.evidence().exact);
    assert!((extent.top().lower_metres() - 2.99).abs() < 1e-12);
    assert!((extent.top().upper_metres() - 3.01).abs() < 1e-12);
    assert!((extent.bottom().lower_metres() - 2.79).abs() < 1e-12);

    // Even a declared zero deviation stays approximate.
    let zero =
        service(AxiolidGeometry::new().with_tessellated_mesh(id("vault"), slab(2.8, 3.0), 0.0));
    let extent = zero.measure_vertical_extent(&id("vault")).unwrap();
    assert!(!extent.is_exact() && !extent.top().is_exact());
}

#[test]
fn bodiless_unmeasured_and_unknown_objects_have_no_extent() {
    let extents = service(
        AxiolidGeometry::new()
            .with_no_body(id("storey"))
            .with_unmeasured(id("broken"), "meshing failed"),
    );
    assert!(matches!(
        extents.measure_vertical_extent(&id("storey")),
        Err(VerticalExtentError::Unavailable(_))
    ));
    assert!(matches!(
        extents.measure_vertical_extent(&id("broken")),
        Err(VerticalExtentError::Unavailable(_))
    ));
    assert_eq!(
        extents.measure_vertical_extent(&id("missing")),
        Err(VerticalExtentError::UnknownObject(id("missing")))
    );
}

#[test]
fn an_invalid_declared_deviation_is_refused_not_exact() {
    let extents = service(AxiolidGeometry::new().with_tessellated_mesh(
        id("vault"),
        slab(0.0, 0.2),
        f64::NAN,
    ));
    assert_eq!(
        extents.measure_vertical_extent(&id("vault")),
        Err(VerticalExtentError::InvalidMeasurement)
    );
}
