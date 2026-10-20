//! Objects a host left out of a geometry set because no rule of its check
//! could ask about them (#410): a question about one, once the watch is
//! armed, is noted, and questions about others are not.

use std::collections::BTreeSet;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidFreeSpaceService, AxiolidGeometry};
use axioval_engine::{
    BoxClearance, ClearanceRequest, ClearanceShape, FreeSpaceService, MetricDirection, MetricFrame,
    MetricPoint,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

fn body(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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
        vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
    )
}

fn request(obstacles: &[&str]) -> ClearanceRequest {
    let frame = MetricFrame::try_new(
        MetricPoint::try_new(id("scope"), [1.0, 1.0, 0.0]).expect("valid point"),
        MetricDirection::try_new([1.0, 0.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 1.0, 0.0]).expect("valid direction"),
        MetricDirection::try_new([0.0, 0.0, 1.0]).expect("valid direction"),
    )
    .expect("valid frame");
    ClearanceRequest::new(
        frame,
        ClearanceShape::Box(BoxClearance::try_new(0.8, 0.8, 2.0).expect("valid box")),
        obstacles.iter().map(|local| id(local)).collect(),
    )
}

#[test]
fn a_question_about_a_left_out_object_is_noted_once_armed() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), body(8.0, 9.0, 8.0, 9.0, 0.0, 3.0))
        .with_left_out(BTreeSet::from([id("pipe")]));
    let watch = geometry.left_out().expect("a watch over the pipe");
    assert_eq!(watch.len(), 1);
    // A host building its services asks before it arms the watch.
    assert!(!geometry.has_no_body(&id("pipe")));
    let service = AxiolidFreeSpaceService::new(geometry, source());
    watch.arm();
    assert!(!watch.reached());

    // Measured among the objects the host meshed: nothing left out reached.
    service
        .assess_clearance(&request(&["column"]))
        .expect("measurable");
    assert!(!watch.reached());

    // The pipe is a named obstacle: its answer assumed it absent.
    let _ = service.assess_clearance(&request(&["column", "pipe"]));
    assert!(watch.reached());
}

#[test]
fn a_geometry_set_leaving_nothing_out_has_no_watch() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), body(8.0, 9.0, 8.0, 9.0, 0.0, 3.0))
        .with_left_out(BTreeSet::new());
    assert!(geometry.left_out().is_none());
}

/// Whether one object is unmeasured is a question about that object alone
/// (#448): counting a meshed body's triangles reaches nothing left out,
/// counting a left-out one's does, and whether an exact body is registered
/// is a question about every object while the host registered none.
#[test]
fn a_question_about_one_object_reaches_only_that_object() {
    use axioval_axiolid::AxiolidTriangleCountService;
    use axioval_engine::TriangleCountService;

    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), body(8.0, 9.0, 8.0, 9.0, 0.0, 3.0))
        .with_unmeasured(id("beam"), "no shape representation")
        .with_left_out(BTreeSet::from([id("pipe")]));
    let watch = geometry.left_out().expect("a watch over the pipe");
    let service = AxiolidTriangleCountService::new(geometry.clone());
    watch.arm();

    service.count_triangles(&id("column")).expect("countable");
    assert!(service.count_triangles(&id("beam")).is_err());
    assert!(!watch.reached());

    let _ = service.count_triangles(&id("pipe"));
    assert!(watch.reached());

    let geometry = AxiolidGeometry::new()
        .with_mesh(id("column"), body(8.0, 9.0, 8.0, 9.0, 0.0, 3.0))
        .with_left_out(BTreeSet::from([id("pipe")]));
    let watch = geometry.left_out().expect("a watch over the pipe");
    watch.arm();
    assert!(geometry.exact_boundary(&id("column")).is_none());
    assert!(watch.reached());
}
