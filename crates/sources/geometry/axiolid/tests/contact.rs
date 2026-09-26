//! Contact measurement over real Axiolid geometry.
//!
//! These run the published kernel, not a double: the point is to prove the
//! adapter measures actual meshes rather than that a stub returns a constant.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidContactService, AxiolidGeometry};
use axioval_engine::{ContactError, ContactService, ContactSide, ContactTolerance};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// An axis-aligned `size` x `size` quad at height `z`.
fn quad(z: f64, size: f64) -> TriMesh {
    TriMesh::new(
        vec![
            Point3::new(0.0, 0.0, z),
            Point3::new(size, 0.0, z),
            Point3::new(size, size, z),
            Point3::new(0.0, size, z),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// A quad offset in x, used to place a counterpart over part of a face.
fn quad_at(z: f64, size: f64, x_offset: f64) -> TriMesh {
    TriMesh::new(
        vec![
            Point3::new(x_offset, 0.0, z),
            Point3::new(x_offset + size, 0.0, z),
            Point3::new(x_offset + size, size, z),
            Point3::new(x_offset, size, z),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn tolerance() -> ContactTolerance {
    ContactTolerance::try_new(0.01, 0.01, 0.0001).expect("valid tolerance")
}

fn request(subject: &str, side: ContactSide) -> axioval_engine::ContactRequest {
    axioval_engine::ContactRequest::new(id(subject), side, tolerance())
}

#[test]
fn a_slab_resting_on_a_wall_reports_full_contact() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad(0.005, 2.0));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    assert!(
        (evidence.whole_area_square_metres() - 4.0).abs() < 1e-9,
        "whole face area is measured from the mesh: {}",
        evidence.whole_area_square_metres()
    );
    assert!(
        (evidence.contact_ratio() - 1.0).abs() < 1e-9,
        "a fully covered face is fully in contact: {}",
        evidence.contact_ratio()
    );
    assert_eq!(evidence.touching(), &[id("wall")]);
}

#[test]
fn a_counterpart_beyond_the_gap_tolerance_is_not_touching() {
    // 0.5 m away, far outside the 0.01 m gap tolerance.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad(0.5, 2.0));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    assert!(evidence.contact_area_square_metres() < 1e-12);
    assert!(evidence.touching().is_empty(), "{:?}", evidence.touching());
    let nearest = evidence.nearest_distance_metres().expect("a distance");
    assert!(
        (nearest - 0.5).abs() < 1e-9,
        "the gap is still measured and reported: {nearest}"
    );
}

#[test]
fn partial_coverage_is_measured_as_a_fraction_not_rounded_to_all_or_nothing() {
    // Counterpart covers the right half of a 2 m face.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad_at(0.005, 1.0, 1.0));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    // 1x1 counterpart over a 2x2 face: exactly a quarter, not "some contact".
    // Per-triangle counting reported 1.0 here, which would tell a reviewer a
    // half-supported slab is fully supported.
    let ratio = evidence.contact_ratio();
    assert!(
        (ratio - 0.25).abs() < 1e-9,
        "partial contact is measured, not rounded: got {ratio}"
    );
}

#[test]
fn a_counterpart_on_the_other_side_is_not_contact_for_the_requested_side() {
    // The wall sits BELOW the slab, but contact was requested above.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad(-0.005, 2.0));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    assert!(
        evidence.contact_area_square_metres() < 1e-12,
        "side is part of the question, not a detail: {:?}",
        evidence.touching()
    );

    // The same geometry IS contact when asked about below.
    let below = service
        .measure_contact(&request("slab", ContactSide::Below))
        .expect("measurable");
    assert_eq!(below.touching(), &[id("wall")]);
}

#[test]
fn an_unknown_subject_is_unavailable_rather_than_silently_zero() {
    let service = AxiolidContactService::new(AxiolidGeometry::new(), source());
    let outcome = service.measure_contact(&request("absent", ContactSide::Above));
    assert_eq!(outcome.unwrap_err(), ContactError::Unavailable);
}

#[test]
fn contact_evidence_is_exact_and_locates_its_subject() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad(0.005, 2.0));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    assert!(evidence.evidence().exact, "kernel measurement is exact");
    assert!(
        evidence.evidence().locator.contains("slab"),
        "a reviewer must be able to find what was measured: {}",
        evidence.evidence().locator
    );
}

/// Two counterparts covering overlapping halves must not sum past the face.
///
/// Independent counterparts are measured independently, so their areas can
/// double-count a shared region. A ratio above 1.0 would tell a reviewer a
/// face is more than fully supported, which is not a thing.
#[test]
fn overlapping_counterparts_cannot_report_more_than_the_whole_face() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall-a"), quad(0.005, 2.0))
        .with_mesh(id("wall-b"), quad(0.005, 2.0));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    assert!(
        evidence.contact_ratio() <= 1.0,
        "a face cannot be more than fully in contact: {}",
        evidence.contact_ratio()
    );
}

