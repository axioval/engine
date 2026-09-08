//! Envelope membership derived from real geometry.

use axiolid_core::Point3;
use axiolid_mesh::TriMesh;
use axioval_axiolid::{AxiolidEnvelopeMembershipService, AxiolidGeometry};
use axioval_engine::{
    EnvelopeDerivation, EnvelopeMembershipError, EnvelopeMembershipRequest,
    EnvelopeMembershipService,
};
use axioval_ir::{ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("cad", "model").expect("valid source")
}

fn id(local: &str) -> ObjectId {
    ObjectId::new(source(), local).expect("valid id")
}

/// An axis-aligned quad in plan at `z`, spanning `x0..x1` and `y0..y1`.
fn quad(x0: f64, x1: f64, y0: f64, y1: f64, z: f64) -> TriMesh {
    TriMesh::new(
        vec![
            Point3::new(x0, y0, z),
            Point3::new(x1, y0, z),
            Point3::new(x1, y1, z),
            Point3::new(x0, y1, z),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// A space with one wall overlapping it and one wall far away.
fn model() -> AxiolidEnvelopeMembershipService {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0))
        .with_mesh(id("wall-touching"), quad(0.0, 10.0, -0.2, 0.2, 0.0))
        .with_mesh(id("wall-remote"), quad(50.0, 60.0, 50.0, 60.0, 0.0));
    AxiolidEnvelopeMembershipService::new(geometry, source()).with_space(id("space"))
}

fn measure(
    service: &AxiolidEnvelopeMembershipService,
    derivation: EnvelopeDerivation,
) -> Result<Vec<String>, EnvelopeMembershipError> {
    let request = EnvelopeMembershipRequest::new(derivation);
    service.measure_envelope_membership(&request).map(|e| {
        e.derived()
            .iter()
            .map(|o| o.local_id.clone())
            .collect::<Vec<_>>()
    })
}

/// Geometry decides membership: overlapping the space puts a wall on the
/// envelope, and being elsewhere in the model keeps it off.
#[test]
fn only_objects_meeting_a_space_are_derived_onto_the_envelope() {
    let derived = measure(&model(), EnvelopeDerivation::AllSpaces).expect("measurable");
    assert_eq!(derived, vec!["wall-touching".to_string()]);
}

/// A model declaring a wall that geometry does not place on the envelope is a
/// discrepancy in one direction; the reverse is a discrepancy in the other.
#[test]
fn declared_and_derived_disagreements_are_reported_separately() {
    let service = model().with_declared_external(id("wall-remote"));
    let request = EnvelopeMembershipRequest::new(EnvelopeDerivation::AllSpaces);
    let measured = service
        .measure_envelope_membership(&request)
        .expect("measurable");
    assert!(!measured.agrees(), "the two sets must not agree");
    let declared_only: Vec<_> = measured
        .declared_only()
        .iter()
        .map(|o| o.local_id.clone())
        .collect();
    let derived_only: Vec<_> = measured
        .derived_only()
        .iter()
        .map(|o| o.local_id.clone())
        .collect();
    assert_eq!(declared_only, vec!["wall-remote".to_string()]);
    assert_eq!(derived_only, vec!["wall-touching".to_string()]);
}

/// The two derivations name different bounding sets, so they must be able to
/// disagree. A space outside the gross-area group bounds `AllSpaces` only.
#[test]
fn the_two_derivations_use_different_bounding_sets() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space-gross"), quad(0.0, 10.0, 0.0, 10.0, 0.0))
        .with_mesh(id("space-plain"), quad(40.0, 50.0, 40.0, 50.0, 0.0))
        .with_mesh(id("wall-by-plain"), quad(40.0, 50.0, 39.8, 40.2, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_gross_area_space(id("space-gross"))
        .with_space(id("space-plain"));

    let all = measure(&service, EnvelopeDerivation::AllSpaces).expect("measurable");
    let gross = measure(&service, EnvelopeDerivation::GrossAreaGroups).expect("measurable");
    assert_eq!(all, vec!["wall-by-plain".to_string()]);
    assert!(
        gross.is_empty(),
        "the gross-area envelope excludes the plain space, got {gross:?}"
    );
}

/// A model with no registered spaces cannot be measured. Reporting an empty
/// derived set would call every declared wall a discrepancy.
#[test]
fn a_model_without_spaces_is_unavailable_not_empty() {
    let service = AxiolidEnvelopeMembershipService::new(AxiolidGeometry::new(), source())
        .with_declared_external(id("wall"));
    let request = EnvelopeMembershipRequest::new(EnvelopeDerivation::AllSpaces);
    assert_eq!(
        service.measure_envelope_membership(&request),
        Err(EnvelopeMembershipError::Unavailable)
    );
}

/// Declaring a space whose geometry is missing is still unmeasurable.
///
/// Distinct from the no-spaces case: here the host DID declare a bounding set,
/// so the empty-bounding guard cannot be what rejects this. Without its own
/// case, disabling that guard leaves every test passing.
#[test]
fn a_declared_space_without_geometry_is_unavailable() {
    let geometry = AxiolidGeometry::new().with_mesh(id("wall"), quad(0.0, 1.0, 0.0, 1.0, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_space(id("space-with-no-mesh"));
    let request = EnvelopeMembershipRequest::new(EnvelopeDerivation::AllSpaces);
    assert_eq!(
        service.measure_envelope_membership(&request),
        Err(EnvelopeMembershipError::Unavailable)
    );
}

/// A model with spaces but no other objects derives an empty envelope rather
/// than failing: the question was answerable, the answer is "nothing bounds it".
///
/// This is what makes the empty-bounding guard load-bearing: measurable and
/// unmeasurable must be distinguishable.
#[test]
fn spaces_present_but_nothing_else_derives_an_empty_envelope() {
    let geometry = AxiolidGeometry::new().with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source()).with_space(id("space"));
    let derived = measure(&service, EnvelopeDerivation::AllSpaces).expect("measurable");
    assert!(
        derived.is_empty(),
        "nothing bounds a lone space, got {derived:?}"
    );
}
