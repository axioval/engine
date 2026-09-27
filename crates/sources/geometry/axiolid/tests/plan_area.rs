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
fn a_bodiless_object_covers_nothing_but_an_unmeasured_one_is_unknown() {
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("space"), cuboid(0.0, 0.0, 4.0, 3.0, 2.5))
            .with_no_body(id("zone"))
            .with_unmeasured(id("slab"), "meshing failed"),
    );
    let footprint = areas.measure_footprint(&id("zone")).unwrap();
    assert!(footprint.is_exact());
    assert!(footprint.upper_square_metres().abs() < f64::EPSILON);
    let overlap = areas
        .measure_plan_overlap(&id("space"), &id("zone"))
        .unwrap();
    assert!(overlap.upper_square_metres().abs() < f64::EPSILON);
    assert!(matches!(
        areas.measure_footprint(&id("slab")),
        Err(PlanAreaError::Unavailable(message)) if message.contains("meshing failed")
    ));
}

#[test]
fn an_object_without_geometry_is_unknown_not_zero() {
    let areas = service(AxiolidGeometry::new());
    assert_eq!(
        areas.measure_footprint(&id("ghost")),
        Err(PlanAreaError::UnknownObject(id("ghost")))
    );
}

// The overlay snaps to a grid (axiolid/kernel#173), so group areas compare
// within 1e-6 m².
const AREA: f64 = 1e-6;

#[test]
fn a_group_covers_the_union_of_its_members() {
    // Two rooms sharing a 1 m strip: 12 + 12 - 3, the strip counted once.
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("a"), cuboid(0.0, 0.0, 4.0, 3.0, 2.5))
            .with_mesh(id("b"), cuboid(3.0, 0.0, 7.0, 3.0, 2.5))
            .with_mesh(id("space"), cuboid(2.0, 0.0, 6.0, 3.0, 2.5))
            .with_group(id("zone"), [id("a"), id("b")]),
    );
    let footprint = areas.measure_footprint(&id("zone")).unwrap();
    assert!(footprint.is_exact());
    assert!((footprint.lower_square_metres() - 21.0).abs() < AREA);
    assert!((footprint.upper_square_metres() - 21.0).abs() < AREA);
    // The space straddles both members and lies wholly within the zone,
    // though neither member alone holds it.
    let overlap = areas
        .measure_plan_overlap(&id("space"), &id("zone"))
        .unwrap();
    assert!(overlap.is_exact());
    assert!((overlap.lower_square_metres() - 12.0).abs() < AREA);
    // A group is bodiless for every other purpose.
    assert!(
        AxiolidGeometry::new()
            .with_group(id("zone"), [id("a")])
            .has_no_body(&id("zone"))
    );
}

#[test]
fn a_group_of_groups_unions_every_member() {
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("a"), cuboid(0.0, 0.0, 4.0, 3.0, 2.5))
            .with_mesh(id("b"), cuboid(10.0, 0.0, 12.0, 3.0, 2.5))
            .with_group(id("inner"), [id("b")])
            .with_group(id("outer"), [id("a"), id("inner")]),
    );
    let footprint = areas.measure_footprint(&id("outer")).unwrap();
    assert!((footprint.lower_square_metres() - 18.0).abs() < AREA);
}

#[test]
fn a_tessellated_member_makes_the_group_inexact() {
    let areas = service(
        AxiolidGeometry::new()
            .with_mesh(id("a"), cuboid(0.0, 0.0, 4.0, 3.0, 2.5))
            .with_tessellated_mesh(id("b"), cuboid(4.0, 0.0, 8.0, 3.0, 2.5), 0.01)
            .with_group(id("zone"), [id("a"), id("b")]),
    );
    let footprint = areas.measure_footprint(&id("zone")).unwrap();
    assert!(!footprint.is_exact());
    // The union is 8 x 3 with perimeter 22 m; the largest deviation is 1 cm.
    let band = 2.0 * 22.0 * 0.01 + std::f64::consts::PI * 1e-4;
    assert!((footprint.lower_square_metres() - (24.0 - band)).abs() < AREA);
    assert!((footprint.upper_square_metres() - (24.0 + band)).abs() < AREA);
}

#[test]
fn a_group_refuses_when_a_member_cannot_give_it_a_footprint() {
    let room = || cuboid(0.0, 0.0, 4.0, 3.0, 2.5);
    let unavailable = |geometry: AxiolidGeometry, needle: &str| {
        let areas = service(geometry.with_mesh(id("space"), room()));
        for result in [
            areas.measure_footprint(&id("zone")),
            areas.measure_plan_overlap(&id("space"), &id("zone")),
        ] {
            assert!(
                matches!(&result, Err(PlanAreaError::Unavailable(message)) if message.contains(needle)),
                "{needle}: {result:?}"
            );
        }
    };
    let base = || AxiolidGeometry::new().with_mesh(id("a"), room());
    unavailable(
        base()
            .with_no_body(id("void"))
            .with_group(id("zone"), [id("a"), id("void")]),
        "has no body",
    );
    unavailable(
        base()
            .with_unmeasured(id("slab"), "meshing failed")
            .with_group(id("zone"), [id("a"), id("slab")]),
        "meshing failed",
    );
    unavailable(
        base().with_group(id("zone"), [id("a"), id("ghost")]),
        "no described geometry",
    );
    unavailable(
        base().with_undecided_group(id("zone"), "a relationship end is missing"),
        "a relationship end is missing",
    );
    unavailable(base().with_group(id("zone"), Vec::new()), "groups nothing");
    unavailable(
        base()
            .with_group(id("zone"), [id("a"), id("loop")])
            .with_group(id("loop"), [id("zone")]),
        "member of itself",
    );
    unavailable(
        base()
            .with_no_body(id("void"))
            .with_group(id("inner"), [id("void")])
            .with_group(id("zone"), [id("a"), id("inner")]),
        "has no body",
    );
}

