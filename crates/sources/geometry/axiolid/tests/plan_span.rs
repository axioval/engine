//! Longest plan diagonals and footprint spans over real Axiolid geometry.

use std::sync::Arc;

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanSpanService};
use axioval_engine::{
    CentrePlacement, PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle,
    RectangleOrientation,
};
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
fn a_rooms_centre_lies_inside_it_and_an_l_shaped_rooms_outside() {
    // An L of 10 m² along x and 9 m² up its west end: its centroid
    // (32.87, 2.87) lies in the notch.
    let geometry = room_with_doors()
        .with_mesh(id("leg-a"), cuboid(30.0, 0.0, 40.0, 1.0, 3.0))
        .with_mesh(id("leg-b"), cuboid(30.0, 1.0, 31.0, 10.0, 3.0))
        .with_group(id("ell"), [id("leg-a"), id("leg-b")]);
    let spans = PlanSpanServiceHandle::new(Arc::new(service(geometry)));
    let room = spans.measure_centre(&id("room")).unwrap();
    assert!(room.is_exact());
    assert_eq!(room.object(), &id("room"));
    assert_eq!(room.placement(), CentrePlacement::Inside);
    let [x, y] = room.point();
    assert!(
        (x - 10.0).abs() < LENGTH && (y - 5.0).abs() < LENGTH,
        "{x} {y}"
    );
    assert!(room.evidence().locator.ends_with(":inside"));

    let ell = spans.measure_centre(&id("ell")).unwrap();
    assert_eq!(ell.placement(), CentrePlacement::Outside);
    // The centre is the one spans between centres measure from.
    let span = spans
        .measure_span(&id("room"), &id("ell"), PlanSpan::Centres)
        .unwrap();
    let [ex, ey] = ell.point();
    assert!(((ex - x).hypot(ey - y) - span.lower_metres()).abs() < LENGTH);
}

