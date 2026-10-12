//! Plan containment of footprints in a stated zone over real Axiolid
//! geometry: inside, crossing and outside, the zone's accuracy and an
//! L-shaped zone.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService};
use axioval_engine::{PlanAreaService, PlanPolygon, ZoneAreaRequest};
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

/// A building limit 20 x 15 m; building `inside` 10 x 8 m well within it,
/// `crossing` 10 x 8 m reaching 0.3 m past its east edge, `outside` beyond
/// it, `round` a tessellated box inside it, and `annex` overlapping
/// `inside` by 2 m².
fn service() -> AxiolidPlanAreaService {
    AxiolidPlanAreaService::new(
        AxiolidGeometry::new()
            .with_mesh(id("inside"), cuboid(2.0, 2.0, 12.0, 10.0, 3.0))
            .with_mesh(id("crossing"), cuboid(10.3, 4.0, 20.3, 12.0, 3.0))
            .with_mesh(id("outside"), cuboid(30.0, 0.0, 35.0, 5.0, 3.0))
            .with_mesh(id("annex"), cuboid(11.0, 0.0, 13.0, 4.0, 3.0))
            .with_tessellated_mesh(id("round"), cuboid(4.0, 11.0, 8.0, 14.0, 3.0), 0.01),
        source(),
    )
}

fn limit() -> Vec<PlanPolygon> {
    vec![
        PlanPolygon::try_new(
            vec![[0.0, 0.0], [20.0, 0.0], [20.0, 15.0], [0.0, 15.0]],
            Vec::new(),
        )
        .unwrap(),
    ]
}

/// An area's least and greatest value, in square metres.
type Bounds = (f64, f64);

/// The footprint, inside and outside areas of `objects` in `zone`, and
/// whether the evidence is exact.
fn measured(
    objects: &[&str],
    zone: Vec<PlanPolygon>,
    accuracy: f64,
) -> (Bounds, Bounds, Bounds, bool) {
    let request = ZoneAreaRequest::try_new(
        objects.iter().map(|local| id(local)).collect(),
        zone,
        accuracy,
    )
    .unwrap();
    let area = service().measure_zone_area(&request).unwrap();
    (
        area.footprint_square_metres(),
        area.inside_square_metres(),
        area.outside_square_metres(),
        area.evidence().exact,
    )
}

fn near((lower, upper): (f64, f64), value: f64) -> bool {
    (lower - value).abs() < 1e-9 && (upper - value).abs() < 1e-9
}

#[test]
fn a_footprint_inside_its_zone_has_nothing_outside_exactly() {
    let (footprint, inside, outside, exact) = measured(&["inside"], limit(), 0.0);
    assert!(near(footprint, 80.0) && near(inside, 80.0), "{inside:?}");
    assert_eq!(outside, (0.0, 0.0));
    assert!(exact);
}

#[test]
fn a_building_crossing_its_limit_by_30_cm_has_the_strip_outside() {
    // 0.3 m past the east edge along 8 m: 2.4 m².
    let (_, inside, outside, exact) = measured(&["crossing"], limit(), 0.0);
    assert!(near(outside, 2.4), "{outside:?}");
    assert!(near(inside, 77.6), "{inside:?}");
    assert!(exact);
    let (_, inside, outside, _) = measured(&["outside"], limit(), 0.0);
    assert_eq!(inside, (0.0, 0.0));
    assert!(near(outside, 25.0));
}

#[test]
fn the_zones_accuracy_widens_both_parts() {
    // Stated within 0.5 m, the crossing strip may be from nothing to the
    // 0.8 m strip: never surely inside.
    let (_, inside, outside, exact) = measured(&["crossing"], limit(), 0.5);
    assert!(!exact);
    assert!(outside.0 <= 1e-9, "{outside:?}");
    assert!(outside.1 >= 6.4 - 1e-9 && outside.1 < 6.6, "{outside:?}");
    assert!(
        inside.0 <= 73.6 + 1e-9 && inside.1 >= 80.0 - 1e-9,
        "{inside:?}"
    );
    // Well inside, the accuracy changes nothing.
    let (_, inside, outside, _) = measured(&["inside"], limit(), 0.5);
    assert!(near(inside, 80.0));
    assert_eq!(outside, (0.0, 0.0));
}

#[test]
fn footprints_are_united_and_a_tessellated_one_widens_the_areas() {
    // 80 + 8 - 2 m² of overlap, all inside.
    let (footprint, inside, outside, exact) = measured(&["inside", "annex"], limit(), 0.0);
    assert!(near(footprint, 86.0) && near(inside, 86.0), "{footprint:?}");
    assert_eq!(outside, (0.0, 0.0));
    assert!(exact);
    let (footprint, inside, outside, exact) = measured(&["round"], limit(), 0.0);
    assert!(!exact);
    assert!(footprint.0 < 12.0 && footprint.1 > 12.0);
    assert!(inside.0 < 12.0 && inside.1 > 12.0);
    assert!(outside.0 <= 0.0 && outside.1 > 0.0);
}

#[test]
fn an_l_shaped_zone_with_a_hole_is_measured() {
    // An L: the 20 x 15 m limit less its north-east 10 x 5 m, with a
    // 2 x 2 m hole under `inside`.
    let zone = vec![
        PlanPolygon::try_new(
            vec![
                [0.0, 0.0],
                [20.0, 0.0],
                [20.0, 10.0],
                [10.0, 10.0],
                [10.0, 15.0],
                [0.0, 15.0],
            ],
            vec![vec![[3.0, 3.0], [5.0, 3.0], [5.0, 5.0], [3.0, 5.0]]],
        )
        .unwrap(),
    ];
    let (_, inside, outside, exact) = measured(&["inside"], zone.clone(), 0.0);
    assert!(near(inside, 76.0), "{inside:?}");
    assert!(near(outside, 4.0), "{outside:?}");
    assert!(exact);
    // 10.3..20.3 x 4..12: outside past x = 20 (0.3 x 6 for y 4..10) and
    // above y = 10 for x 10.3..20.3 (10 x 2).
    let (_, _, outside, _) = measured(&["crossing"], zone, 0.0);
    assert!(near(outside, 0.3 * 6.0 + 10.0 * 2.0), "{outside:?}");
}
