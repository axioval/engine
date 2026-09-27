//! Triangle counts over registered meshes.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidTriangleCountService};
use axioval_engine::{TriangleCountError, TriangleCountService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented unit cube: twelve triangles.
fn cube() -> TriMesh {
    let points = vec![
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(1.0, 1.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
        Point3::new(0.0, 0.0, 1.0),
        Point3::new(1.0, 0.0, 1.0),
        Point3::new(1.0, 1.0, 1.0),
        Point3::new(0.0, 1.0, 1.0),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn service(geometry: AxiolidGeometry) -> AxiolidTriangleCountService {
    AxiolidTriangleCountService::new(geometry)
}

#[test]
fn a_planar_mesh_is_counted_with_exact_evidence() {
    let counts = service(AxiolidGeometry::new().with_mesh(id("box"), cube()));
    let count = counts.count_triangles(&id("box")).unwrap();
    assert_eq!(count.triangles(), 12);
    assert!(count.is_exact());
    assert_eq!(
        count.evidence().locator,
        format!("triangle-count:{}", id("box"))
    );
}

/// The host's tessellation decides the count, so it is not exact evidence.
#[test]
fn a_tessellation_is_counted_with_approximate_evidence() {
    let counts = service(AxiolidGeometry::new().with_tessellated_mesh(id("column"), cube(), 0.001));
    let count = counts.count_triangles(&id("column")).unwrap();
    assert_eq!(count.triangles(), 12);
    assert!(!count.is_exact());
}

#[test]
fn bodiless_objects_count_none_and_unmeasured_ones_refuse() {
    let counts = service(
        AxiolidGeometry::new()
            .with_no_body(id("storey"))
            .with_unmeasured(id("broken"), "meshing failed"),
    );
    let none = counts.count_triangles(&id("storey")).unwrap();
    assert_eq!(none.triangles(), 0);
    assert!(none.is_exact());
    assert!(matches!(
        counts.count_triangles(&id("broken")),
        Err(TriangleCountError::Unavailable(_))
    ));
    assert_eq!(
        counts.count_triangles(&id("missing")),
        Err(TriangleCountError::UnknownObject(id("missing")))
    );
}

/// One geometry set may hold several files; each count cites its object's.
#[test]
fn a_count_cites_the_counted_objects_own_source() {
    let other = ObjectId::new(SourceId::new("cad", "structure").unwrap(), "box").unwrap();
    let counts = service(
        AxiolidGeometry::new()
            .with_mesh(id("box"), cube())
            .with_mesh(other.clone(), cube()),
    );
    assert_eq!(
        counts.count_triangles(&other).unwrap().evidence().source,
        other.source
    );
    assert_eq!(
        counts
            .count_triangles(&id("box"))
            .unwrap()
            .evidence()
            .source,
        source()
    );
}

#[test]
fn a_mesh_with_indices_past_its_positions_refuses() {
    let broken = TriMesh::new(vec![Point3::new(0.0, 0.0, 0.0)], vec![0, 1, 2]);
    let counts = service(AxiolidGeometry::new().with_mesh(id("broken"), broken));
    assert!(matches!(
        counts.count_triangles(&id("broken")),
        Err(TriangleCountError::Unavailable(_))
    ));
}
