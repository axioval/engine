//! Uncovered elevation areas over real Axiolid geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService};
use axioval_engine::{ElevationRequest, PlanAreaService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed prism over a counter-clockwise plan quadrilateral.
fn prism(plan: [(f64, f64); 4], bottom: f64, top: f64) -> TriMesh {
    let mut points: Vec<Point3> = plan
        .iter()
        .map(|&(x, y)| Point3::new(x, y, bottom))
        .collect();
    points.extend(plan.iter().map(|&(x, y)| Point3::new(x, y, top)));
    let indices = vec![
        0, 2, 1, 0, 3, 2, // floor
        4, 5, 6, 4, 6, 7, // ceiling
        0, 1, 5, 0, 5, 4, // sides
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

fn cuboid(x0: f64, y0: f64, x1: f64, y1: f64, bottom: f64, top: f64) -> TriMesh {
    prism([(x0, y0), (x1, y0), (x1, y1), (x0, y1)], bottom, top)
}

/// A 4 m wall along x, 0.2 m thick and 3 m high, with structure around it.
fn walls() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("wall"), cuboid(0.0, 0.0, 4.0, 0.2, 0.0, 3.0))
        .with_mesh(id("full"), cuboid(0.0, 0.0, 2.0, 0.2, 0.0, 3.0))
        .with_mesh(id("half"), cuboid(2.0, 0.0, 4.0, 0.2, 0.0, 1.5))
        .with_mesh(id("thick"), cuboid(0.0, -0.1, 4.0, 0.3, 0.0, 3.0))
        .with_mesh(id("far"), cuboid(0.0, 5.0, 4.0, 5.2, 0.0, 3.0))
        .with_mesh(id("left"), cuboid(-0.3, 0.0, 0.0, 0.2, 0.0, 3.0))
        .with_mesh(id("right"), cuboid(4.0, 0.0, 4.3, 0.2, 0.0, 3.0))
        .with_mesh(id("beam"), cuboid(-0.3, 0.0, 4.3, 0.2, 3.0, 3.4))
}

fn service(geometry: AxiolidGeometry) -> AxiolidPlanAreaService {
    AxiolidPlanAreaService::new(geometry, source())
}

fn uncovered(
    areas: &AxiolidPlanAreaService,
    axis: [f64; 2],
    cover: &[&str],
    frame: &[&str],
    growth: (f64, f64),
) -> ((f64, f64), (f64, f64), bool) {
    let ids = |names: &[&str]| names.iter().map(|name| id(name)).collect::<Vec<_>>();
    let request = ElevationRequest::try_new(
        id("wall"),
        axis,
        &ids(cover),
        &ids(frame),
        growth.0,
        growth.1,
    )
    .unwrap();
    let answer = areas.measure_elevation_cover(&request).unwrap();
    (
        answer.area_square_metres(),
        answer.uncovered_square_metres(),
        answer.is_exact(),
    )
}

fn is(measured: (f64, f64), value: f64) -> bool {
    (measured.0 - value).abs() < 1e-7 && (measured.1 - value).abs() < 1e-7
}

#[test]
fn a_full_and_a_half_height_counterpart_leave_a_quarter_of_the_face_uncovered() {
    let areas = service(walls());
    let (area, uncovered, exact) =
        uncovered(&areas, [1.0, 0.0], &["full", "half"], &[], (0.0, 0.0));
    assert!(exact);
    assert!(is(area, 12.0), "{area:?}");
    assert!(is(uncovered, 3.0), "{uncovered:?}");
    // Grown 0.5 m in height, the half-height one reaches 2 m.
    let (_, uncovered, _) = uncovered_with(&areas, (0.0, 0.5));
    assert!(is(uncovered, 2.0), "{uncovered:?}");
}

fn uncovered_with(
    areas: &AxiolidPlanAreaService,
    growth: (f64, f64),
) -> ((f64, f64), (f64, f64), bool) {
    uncovered(areas, [1.0, 0.0], &["full", "half"], &[], growth)
}

#[test]
fn a_thicker_counterpart_covers_by_its_cross_section_and_a_far_one_not_at_all() {
    let areas = service(walls());
    let (_, covered, exact) = uncovered(&areas, [1.0, 0.0], &["thick"], &[], (0.0, 0.0));
    assert!(exact);
    assert!(is(covered, 0.0), "{covered:?}");
    // Parallel but 4.8 m away across the axis: outside the wall's depth.
    let (_, open, _) = uncovered(&areas, [1.0, 0.0], &["far"], &[], (0.1, 0.1));
    assert!(is(open, 12.0), "{open:?}");
    // Along the other axis the wall's face is its 0.2 m end.
    let (area, _, _) = uncovered(&areas, [0.0, 1.0], &[], &[], (0.0, 0.0));
    assert!(is(area, 0.6), "{area:?}");
}

#[test]
fn a_frame_covers_the_bay_it_encloses() {
    let areas = service(walls());
    // As bodies the columns and the beam stand beside and above the wall.
    let (_, open, _) = uncovered(
        &areas,
        [1.0, 0.0],
        &["left", "right", "beam"],
        &[],
        (0.0, 0.0),
    );
    assert!(is(open, 12.0), "{open:?}");
    let (_, filled, exact) = uncovered(
        &areas,
        [1.0, 0.0],
        &[],
        &["left", "right", "beam"],
        (0.0, 0.0),
    );
    assert!(exact);
    assert!(is(filled, 0.0), "{filled:?}");
}

#[test]
fn an_axis_off_the_coordinate_axes_measures_within_rounding() {
    let (c, s) = (30.0_f64.to_radians().cos(), 30.0_f64.to_radians().sin());
    let at = |along: f64, across: f64| (along * c - across * s, along * s + across * c);
    let rotated = |a0: f64, a1: f64, bottom: f64, top: f64| {
        prism(
            [at(a0, 0.0), at(a1, 0.0), at(a1, 0.2), at(a0, 0.2)],
            bottom,
            top,
        )
    };
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("wall"), rotated(0.0, 4.0, 0.0, 3.0))
            .with_mesh(id("full"), rotated(0.0, 2.0, 0.0, 3.0))
            .with_mesh(id("half"), rotated(2.0, 4.0, 0.0, 1.5)),
    );
    let (area, uncovered, exact) = uncovered(&areas, [c, s], &["full", "half"], &[], (0.01, 0.0));
    assert!(!exact, "a rotated projection rounds");
    assert!(area.0 <= 12.0 && 12.0 <= area.1, "{area:?}");
    assert!(
        uncovered.0 <= 2.985 && 2.985 <= uncovered.1,
        "{uncovered:?}"
    );
    assert!(uncovered.1 - uncovered.0 < 1e-3, "{uncovered:?}");
}

#[test]
fn a_tessellated_cover_widens_the_uncovered_area() {
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("wall"), cuboid(0.0, 0.0, 4.0, 0.2, 0.0, 3.0))
            .with_tessellated_mesh(id("round"), cuboid(0.0, 0.0, 2.0, 0.2, 0.0, 3.0), 0.01),
    );
    let (area, uncovered, exact) = uncovered(&areas, [1.0, 0.0], &["round"], &[], (0.0, 0.0));
    assert!(!exact);
    assert!(is(area, 12.0), "the wall is exact: {area:?}");
    assert!(uncovered.0 < 6.0 && 6.0 < uncovered.1, "{uncovered:?}");
    assert!(uncovered.1 - uncovered.0 < 0.5, "{uncovered:?}");
}
