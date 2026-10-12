//! Setback areas in front of walls over real Axiolid geometry: a wall's
//! footprint swept along its outward directions, the part outside a
//! parcel within its accuracy, and the overlap of two buildings' setbacks.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService};
use axioval_engine::{
    PlanAreaError, PlanAreaService, PlanPolygon, SetbackDirection, SetbackOther, SetbackRequest,
    SetbackStrip,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed axis-aligned box.
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
        0, 2, 1, 0, 3, 2, // floor
        4, 5, 6, 4, 6, 7, // ceiling
        0, 1, 5, 0, 5, 4, //
        1, 2, 6, 1, 6, 5, //
        2, 3, 7, 2, 7, 6, //
        3, 0, 4, 3, 4, 7,
    ];
    TriMesh::new(points, indices)
}

/// Wall `south` (x 0..10, y 0..0.3) of one building, wall `facing` (x
/// 4..14, y -8.3..-8) of a second one 8 m south of it, looking north, a
/// tessellated wall `round` and an L-shaped wall `corner` (x 20..30 along
/// y 0..0.3, then up x 29.7..30 to y 10).
fn service() -> AxiolidPlanAreaService {
    AxiolidPlanAreaService::new(
        AxiolidGeometry::new()
            .with_mesh(id("south"), cuboid(0.0, 0.0, 10.0, 0.3, 6.0))
            .with_mesh(id("facing"), cuboid(4.0, -8.3, 14.0, -8.0, 6.0))
            .with_mesh(id("corner-a"), cuboid(20.0, 0.0, 30.0, 0.3, 3.0))
            .with_mesh(id("corner-b"), cuboid(29.7, 0.3, 30.0, 10.0, 3.0))
            .with_group(id("corner"), vec![id("corner-a"), id("corner-b")])
            .with_tessellated_mesh(id("round"), cuboid(40.0, 0.0, 50.0, 0.3, 3.0), 0.01),
        source(),
    )
}

fn direction(x: f64, y: f64, sure: bool) -> SetbackDirection {
    SetbackDirection::try_new([x, y], sure).unwrap()
}

fn strip(wall: &str, directions: Vec<SetbackDirection>, depth: (f64, f64)) -> SetbackStrip {
    SetbackStrip::try_new(id(wall), directions, depth).unwrap()
}

/// The parcel x -5..15, y -5..5.
fn parcel() -> Vec<PlanPolygon> {
    vec![
        PlanPolygon::try_new(
            vec![[-5.0, -5.0], [15.0, -5.0], [15.0, 5.0], [-5.0, 5.0]],
            Vec::new(),
        )
        .unwrap(),
    ]
}

fn close((lower, upper): (f64, f64), expected: f64) -> bool {
    (lower - expected).abs() < 1e-6 && (upper - expected).abs() < 1e-6
}

#[test]
fn a_wall_sweeps_its_setback_in_front_of_its_outer_face() {
    let request = SetbackRequest::new(strip("south", vec![direction(0.0, -1.0, true)], (3.0, 3.0)));
    let setback = service().measure_setback(&request).unwrap();
    // 10 m along the face, 3 m deep, nothing behind the wall.
    assert!(close(setback.area_square_metres(), 30.0), "{setback:?}");
    assert!(setback.evidence().exact);
    let polygon = &setback.most_polygons()[0];
    let (low, high) = polygon.outer().iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(low, high), [x, y]| {
            (
                [low[0].min(*x), low[1].min(*y)],
                [high[0].max(*x), high[1].max(*y)],
            )
        },
    );
    assert!(
        (low[1] + 3.0).abs() < 1e-9 && high[1].abs() < 1e-9,
        "{polygon:?}"
    );
    assert!(
        low[0].abs() < 1e-9 && (high[0] - 10.0).abs() < 1e-9,
        "{polygon:?}"
    );
    assert_eq!(setback.least_polygons(), setback.most_polygons());
}

#[test]
fn a_depth_known_to_an_interval_brackets_the_area() {
    let request = SetbackRequest::new(strip(
        "south",
        vec![direction(0.0, -1.0, true), direction(-1.0, 0.0, false)],
        (2.9, 3.1),
    ));
    let setback = service().measure_setback(&request).unwrap();
    let (lower, upper) = setback.area_square_metres();
    // The least sweeps only the sure face by 2.9 m; the most adds the end
    // face's 0.3 m by 3.1 m.
    assert!((lower - 29.0).abs() < 1e-6, "{lower}");
    assert!((upper - (31.0 + 0.93)).abs() < 1e-6, "{upper}");
    assert!(!setback.evidence().exact);
}

