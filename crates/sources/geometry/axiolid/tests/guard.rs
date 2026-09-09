//! Guard edges measured from real geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidGeometry, AxiolidGuardService};
use axioval_engine::{GuardSearch, GuardService};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// A box spanning `x0..x1`, `y0..y1`, `z0..z1`.
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

fn search() -> GuardSearch {
    GuardSearch::try_new(0.5, 0.1).expect("valid search")
}

/// A railing above the deck is a barrier; the sign of its top offset is what
/// distinguishes it from something to step down onto.
#[test]
fn an_element_above_the_surface_is_a_barrier() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 0.0, 0.2))
        // A railing standing on the deck's north edge, 1 m tall.
        .with_mesh(id("railing"), body(0.0, 4.0, 3.95, 4.05, 0.2, 1.2));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");

    assert_eq!(evidence.evaluated_surfaces(), 1);
    let edge = &evidence.edges()[0];
    assert_eq!(edge.barriers().len(), 1, "the railing is a barrier");
    assert!(edge.landings().is_empty(), "nothing to step down onto");
    let barrier = &edge.barriers()[0];
    assert_eq!(barrier.element(), &id("railing"));
    assert!(
        barrier.top_offset_metres() > 0.0,
        "a barrier stands above the walking surface, got {}",
        barrier.top_offset_metres()
    );
}

/// A terrace below the deck is a landing, not a barrier.
#[test]
fn an_element_below_the_surface_is_a_landing() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 1.0, 1.2))
        // A wide step just below the deck's edge.
        .with_mesh(id("terrace"), body(0.0, 4.0, 3.9, 5.0, 0.0, 0.9));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    let edge = &evidence.edges()[0];

    assert_eq!(edge.landings().len(), 1, "the terrace is a landing");
    assert!(edge.barriers().is_empty(), "nothing stands above the deck");
    assert!(
        edge.landings()[0].top_offset_metres() < 0.0,
        "a landing sits below the walking surface"
    );
}

/// A railing along one side of a square deck covers about a quarter of the
/// perimeter, and the interval says which quarter.
///
/// Coverage is what a guard policy unions to decide whether an edge is
/// protected, so a railing must not report covering the whole edge.
#[test]
fn a_barrier_covers_only_the_edge_it_runs_along() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 0.0, 0.2))
        .with_mesh(id("railing"), body(0.0, 4.0, 3.95, 4.05, 0.2, 1.2));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    let [start, end] = evidence.edges()[0].barriers()[0].edge_interval();

    let covered = end - start;
    assert!(
        covered > 0.0 && covered < 0.5,
        "one side of a square is a minority of its perimeter, got {covered}"
    );
    assert!((0.0..=1.0).contains(&start) && (0.0..=1.0).contains(&end));
}

/// A crate beside a railing is a climbable, reported against that barrier.
#[test]
fn an_object_beside_a_barrier_is_reported_as_climbable() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 0.0, 0.2))
        .with_mesh(id("railing"), body(0.0, 4.0, 3.95, 4.05, 0.2, 1.2))
        // A planter pushed up against the railing, inside the deck.
        .with_mesh(id("planter"), body(1.0, 1.6, 3.3, 3.9, 0.2, 0.8));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    let edge = &evidence.edges()[0];

    let climbable = edge
        .climbables()
        .iter()
        .find(|c| c.element() == &id("planter"))
        .expect("the planter is a climbing aid");
    assert_eq!(
        climbable.barrier(),
        &id("railing"),
        "a climbable is measured against the barrier it would defeat"
    );
    assert!(climbable.distance_to_barrier_metres() >= 0.0);
}

/// A surface with no geometry contributes no edge rather than a fake one.
#[test]
fn a_surface_without_geometry_yields_no_edge() {
    let service = AxiolidGuardService::new(AxiolidGeometry::new(), source())
        .with_walking_surface(id("ghost"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    assert_eq!(evidence.evaluated_surfaces(), 0);
    assert!(evidence.edges().is_empty());
}

/// A low parapet beside a tall railing is BOTH a barrier and a climbing aid.
///
/// Being a barrier in its own right does not stop an object helping defeat a
/// taller one -- that is the classic defeat case, and excluding barriers from
/// the climbable pass hid it entirely.
#[test]
fn a_low_barrier_can_still_be_a_climbing_aid_for_a_taller_one() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 0.0, 0.2))
        .with_mesh(id("railing"), body(0.0, 4.0, 3.95, 4.05, 0.2, 1.2))
        .with_mesh(id("parapet"), body(1.0, 1.6, 3.3, 3.9, 0.2, 0.8));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    let edge = &evidence.edges()[0];

    assert!(
        edge.barriers()
            .iter()
            .any(|b| b.element() == &id("parapet")),
        "the parapet stands above the deck, so it is a barrier"
    );
    assert!(
        edge.climbables()
            .iter()
            .any(|c| c.element() == &id("parapet") && c.barrier() == &id("railing")),
        "and it is also a step up to the taller railing"
    );
    assert!(
        !edge
            .climbables()
            .iter()
            .any(|c| c.element() == &id("railing") && c.barrier() == &id("parapet")),
        "but the taller railing is not a climbing aid for the shorter parapet"
    );
}

/// Proximity is measured between footprints, not between vertices.
///
/// Two boxes whose faces are 50 mm apart have corner vertices a metre apart.
/// Measuring vertex-to-vertex put a touching climbing aid outside a 0.5 m
/// search radius, so nothing was ever reported.
#[test]
fn proximity_is_measured_between_footprints_not_vertices() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 0.0, 0.2))
        .with_mesh(id("railing"), body(0.0, 4.0, 3.95, 4.05, 0.2, 1.2))
        // 0.05 m from the railing face, but ~1 m corner-to-corner.
        .with_mesh(id("crate-box"), body(1.0, 1.6, 3.3, 3.9, 0.2, 0.5));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    let edge = &evidence.edges()[0];

    let aid = edge
        .climbables()
        .iter()
        .find(|c| c.element() == &id("crate-box"))
        .expect("a box 50 mm from the railing is within a 0.5 m search");
    assert!(
        aid.distance_to_barrier_metres() < 0.1,
        "the measured gap is the face separation, got {}",
        aid.distance_to_barrier_metres()
    );
}

/// An object far from the barrier is not a climbing aid.
///
/// The search radius is what bounds the measurement: without it every object
/// in the model is reported against every barrier, and a reviewer is handed
/// furniture on the far side of the building as a climbing risk.
#[test]
fn a_distant_object_is_not_a_climbing_aid() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("deck"), body(0.0, 4.0, 0.0, 4.0, 0.0, 0.2))
        .with_mesh(id("railing"), body(0.0, 4.0, 3.95, 4.05, 0.2, 1.2))
        // Well beyond the 0.5 m search radius from the railing.
        .with_mesh(id("far-crate"), body(0.5, 1.1, 0.2, 0.8, 0.2, 0.6));
    let service = AxiolidGuardService::new(geometry, source()).with_walking_surface(id("deck"));
    let evidence = service.measure_guard_edges(search()).expect("measurable");
    let edge = &evidence.edges()[0];

    assert!(
        !edge
            .climbables()
            .iter()
            .any(|c| c.element() == &id("far-crate")),
        "an object 3 m from the railing is not a step up to it"
    );
}
