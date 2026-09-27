//! Longest plan diagonals and footprint spans over real Axiolid geometry.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanSpanService};
use axioval_engine::{PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented axis-aligned box.
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

fn service(geometry: AxiolidGeometry) -> AxiolidPlanSpanService {
    AxiolidPlanSpanService::new(geometry, source())
}

// The overlay snaps to a grid (axiolid/kernel#173), so centroids compare
// within 1e-6 m.
const LENGTH: f64 = 1e-6;

/// A 20 x 10 m room with two 1 m doors in its north wall, at x 1..2 and 5..6.
fn room_with_doors() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid(0.0, 0.0, 20.0, 10.0, 3.0))
        .with_mesh(id("west"), cuboid(1.0, 10.0, 2.0, 10.1, 2.1))
        .with_mesh(id("east"), cuboid(5.0, 10.0, 6.0, 10.1, 2.1))
}

#[test]
fn a_rooms_longest_plan_diagonal_joins_opposite_corners() {
    let spans = service(room_with_doors());
    let diameter = spans.measure_diameter(&id("room")).unwrap();
    assert!(diameter.is_exact());
    assert!((diameter.lower_metres() - 500.0_f64.sqrt()).abs() < 1e-12);
    assert_eq!(
        diameter.evidence().locator,
        format!("plan-diameter:{}", id("room"))
    );
}

#[test]
fn doors_are_measured_between_centres_or_farthest_points() {
    let spans = service(room_with_doors());
    let centres = spans
        .measure_span(&id("west"), &id("east"), PlanSpan::Centres)
        .unwrap();
    assert!(centres.is_exact());
    assert!((centres.lower_metres() - 4.0).abs() < LENGTH);
    let farthest = spans
        .measure_span(&id("west"), &id("east"), PlanSpan::Farthest)
        .unwrap();
    assert!(farthest.is_exact());
    // From (1, 10) to (6, 10.1).
    assert!((farthest.lower_metres() - 25.01_f64.sqrt()).abs() < 1e-12);
    assert_eq!(
        farthest.evidence().locator,
        format!("plan-span:farthest:{}:{}", id("west"), id("east"))
    );
}

#[test]
fn a_non_convex_group_spans_its_union() {
    // An L: 6 x 2 m along x, and 2 x 3 m on top of its west end.
    let spans = service(
        AxiolidGeometry::new()
            .with_mesh(id("long"), cuboid(0.0, 0.0, 6.0, 2.0, 3.0))
            .with_mesh(id("short"), cuboid(0.0, 2.0, 2.0, 5.0, 3.0))
            .with_group(id("ell"), [id("long"), id("short")])
            .with_mesh(id("post"), cuboid(9.5, 0.5, 10.5, 1.5, 3.0)),
    );
    let diameter = spans.measure_diameter(&id("ell")).unwrap();
    // (6, 0) to (0, 5).
    assert!((diameter.lower_metres() - 61.0_f64.sqrt()).abs() < 1e-12);
    // Centroid (7/3, 11/6): 12 m² at (3, 1) and 6 m² at (1, 3.5).
    let centres = spans
        .measure_span(&id("ell"), &id("post"), PlanSpan::Centres)
        .unwrap();
    let expected = (10.0_f64 - 7.0 / 3.0).hypot(1.0 - 11.0 / 6.0);
    assert!((centres.lower_metres() - expected).abs() < LENGTH);
}

#[test]
fn tessellated_spans_are_intervals_around_the_measurement() {
    let deviation = 0.01;
    let spans = service(
        AxiolidGeometry::new()
            .with_tessellated_mesh(id("room"), cuboid(0.0, 0.0, 4.0, 3.0, 3.0), deviation)
            .with_mesh(id("door"), cuboid(10.0, 1.0, 11.0, 2.0, 2.0)),
    );
    let diameter = spans.measure_diameter(&id("room")).unwrap();
    assert!(!diameter.is_exact());
    assert!((diameter.lower_metres() - (5.0 - 2.0 * deviation)).abs() < 1e-12);
    assert!((diameter.upper_metres() - (5.0 + 2.0 * deviation)).abs() < 1e-12);

    let farthest = spans
        .measure_span(&id("room"), &id("door"), PlanSpan::Farthest)
        .unwrap();
    assert!(!farthest.is_exact());
    let far = 11.0_f64.hypot(2.0);
    assert!((farthest.lower_metres() - (far - deviation)).abs() < 1e-12);
    assert!((farthest.upper_metres() - (far + deviation)).abs() < 1e-12);

    // Area 12, perimeter 14, farthest corner 2.5 m from the centre (2, 1.5).
    let centres = spans
        .measure_span(&id("room"), &id("door"), PlanSpan::Centres)
        .unwrap();
    let band = 2.0 * 14.0 * deviation + std::f64::consts::PI * deviation * deviation;
    let slack = band * (2.5 + deviation) / (12.0 - band);
    let measured = 8.5_f64.hypot(0.0);
    assert!(!centres.is_exact());
    assert!(centres.lower_metres() < measured && measured < centres.upper_metres());
    assert!((centres.lower_metres() - (measured - slack)).abs() < LENGTH);
    assert!((centres.upper_metres() - (measured + slack)).abs() < LENGTH);
}

#[test]
fn a_footprint_too_small_for_its_deviation_has_no_centre() {
    let spans = service(
        AxiolidGeometry::new()
            .with_tessellated_mesh(id("rod"), cuboid(0.0, 0.0, 0.1, 0.1, 3.0), 0.05)
            .with_mesh(id("door"), cuboid(10.0, 1.0, 11.0, 2.0, 2.0)),
    );
    assert!(matches!(
        spans.measure_span(&id("rod"), &id("door"), PlanSpan::Centres),
        Err(PlanSpanError::Unavailable(message)) if message.contains("too small")
    ));
    // Its farthest points are still bounded.
    assert!(
        spans
            .measure_span(&id("rod"), &id("door"), PlanSpan::Farthest)
            .is_ok()
    );
}

#[test]
fn objects_without_a_measured_footprint_are_refused_never_zero() {
    let spans = service(
        room_with_doors()
            .with_no_body(id("storey"))
            .with_unmeasured(id("slab"), "meshing failed"),
    );
    assert!(matches!(
        spans.measure_diameter(&id("storey")),
        Err(PlanSpanError::Unavailable(message)) if message.contains("no footprint")
    ));
    assert!(matches!(
        spans.measure_span(&id("west"), &id("slab"), PlanSpan::Farthest),
        Err(PlanSpanError::Unavailable(message)) if message.contains("meshing failed")
    ));
    assert_eq!(
        spans.measure_diameter(&id("ghost")),
        Err(PlanSpanError::UnknownObject(id("ghost")))
    );
}

#[test]
fn the_handle_refuses_one_object_twice() {
    let handle = PlanSpanServiceHandle::new(Arc::new(service(room_with_doors())));
    assert!(matches!(
        handle.measure_span(&id("west"), &id("west"), PlanSpan::Centres),
        Err(PlanSpanError::Unavailable(_))
    ));
    assert!(
        handle
            .measure_span(&id("west"), &id("east"), PlanSpan::Centres)
            .is_ok()
    );
}
