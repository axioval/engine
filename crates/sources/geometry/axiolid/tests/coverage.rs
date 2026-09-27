//! Effective coverage of a room's footprint, measured from real meshes in
//! each reach: grown footprints, travel distance and sight.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidPlanAreaService};
use axioval_engine::{
    CoverageEvidence, CoverageRequest, EffectMeets, EffectReach, Participant, PlanAreaError,
    PlanAreaService,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A closed, outward-oriented axis-aligned box.
fn cuboid(min: [f64; 3], max: [f64; 3]) -> TriMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
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
        vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7,
            3, 1, 2, 6, 1, 6, 5,
        ],
    )
}

/// Room `room`, x 0 to 10 and y 0 to 4, split by wall `wall` at x 5 to 5.2
/// up to y 3.5, which leaves a 0.5 m gap along the north side. Device `a`
/// stands at (2.5, 2), device `b` at (1.1, 1.1), device `far` outside the
/// room at (20, 2), and device `edge` straddles the wall's west face.
fn scene() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 3.0]))
        .with_mesh(id("wall"), cuboid([5.0, 0.0, 0.0], [5.2, 3.5, 3.0]))
        .with_mesh(id("a"), cuboid([2.4, 1.9, 2.5], [2.6, 2.1, 2.7]))
        .with_mesh(id("b"), cuboid([1.0, 1.0, 2.5], [1.2, 1.2, 2.7]))
        .with_mesh(id("far"), cuboid([19.9, 1.9, 2.5], [20.1, 2.1, 2.7]))
        .with_mesh(id("edge"), cuboid([4.9, 0.9, 2.5], [5.1, 1.1, 2.7]))
        .with_no_body(id("zone"))
}

fn measure(
    geometry: AxiolidGeometry,
    reach: EffectReach,
    range: f64,
    sources: &[(&str, bool)],
    blockers: &[(&str, bool)],
) -> Result<CoverageEvidence, PlanAreaError> {
    let participants = |list: &[(&str, bool)]| {
        list.iter()
            .map(|(local, certain)| Participant::new(id(local), *certain))
            .collect()
    };
    let request = CoverageRequest::try_new(
        id("room"),
        reach,
        range,
        participants(sources),
        participants(blockers),
    )
    .expect("valid request");
    AxiolidPlanAreaService::new(geometry, source()).measure_coverage(&request)
}

fn covered(evidence: &CoverageEvidence) -> (f64, f64) {
    (
        evidence.covered().lower_square_metres(),
        evidence.covered().upper_square_metres(),
    )
}

fn meets(evidence: &CoverageEvidence) -> Vec<EffectMeets> {
    evidence
        .effects()
        .iter()
        .map(|(_, meets)| meets.clone())
        .collect()
}

#[test]
fn grown_footprints_are_bracketed_by_the_two_dilations() {
    let evidence = measure(
        scene(),
        EffectReach::Grown,
        1.0,
        &[("b", true), ("far", true)],
        &[],
    )
    .unwrap();
    assert!((evidence.footprint().lower_square_metres() - 40.0).abs() < 1e-9);
    // A 0.2 m square grown by 1 m: 0.04 + 4 · 0.2 + π, all inside the room.
    let exact = 0.04 + 0.8 + std::f64::consts::PI;
    let (lower, upper) = covered(&evidence);
    assert!(lower <= exact && exact <= upper, "{lower} {upper}");
    assert!(upper - lower < 0.02, "{lower} {upper}");
    assert!(!evidence.covered().is_exact());
    assert_eq!(meets(&evidence), [EffectMeets::Surely, EffectMeets::No]);
}

#[test]
fn an_uncertain_source_raises_only_the_upper_bound() {
    let evidence = measure(
        scene(),
        EffectReach::Grown,
        1.0,
        &[("a", false), ("b", true)],
        &[],
    )
    .unwrap();
    let (lower, upper) = covered(&evidence);
    let one = 0.04 + 0.8 + std::f64::consts::PI;
    assert!(lower <= one && one + 3.0 < upper, "{lower} {upper}");
}