#[test]
fn a_tessellated_centre_is_an_approximate_disc() {
    let spans = service(AxiolidGeometry::new().with_tessellated_mesh(
        id("room"),
        cuboid(0.0, 0.0, 4.0, 3.0, 3.0),
        0.01,
    ));
    let centre = spans.measure_centre(&id("room")).unwrap();
    assert!(!centre.is_exact());
    assert!(centre.radius_metres() > 0.0);
    assert_eq!(centre.placement(), CentrePlacement::Inside);
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

/// A closed, outward-oriented prism over a convex counter-clockwise plan
/// polygon, from `z0` to `z1`.
fn prism(corners: &[(f64, f64)], z0: f64, z1: f64) -> TriMesh {
    let n = u32::try_from(corners.len()).unwrap();
    let mut points: Vec<Point3> = corners
        .iter()
        .map(|(x, y)| Point3::new(*x, *y, z0))
        .collect();
    points.extend(corners.iter().map(|(x, y)| Point3::new(*x, *y, z1)));
    let mut indices = Vec::new();
    for i in 1..n - 1 {
        indices.extend([0, i + 1, i]);
        indices.extend([n, n + i, n + i + 1]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend([i, j, n + j, i, n + j, n + i]);
    }
    TriMesh::new(points, indices)
}

/// A 10 × 6 m space with a niche 2 m wide and 1.5 m deep in its south side.
fn niche() -> TriMesh {
    axiolid_mesh::compose(&[
        cuboid(0.0, 0.0, 4.0, 6.0, 3.0),
        cuboid(6.0, 0.0, 10.0, 6.0, 3.0),
        cuboid(4.0, 1.5, 6.0, 6.0, 3.0),
    ])
}

#[test]
fn a_niche_is_a_recess_as_wide_as_its_mouth_and_as_deep_as_its_back() {
    let spans = service(AxiolidGeometry::new().with_mesh(id("room"), niche()));
    let found = spans.measure_recesses(&id("room")).unwrap();
    assert_eq!(found.object(), &id("room"));
    assert_eq!(found.recesses().len(), 1, "{found:?}");
    let recess = &found.recesses()[0];
    assert!(recess.width().is_exact() && recess.depth().is_exact());
    assert!((recess.width().lower_metres() - 2.0).abs() < LENGTH);
    assert!((recess.depth().lower_metres() - 1.5).abs() < LENGTH);
    let mut mouth = recess.mouth();
    mouth.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert!((mouth[0][0] - 4.0).abs() < LENGTH && mouth[0][1].abs() < LENGTH);
    assert!((mouth[1][0] - 6.0).abs() < LENGTH && mouth[1][1].abs() < LENGTH);
}

#[test]
fn a_convex_space_has_no_recess_and_an_l_shape_one_across_its_corner() {
    let spans = service(
        AxiolidGeometry::new()
            .with_mesh(id("room"), cuboid(0.0, 0.0, 4.0, 3.0, 3.0))
            .with_mesh(
                id("ell"),
                axiolid_mesh::compose(&[
                    cuboid(0.0, 0.0, 6.0, 2.0, 3.0),
                    cuboid(0.0, 2.0, 2.0, 5.0, 3.0),
                ]),
            ),
    );
    assert!(
        spans
            .measure_recesses(&id("room"))
            .unwrap()
            .recesses()
            .is_empty()
    );
    let ell = spans.measure_recesses(&id("ell")).unwrap();
    assert_eq!(ell.recesses().len(), 1);
    // The mouth joins (6, 2) and (2, 5): 5 m; the inner corner (2, 2) lies
    // 12 / 5 m behind it.
    let recess = &ell.recesses()[0];
    assert!((recess.width().lower_metres() - 5.0).abs() < LENGTH);
    assert!((recess.depth().lower_metres() - 2.4).abs() < LENGTH);
}

#[test]
fn a_tessellated_or_bodiless_space_has_no_measured_recesses() {
    let spans = service(
        AxiolidGeometry::new()
            .with_tessellated_mesh(id("curved"), niche(), 0.01)
            .with_no_body(id("zone")),
    );
    assert!(matches!(
        spans.measure_recesses(&id("curved")),
        Err(PlanSpanError::Unavailable(_))
    ));
    assert!(matches!(
        spans.measure_recesses(&id("zone")),
        Err(PlanSpanError::Unavailable(_))
    ));
}

/// Three spaces stacked into a shaft: the clear section is what all three
/// share.
#[test]
fn a_stack_shares_the_intersection_of_its_footprints() {
    let spans = service(
        AxiolidGeometry::new()
            .with_mesh(id("ground"), cuboid(0.0, 0.0, 4.0, 3.0, 3.0))
            .with_mesh(id("first"), cuboid(1.0, 0.0, 5.0, 3.0, 3.0))
            .with_mesh(id("second"), cuboid(1.0, 0.5, 4.0, 3.5, 3.0))
            .with_mesh(id("apart"), cuboid(10.0, 0.0, 12.0, 2.0, 3.0)),
    );
    let section = spans
        .measure_section(&[id("ground"), id("first"), id("second")])
        .unwrap();
    assert!(section.evidence().exact);
    assert!((section.area_lower() - 7.5).abs() < 1e-6);
    let width = section.width().unwrap();
    let length = section.length().unwrap();
    assert!((width.lower_metres() - 2.5).abs() < LENGTH, "{width:?}");
    assert!((length.upper_metres() - 3.0).abs() < LENGTH, "{length:?}");

    let empty = spans.measure_section(&[id("ground"), id("apart")]).unwrap();
    assert!(empty.area_upper() == 0.0 && empty.width().is_none());
}

/// The width is the short side of the minimum-area rectangle, whatever the
/// section's orientation.
#[test]
fn a_rotated_section_is_as_wide_as_its_short_side() {
    let (c, s) = (30.0_f64.to_radians().cos(), 30.0_f64.to_radians().sin());
    let corner = |x: f64, y: f64| (x * c - y * s, x * s + y * c);
    let rotated = prism(
        &[
            corner(0.0, 0.0),
            corner(4.0, 0.0),
            corner(4.0, 2.0),
            corner(0.0, 2.0),
        ],
        0.0,
        3.0,
    );
    let spans = service(AxiolidGeometry::new().with_mesh(id("shaft"), rotated));
    let section = spans.measure_section(&[id("shaft")]).unwrap();
    let width = section.width().unwrap();
    assert!(width.lower_metres() <= 2.0 + LENGTH && width.upper_metres() >= 2.0 - LENGTH);
    assert!(width.upper_metres() - width.lower_metres() < 1e-9);
    assert!((section.area_lower() - 8.0).abs() < 1e-6);
}

/// A hexagon whose least-area rectangles tie: 4 × 4 along the axes and
/// `8/√2 × 4/√2` along the diagonals. The widths differ, so the section has
/// no known width and is refused, as its footprint's rectangle is tied.
#[test]
fn a_section_whose_least_area_rectangles_tie_is_refused() {
    let hexagon = prism(
        &[
            (2.0, 0.0),
            (2.0, 2.0),
            (0.0, 2.0),
            (-2.0, 0.0),
            (-2.0, -2.0),
            (0.0, -2.0),
        ],
        0.0,
        3.0,
    );
    let spans = service(AxiolidGeometry::new().with_mesh(id("shaft"), hexagon));
    assert_eq!(
        spans.measure_rectangle(&id("shaft")).unwrap().orientation(),
        RectangleOrientation::Tied
    );
    assert!(matches!(
        spans.measure_section(&[id("shaft")]),
        Err(PlanSpanError::Unavailable(_))
    ));
}

#[test]
fn a_section_through_a_curved_or_bodiless_space_is_refused() {
    let spans = service(
        AxiolidGeometry::new()
            .with_mesh(id("ground"), cuboid(0.0, 0.0, 4.0, 3.0, 3.0))
            .with_tessellated_mesh(id("curved"), cuboid(0.0, 0.0, 4.0, 3.0, 3.0), 0.01)
            .with_no_body(id("zone")),
    );
    for other in ["curved", "zone"] {
        assert!(matches!(
            spans.measure_section(&[id("ground"), id(other)]),
            Err(PlanSpanError::Unavailable(_))
        ));
    }
}
