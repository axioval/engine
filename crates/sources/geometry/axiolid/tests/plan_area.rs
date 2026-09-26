//! Plan footprint and overlap areas over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService};
use axioval_engine::{PlanAreaError, PlanAreaService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed axis-aligned box: both its floor and ceiling project onto the
/// same footprint, which must count once.
fn cuboid(x0: f64, y0: f64, x1: f64, y1: f64, height: f64) -> TriMesh {
    let points = vec![
        Point3::new(x0, y0, 0.0),
        Point3::new(x1, y0, 0.0),
        Point3::new(x1, y1, 0.0),
        Point3::new(x0, y1, 0.0),
        Point3::new(x0, y0, height),
        Point3::new(x1, y0, height),
        Point3::new(x1, y1, height),
        Point3::new(x0, y1, height),
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

fn service(geometry: AxiolidGeometry) -> AxiolidPlanAreaService {
    AxiolidPlanAreaService::new(geometry, source())
}

#[test]
fn a_closed_solid_counts_its_footprint_once() {
    let areas =
        service(AxiolidGeometry::new().with_mesh(id("room"), cuboid(0.0, 0.0, 4.0, 3.0, 2.5)));
    let area = areas.measure_footprint(&id("room")).unwrap();
    assert!(area.is_exact());
    assert!((area.lower_square_metres() - 12.0).abs() < 1e-9);
    assert_eq!(area.evidence().locator, format!("footprint:{}", id("room")));
}

#[test]
fn an_overlap_is_the_shared_part_of_two_footprints() {
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("space"), cuboid(0.0, 0.0, 4.0, 3.0, 2.5))
            .with_mesh(id("zone"), cuboid(1.0, 0.0, 10.0, 10.0, 3.0))
            .with_mesh(id("far"), cuboid(20.0, 20.0, 21.0, 21.0, 3.0)),
    );
    let overlap = areas
        .measure_plan_overlap(&id("space"), &id("zone"))
        .unwrap();
    assert!((overlap.lower_square_metres() - 9.0).abs() < 1e-9);
    let none = areas
        .measure_plan_overlap(&id("space"), &id("far"))
        .unwrap();
    assert!(none.upper_square_metres().abs() < 1e-12);
}

#[test]
fn a_tessellated_footprint_is_an_interval_around_the_measured_area() {
    let areas = service(AxiolidGeometry::new().with_tessellated_mesh(
        id("round"),
        cuboid(0.0, 0.0, 4.0, 3.0, 2.5),
        0.01,
    ));
    let area = areas.measure_footprint(&id("round")).unwrap();
    assert!(!area.is_exact());
    // Perimeter 14 m, deviation 1 cm: 2 * 14 * 0.01 + pi * 0.0001.
    let band = 0.28 + std::f64::consts::PI * 1e-4;
    assert!((area.lower_square_metres() - (12.0 - band)).abs() < 1e-9);
    assert!((area.upper_square_metres() - (12.0 + band)).abs() < 1e-9);
}

#[test]
fn an_object_without_geometry_is_unknown_not_zero() {
    let areas = service(AxiolidGeometry::new());
    assert_eq!(
        areas.measure_footprint(&id("ghost")),
        Err(PlanAreaError::UnknownObject(id("ghost")))
    );
}