#[test]
fn travel_distance_goes_round_a_wall_through_its_gap() {
    let open = measure(scene(), EffectReach::Travel, 4.0, &[("a", true)], &[]).unwrap();
    let walled = measure(
        scene(),
        EffectReach::Travel,
        4.0,
        &[("a", true)],
        &[("wall", true)],
    )
    .unwrap();
    let (open_lower, open_upper) = covered(&open);
    let (lower, upper) = covered(&walled);
    // Behind the wall, only what 1.085 m of travel past the gap's corner
    // at (5, 3.5) reaches; the whole west half lies within 3.2 m.
    assert!(lower >= 19.9, "{lower} {upper}");
    assert!(
        upper <= 20.0 + 0.1 + 0.25 * std::f64::consts::PI * 1.3 * 1.3,
        "{lower} {upper}"
    );
    assert!(upper < open_lower, "{upper} {open_lower}");
    assert!(open_upper - open_lower < 1.0, "{open_lower} {open_upper}");
    assert_eq!(meets(&walled), [EffectMeets::Surely]);
}

#[test]
fn an_uncertain_blocker_narrows_only_the_inner_bound() {
    let evidence = measure(
        scene(),
        EffectReach::Travel,
        4.0,
        &[("a", true)],
        &[("wall", false)],
    )
    .unwrap();
    let (lower, upper) = covered(&evidence);
    assert!(lower < 21.5 && upper > 25.0, "{lower} {upper}");
}

#[test]
fn sight_reaches_through_the_gap_but_not_through_the_wall() {
    let evidence = measure(
        scene(),
        EffectReach::Visible,
        20.0,
        &[("a", true)],
        &[("wall", true)],
    )
    .unwrap();
    let (lower, upper) = covered(&evidence);
    // The whole west half, and a wedge through the gap.
    assert!(lower > 20.0 && upper < 24.0, "{lower} {upper}");
    assert!(upper - lower < 1e-6, "{lower} {upper}");
    let short = measure(
        scene(),
        EffectReach::Visible,
        1.0,
        &[("a", true)],
        &[("wall", true)],
    )
    .unwrap();
    let (lower, upper) = covered(&short);
    assert!(
        lower <= std::f64::consts::PI && std::f64::consts::PI <= upper,
        "{lower} {upper}"
    );
}

#[test]
fn a_centre_outside_covers_nothing_and_one_on_a_boundary_is_unmeasured() {
    let evidence = measure(
        scene(),
        EffectReach::Visible,
        20.0,
        &[("far", true), ("edge", true)],
        &[("wall", true)],
    )
    .unwrap();
    assert_eq!(meets(&evidence)[1], EffectMeets::No);
    assert!(matches!(meets(&evidence)[0], EffectMeets::Unmeasured(_)));
    // An unmeasured effect may cover anything.
    assert_eq!(covered(&evidence), (0.0, 40.0));
}

#[test]
fn tessellated_sources_are_unmeasured_and_tessellated_subjects_refuse() {
    let curved =
        scene().with_tessellated_mesh(id("b"), cuboid([1.0, 1.0, 2.5], [1.2, 1.2, 2.7]), 1e-3);
    let evidence = measure(curved, EffectReach::Grown, 1.0, &[("b", true)], &[]).unwrap();
    assert!(matches!(meets(&evidence)[0], EffectMeets::Unmeasured(_)));
    assert_eq!(covered(&evidence), (0.0, 40.0));
    let curved_room =
        scene().with_tessellated_mesh(id("room"), cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 3.0]), 1e-3);
    assert!(measure(curved_room, EffectReach::Grown, 1.0, &[("b", true)], &[]).is_err());
    let evidence = measure(
        scene(),
        EffectReach::Travel,
        1.0,
        &[("b", true)],
        &[("zone", true)],
    )
    .unwrap();
    assert_eq!(meets(&evidence), [EffectMeets::Surely]);
}

/// Rooms `west` (x 0 to 5) and `east` (x 5.2 to 10.2), both 4 m deep, with
/// the 0.2 m wall between them left out; door `door` fills the wall at y 1.5
/// to 2.5, and opening `gap`, bodiless, at y 3 to 3.5. Sprinkler `sprinkler`
/// hangs in the east room at (7, 2).
fn rooms() -> AxiolidGeometry {
    AxiolidGeometry::new()
        .with_mesh(id("west"), cuboid([0.0, 0.0, 0.0], [5.0, 4.0, 3.0]))
        .with_mesh(id("east"), cuboid([5.2, 0.0, 0.0], [10.2, 4.0, 3.0]))
        .with_mesh(id("door"), cuboid([5.0, 1.5, 0.0], [5.2, 2.5, 2.1]))
        .with_mesh(id("sprinkler"), cuboid([6.9, 1.9, 2.8], [7.1, 2.1, 2.9]))
        .with_no_body(id("gap"))
}