/// An architectural wall 4 m long and 0.2 m thick, with structural walls
/// under all of it, half of it, all of it shifted 2 cm across, and a stub
/// whose corners stand inside it.
fn walls() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("wall"), cuboid(0.0, 0.0, 4.0, 0.2, 3.0))
        .with_mesh(id("full"), cuboid(0.0, 0.0, 4.0, 0.2, 3.0))
        .with_mesh(id("half"), cuboid(0.0, 0.0, 2.0, 0.2, 3.0))
        .with_mesh(id("shifted"), cuboid(0.0, 0.02, 4.0, 0.22, 3.0))
        .with_mesh(id("far"), cuboid(10.0, 10.0, 11.0, 11.0, 3.0))
        .with_mesh(id("stub"), cuboid(1.0, 0.1, 2.0, 0.3, 3.0))
}

fn uncovered(areas: &AxiolidPlanAreaService, cover: &[&str], growth: f64) -> (f64, f64, bool) {
    let cover: Vec<ObjectId> = cover.iter().map(|local| id(local)).collect();
    let area = areas
        .measure_uncovered_area(&id("wall"), &cover, growth)
        .unwrap();
    (
        area.lower_square_metres(),
        area.upper_square_metres(),
        area.is_exact(),
    )
}

/// Exactly `value`, up to the overlay's grid snapping (axiolid/kernel#173).
fn is(measured: (f64, f64, bool), value: f64) -> bool {
    let (lower, upper, exact) = measured;
    exact && (lower - value).abs() < 1e-7 && (upper - value).abs() < 1e-7
}

#[test]
fn an_uncovered_area_is_the_footprint_less_the_cover() {
    let areas = service(walls());
    assert!(is(uncovered(&areas, &[], 0.0), 0.8), "nothing covers it");
    assert!(
        is(uncovered(&areas, &["far"], 0.5), 0.8),
        "too far to cover"
    );
    assert!(is(uncovered(&areas, &["full"], 0.0), 0.0));
    assert!(is(uncovered(&areas, &["half"], 0.0), 0.4));
    // Overlapping covers count once.
    assert!(is(uncovered(&areas, &["half", "full"], 0.0), 0.0));
    // Shifted 2 cm, the cover leaves a 2 cm strip until it is grown by 2 cm.
    assert!(is(uncovered(&areas, &["shifted"], 0.0), 0.08));
    assert!(is(uncovered(&areas, &["shifted"], 0.03), 0.0));
    // A cover's corners beyond the wall do not matter: grown 1 cm it leaves
    // a 1 cm strip, and the half cover grown 10 cm reaches 10 cm further.
    assert!(is(uncovered(&areas, &["shifted"], 0.01), 0.04));
    assert!(is(uncovered(&areas, &["half"], 0.1), 0.38));
    // Corners inside it are rounded: the stub grown 5 cm covers 1.1 m by
    // 0.15 m less two corners of (1 − π/4) · 0.05², bracketed by the two
    // polygons standing in for the disc.
    let exact = 0.8 - (1.1 * 0.15 - 2.0 * (1.0 - std::f64::consts::FRAC_PI_4) * 0.0025);
    let (lower, upper, is_exact) = uncovered(&areas, &["stub"], 0.05);
    assert!(!is_exact);
    assert!(
        lower <= exact && exact <= upper && upper - lower < 2e-4,
        "{lower} {exact} {upper}"
    );
}

#[test]
fn a_tessellated_cover_or_subject_widens_the_uncovered_area() {
    let areas = service(
        walls()
            .with_tessellated_mesh(id("round"), cuboid(0.0, 0.0, 2.0, 0.2, 3.0), 0.005)
            .with_tessellated_mesh(id("rough"), cuboid(0.0, 0.0, 4.0, 0.2, 3.0), 0.005),
    );
    // Growth below the deviation: the measured cover widened by its band.
    let (lower, upper, exact) = uncovered(&areas, &["round"], 0.0);
    assert!(!exact && lower < 0.4 && 0.4 < upper, "{lower} {upper}");
    // Growth above it: bracketed between r − d and r + d.
    let (lower, upper, exact) = uncovered(&areas, &["round"], 0.02);
    assert!(!exact && lower < 0.396 && 0.396 < upper, "{lower} {upper}");
    let rough = areas
        .measure_uncovered_area(&id("rough"), &[id("half")], 0.0)
        .unwrap();
    assert!(!rough.is_exact());
    assert!(rough.lower_square_metres() < 0.4 && 0.4 < rough.upper_square_metres());
}

#[test]
fn an_uncovered_area_with_an_unmeasured_cover_is_unknown() {
    let areas = service(walls().with_unmeasured(id("column"), "meshing failed"));
    assert!(matches!(
        areas.measure_uncovered_area(&id("wall"), &[id("column")], 0.0),
        Err(PlanAreaError::Unavailable(_))
    ));
    assert!(matches!(
        areas.measure_uncovered_area(&id("wall"), &[id("nothing")], 0.0),
        Err(PlanAreaError::UnknownObject(_))
    ));
}
