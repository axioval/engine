//! Thicknesses and perimeters over real Axiolid geometry: a tapered member
//! spans its thinnest and thickest, a sloped slab measured square to its
//! top is as thick everywhere, and a footprint's boundary is measured.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService, AxiolidVerticalExtentService};
use axioval_engine::{
    MetricDirection, PlanAreaService, VerticalExtentError, VerticalExtentService,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn direction(vector: [f64; 3]) -> MetricDirection {
    MetricDirection::try_new(vector).unwrap()
}

/// A closed, outward-oriented box over `[0, 4] × [0, 3]` whose bottom
/// corners lie at `bottom` and top corners at `top`, each in the order
/// (0,0), (4,0), (4,3), (0,3).
fn block(bottom: [f64; 4], top: [f64; 4]) -> TriMesh {
    let plan = [[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]];
    let mut points: Vec<Point3> = (0..4)
        .map(|i| Point3::new(plan[i][0], plan[i][1], bottom[i]))
        .collect();
    points.extend((0..4).map(|i| Point3::new(plan[i][0], plan[i][1], top[i])));
    let indices = vec![
        0, 2, 1, 0, 3, 2, // bottom, facing down
        4, 5, 6, 4, 6, 7, // top, facing up
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn holds(lower: f64, upper: f64, value: f64) -> bool {
    lower <= value && value <= upper
}

#[test]
fn a_tapered_member_spans_every_local_thickness() {
    // 0.2 m thick at x = 0, 0.4 m at x = 4, on a flat bottom.
    let service = AxiolidVerticalExtentService::new(
        AxiolidGeometry::new().with_mesh(id("taper"), block([0.0; 4], [0.2, 0.4, 0.4, 0.2])),
    );
    let thickness = service
        .measure_thickness(&id("taper"), direction([0.0, 0.0, 1.0]))
        .unwrap();
    let (lower, upper) = (thickness.lower_metres(), thickness.upper_metres());
    assert!(
        holds(lower, upper, 0.2) && holds(lower, upper, 0.4),
        "{thickness:?}"
    );
    assert!(lower > 0.199 && upper < 0.401, "{thickness:?}");
    assert!(!thickness.evidence().exact);
}

#[test]
fn a_sloped_slab_is_as_thick_everywhere_square_to_its_top() {
    // 0.3 m thick vertically, rising 0.4 over 4 m: square to its top it is
    // 0.3 · cos(atan 0.1) thick, and its vertical ends are not crossed.
    let service = AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_mesh(
        id("sloped"),
        block([0.0, 0.4, 0.4, 0.0], [0.3, 0.7, 0.7, 0.3]),
    ));
    let vertical = service
        .measure_thickness(&id("sloped"), direction([0.0, 0.0, 1.0]))
        .unwrap();
    assert!(holds(vertical.lower_metres(), vertical.upper_metres(), 0.3));
    assert!(vertical.upper_metres() - vertical.lower_metres() < 1e-6);
    let square = service
        .measure_thickness(&id("sloped"), direction([-0.1, 0.0, 1.0]))
        .unwrap();
    let expected = 0.3 / 1.01_f64.sqrt();
    assert!(
        holds(square.lower_metres(), square.upper_metres(), expected),
        "{square:?}"
    );
    assert!(square.upper_metres() - square.lower_metres() < 1e-6);
}

#[test]
fn an_open_surface_has_no_thickness() {
    let sheet = TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
        ],
        vec![0, 1, 2],
    );
    let service =
        AxiolidVerticalExtentService::new(AxiolidGeometry::new().with_mesh(id("sheet"), sheet));
    assert!(matches!(
        service.measure_thickness(&id("sheet"), direction([0.0, 0.0, 1.0])),
        Err(VerticalExtentError::Unavailable(reason)) if reason.contains("not closed")
    ));
}

#[test]
fn an_axis_aligned_footprint_measures_its_perimeter_exactly() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("block"), block([0.0; 4], [1.0; 4]))
        .with_tessellated_mesh(id("rough"), block([0.0; 4], [1.0; 4]), 0.001);
    let service = AxiolidPlanAreaService::new(geometry, source());
    let exact = service.measure_footprint_perimeter(&id("block")).unwrap();
    assert_eq!((exact.lower_metres(), exact.upper_metres()), (14.0, 14.0));
    assert!(exact.evidence().exact);
    let rough = service.measure_footprint_perimeter(&id("rough")).unwrap();
    assert!(holds(rough.lower_metres(), rough.upper_metres(), 14.0));
    assert!(!rough.evidence().exact && rough.upper_metres() < 14.01);
}