fn through(
    service: &AxiolidPlanAreaService,
    reach: EffectReach,
    range: f64,
    connected: &[(&str, bool)],
    passages: &[(&str, bool)],
) -> Result<CoverageEvidence, PlanAreaError> {
    let participants = |list: &[(&str, bool)]| {
        list.iter()
            .map(|(local, certain)| Participant::new(id(local), *certain))
            .collect()
    };
    let request = CoverageRequest::try_new(
        id("west"),
        reach,
        range,
        vec![Participant::new(id("sprinkler"), true)],
        Vec::new(),
    )
    .and_then(|request| request.with_connections(participants(connected), participants(passages)))
    .expect("valid request");
    service.measure_coverage(&request)
}

#[test]
fn an_effect_continues_through_a_door_into_the_next_room() {
    let service = AxiolidPlanAreaService::new(rooms(), source());
    // On its own, the west room holds no sprinkler.
    let alone = through(&service, EffectReach::Travel, 3.0, &[], &[]).unwrap();
    assert_eq!(covered(&alone), (0.0, 0.0));
    assert_eq!(meets(&alone), [EffectMeets::No]);

    // Through the door, 1.2 m of travel is left past the doorway at x 5:
    // at least the half disc of 1 m round (5, 2), at most the half disc of
    // 1.2 m widened by the door's width.
    let joined = through(
        &service,
        EffectReach::Travel,
        3.0,
        &[("east", true)],
        &[("door", true)],
    )
    .unwrap();
    let (lower, upper) = covered(&joined);
    assert!(lower > 0.5 * std::f64::consts::PI, "{lower} {upper}");
    assert!(
        upper < 0.5 * std::f64::consts::PI * 1.44 + 1.2 + 0.2,
        "{lower} {upper}"
    );
    assert_eq!(meets(&joined), [EffectMeets::Surely]);
    let locator = &joined.covered().evidence().locator;
    assert!(
        locator.ends_with(&format!(":into={}:via={}", id("east"), id("door"))),
        "{locator}"
    );

    // Sight passes the doorway too, but no farther than it reaches.
    let seen = through(
        &service,
        EffectReach::Visible,
        20.0,
        &[("east", true)],
        &[("door", true)],
    )
    .unwrap();
    let (lower, upper) = covered(&seen);
    assert!(lower > 1.0 && upper < 20.0, "{lower} {upper}");

    // An uncertain door widens only the upper bound.
    let maybe = through(
        &service,
        EffectReach::Travel,
        3.0,
        &[("east", true)],
        &[("door", false)],
    )
    .unwrap();
    let (lower, upper) = covered(&maybe);
    assert!(
        lower < 1e-6 && upper > 0.5 * std::f64::consts::PI,
        "{lower} {upper}"
    );
    assert_eq!(meets(&maybe), [EffectMeets::Possibly]);
}

#[test]
fn a_bodiless_opening_joins_through_its_void_or_leaves_the_bound_open() {
    let with_void = AxiolidPlanAreaService::new(rooms(), source())
        .with_opening_void(id("gap"), cuboid([5.0, 3.0, 0.0], [5.2, 3.5, 2.1]));
    let evidence = through(
        &with_void,
        EffectReach::Travel,
        3.0,
        &[("east", true)],
        &[("gap", true)],
    )
    .unwrap();
    let (lower, upper) = covered(&evidence);
    assert!(lower > 0.1 && upper < 20.0, "{lower} {upper}");

    // Without a void the opening cannot be joined: the reach may be larger
    // than measured, up to the whole room.
    for service in [
        AxiolidPlanAreaService::new(rooms(), source()),
        AxiolidPlanAreaService::new(rooms(), source()).with_unmeasured_opening_void(id("gap")),
    ] {
        let evidence = through(
            &service,
            EffectReach::Travel,
            3.0,
            &[("east", true)],
            &[("gap", true)],
        )
        .unwrap();
        let (lower, upper) = covered(&evidence);
        assert!(lower < 1e-6, "{lower} {upper}");
        assert!((upper - 20.0).abs() < 1e-6, "{lower} {upper}");
    }
}