/// Two counterparts leaving a gap between them do not support the gap.
///
/// The overlay may return a polygon whose interior boundary is that void.
/// Counting a hole as covered would report an unsupported strip as supported.
#[test]
fn a_void_between_counterparts_is_not_reported_as_contact() {
    // Two 0.5 m strips at x=[0,0.5] and x=[1.5,2.0]: half the 2 m face is void.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("left"), quad_at(0.005, 0.5, 0.0))
        .with_mesh(id("right"), quad_at(0.005, 0.5, 1.5));
    let service = AxiolidContactService::new(geometry, source());

    let evidence = service
        .measure_contact(&request("slab", ContactSide::Above))
        .expect("measurable");

    // Each strip covers 0.5 x 0.5 = 0.25 m2 of a 4 m2 face.
    let ratio = evidence.contact_ratio();
    assert!(
        (ratio - 0.125).abs() < 1e-9,
        "only the covered strips count, not the void between: {ratio}"
    );
}

/// An overlap below the minimum polygon area is numerical dust, not contact.
///
/// The tolerance exists to reject specks: two faces sharing a hairline strip
/// have not been shown to bear on each other. Without the filter the ratio
/// would creep upward from slivers the model never meant as support.
// Exact zero is the contract: the filter discards the polygon entirely rather
// than accumulating a small area.
#[allow(clippy::float_cmp)]
#[test]
fn an_overlap_below_the_minimum_polygon_area_is_discarded() {
    // A 2 m quad offset by 1.999 m overlaps by 0.001 m x 2 m = 0.002 m2.
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad_at(0.005, 2.0, 1.999));
    let service = AxiolidContactService::new(geometry, source());
    // Minimum polygon area 0.01 m2 is five times the sliver.
    let strict = ContactTolerance::try_new(0.01, 0.01, 0.01).expect("valid tolerance");
    let measured = service
        .measure_contact(&axioval_engine::ContactRequest::new(
            id("slab"),
            ContactSide::Above,
            strict,
        ))
        .expect("measurable");
    assert_eq!(
        measured.contact_area_square_metres(),
        0.0,
        "a sliver under the minimum polygon area must not count as contact"
    );
    assert!(measured.touching().is_empty(), "nothing bears on the slab");
}

/// An object whose body could not be measured may be exactly what the
/// subject rests on, so no contact answer is complete while one exists.
#[test]
fn an_unmeasured_object_makes_contact_unavailable() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("slab"), quad(0.0, 2.0))
        .with_mesh(id("wall"), quad(0.005, 2.0))
        .with_unmeasured(id("beam"), "unsupported representation");
    let service = AxiolidContactService::new(geometry, source());
    assert_eq!(
        service.measure_contact(&request("slab", ContactSide::Above)),
        Err(ContactError::Unavailable)
    );
}