#[test]
fn the_part_of_a_setback_beyond_its_parcel_is_measured_within_the_accuracy() {
    // 6 m deep: 5 m of it on the parcel, 1 m by 10 m beyond its edge.
    let deep = strip("south", vec![direction(0.0, -1.0, true)], (6.0, 6.0));
    let setback = service()
        .measure_setback(
            &SetbackRequest::new(deep.clone())
                .with_zone(parcel(), 0.0)
                .unwrap(),
        )
        .unwrap();
    assert!(
        close(setback.outside_square_metres().unwrap(), 10.0),
        "{setback:?}"
    );
    assert!(setback.evidence().exact);
    // 3 m deep lies on the parcel.
    let shallow = strip("south", vec![direction(0.0, -1.0, true)], (3.0, 3.0));
    let setback = service()
        .measure_setback(
            &SetbackRequest::new(shallow)
                .with_zone(parcel(), 0.0)
                .unwrap(),
        )
        .unwrap();
    assert!(
        close(setback.outside_square_metres().unwrap(), 0.0),
        "{setback:?}"
    );
    // A setback reaching the boundary within its accuracy is never surely
    // inside.
    let edge = strip("south", vec![direction(0.0, -1.0, true)], (5.0, 5.0));
    let setback = service()
        .measure_setback(&SetbackRequest::new(edge).with_zone(parcel(), 0.1).unwrap())
        .unwrap();
    let (lower, upper) = setback.outside_square_metres().unwrap();
    assert!(lower <= 0.0 && upper >= 1.0 - 1e-9, "{lower} {upper}");
    assert!(!setback.evidence().exact);
}

#[test]
fn overlapping_setbacks_of_two_buildings_are_measured_and_named() {
    // South's 5 m and facing's 5 m meet 2 m deep over x 4..10.
    let subject = strip("south", vec![direction(0.0, -1.0, true)], (5.0, 5.0));
    let other = strip("facing", vec![direction(0.0, 1.0, true)], (5.0, 5.0));
    let request = SetbackRequest::new(subject.clone())
        .with_others(vec![SetbackOther::new(other.clone(), true)])
        .unwrap();
    let setback = service().measure_setback(&request).unwrap();
    assert!(
        close(setback.overlap_square_metres().unwrap(), 12.0),
        "{setback:?}"
    );
    assert_eq!(setback.overlapping(), [id("facing")]);
    // An other the selection leaves undecided overlaps only possibly.
    let request = SetbackRequest::new(subject.clone())
        .with_others(vec![SetbackOther::new(other, false)])
        .unwrap();
    let (lower, upper) = service()
        .measure_setback(&request)
        .unwrap()
        .overlap_square_metres()
        .unwrap();
    assert!(
        lower.abs() < 1e-9 && (upper - 12.0).abs() < 1e-6,
        "{lower} {upper}"
    );
    // 3 m each leave a 2 m gap: no overlap, nothing named.
    let request = SetbackRequest::new(strip("south", vec![direction(0.0, -1.0, true)], (3.0, 3.0)))
        .with_others(vec![SetbackOther::new(
            strip("facing", vec![direction(0.0, 1.0, true)], (3.0, 3.0)),
            true,
        )])
        .unwrap();
    let setback = service().measure_setback(&request).unwrap();
    assert!(close(setback.overlap_square_metres().unwrap(), 0.0));
    assert!(setback.overlapping().is_empty());
}

#[test]
fn an_l_shaped_wall_sweeps_each_of_its_directions() {
    let request = SetbackRequest::new(strip(
        "corner",
        vec![direction(0.0, -1.0, true), direction(1.0, 0.0, true)],
        (2.0, 2.0),
    ));
    let setback = service().measure_setback(&request);
    // A declared group's footprint is its members'.
    let setback = setback.unwrap();
    // 10 m south and 10 m east, 2 m deep, and the 2 x 2 m corner between
    // them belongs to neither sweep.
    assert!(close(setback.area_square_metres(), 40.0), "{setback:?}");
}

#[test]
fn a_tessellated_wall_widens_its_setback_and_is_inexact() {
    let request = SetbackRequest::new(strip("round", vec![direction(0.0, -1.0, true)], (3.0, 3.0)));
    let setback = service().measure_setback(&request).unwrap();
    let (lower, upper) = setback.area_square_metres();
    assert!(lower < 30.0 && upper > 30.0, "{lower} {upper}");
    assert!(!setback.evidence().exact);
}

#[test]
fn a_wall_without_a_body_has_no_setback() {
    let request = SetbackRequest::new(strip("ghost", vec![direction(0.0, -1.0, true)], (3.0, 3.0)));
    assert!(matches!(
        service().measure_setback(&request),
        Err(PlanAreaError::UnknownObject(_))
    ));
}
