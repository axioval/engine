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
    AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_declared_internal(id("wall-touching"))
        .with_declared_internal(id("wall-remote"))
}

/// A request for `derivation` around the objects named `bounding`.
fn request(derivation: EnvelopeDerivation, bounding: &[&str]) -> EnvelopeMembershipRequest {
    EnvelopeMembershipRequest::new(derivation, bounding.iter().map(|b| id(b)).collect())
}

fn measure(
    service: &AxiolidEnvelopeMembershipService,
    derivation: EnvelopeDerivation,
    bounding: &[&str],
) -> Result<Vec<String>, EnvelopeMembershipError> {
    service
        .measure_envelope_membership(&request(derivation, bounding))
        .map(|e| {
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
    let derived = measure(&model(), EnvelopeDerivation::AllSpaces, &["space"]).expect("measurable");
    assert_eq!(derived, vec!["wall-touching".to_string()]);
}

/// A model declaring a wall that geometry does not place on the envelope is a
/// discrepancy in one direction; the reverse is a discrepancy in the other.
#[test]
fn declared_and_derived_disagreements_are_reported_separately() {
    let service = model().with_declared_external(id("wall-remote"));
    let request = request(EnvelopeDerivation::AllSpaces, &["space"]);
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

/// The request's bounding objects are used exactly: a space it does not name
/// bounds nothing, and one it names that the other derivation would not use
/// bounds this one. Both derivations derive around whatever they are given.
#[test]
fn the_request_names_exactly_the_bounding_objects() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space-gross"), quad(0.0, 10.0, 0.0, 10.0, 0.0))
        .with_mesh(id("space-plain"), quad(40.0, 50.0, 40.0, 50.0, 0.0))
        .with_mesh(id("wall-by-plain"), quad(40.0, 50.0, 39.8, 40.2, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_declared_internal(id("wall-by-plain"));

    let all = measure(
        &service,
        EnvelopeDerivation::AllSpaces,
        &["space-gross", "space-plain"],
    )
    .expect("measurable");
    let gross = measure(
        &service,
        EnvelopeDerivation::GrossAreaGroups,
        &["space-gross"],
    )
    .expect("measurable");
    assert_eq!(all, vec!["wall-by-plain".to_string()]);
    assert!(
        gross.is_empty(),
        "the gross-area envelope excludes the plain space, got {gross:?}"
    );
    // The derivation name does not choose the set; the request does.
    let plain = measure(
        &service,
        EnvelopeDerivation::GrossAreaGroups,
        &["space-plain"],
    )
    .expect("measurable");
    assert_eq!(plain, vec!["wall-by-plain".to_string()]);
}

/// A request naming no bounding object cannot be measured. Reporting an empty
/// derived set would call every declared wall a discrepancy.
#[test]
fn a_request_without_bounding_objects_is_unavailable_not_empty() {
    let geometry = AxiolidGeometry::new().with_mesh(id("wall"), quad(0.0, 1.0, 0.0, 1.0, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_declared_external(id("wall"));
    assert_eq!(
        service.measure_envelope_membership(&request(EnvelopeDerivation::AllSpaces, &[])),
        Err(EnvelopeMembershipError::Unavailable)
    );
}

/// A bounding object without a mesh leaves the region unknown, whether it was
/// never described or is declared bodiless, even beside a measurable one.
///
/// Distinct from the empty case: here the request DID name a bounding set, so
/// the empty-bounding guard cannot be what rejects this.
#[test]
fn a_bounding_object_without_geometry_is_unavailable() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0))
        .with_mesh(id("wall"), quad(0.0, 1.0, 0.0, 1.0, 0.0))
        .with_no_body(id("space-bodiless"));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source());
    for missing in ["space-with-no-mesh", "space-bodiless"] {
        assert_eq!(
            service.measure_envelope_membership(&request(
                EnvelopeDerivation::AllSpaces,
                &["space", missing]
            )),
            Err(EnvelopeMembershipError::Unavailable),
            "{missing}"
        );
    }
}

/// A bounding object is the envelope's inside: declaring it external is not a
/// discrepancy the derivation could ever resolve, so it leaves the comparison.
#[test]
fn a_bounding_object_is_never_declared_on_the_envelope() {
    let geometry = AxiolidGeometry::new().with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_declared_external(id("space"));
    let measured = service
        .measure_envelope_membership(&request(EnvelopeDerivation::AllSpaces, &["space"]))
        .unwrap();
    assert!(measured.declared().is_empty(), "{measured:?}");
    assert!(measured.agrees());
}

/// A model with spaces but no other objects derives an empty envelope rather
/// than failing: the question was answerable, the answer is "nothing bounds it".
///
/// This is what makes the empty-bounding guard load-bearing: measurable and
/// unmeasurable must be distinguishable.
#[test]
fn spaces_present_but_nothing_else_derives_an_empty_envelope() {
    let geometry = AxiolidGeometry::new().with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source());
    let derived = measure(&service, EnvelopeDerivation::AllSpaces, &["space"]).expect("measurable");
    assert!(
        derived.is_empty(),
        "nothing bounds a lone space, got {derived:?}"
    );
}

/// A closed, outward-oriented box, as real exports produce: its bottom face
/// winds opposite to its top when seen from above.
fn closed_box(x0: f64, x1: f64, y0: f64, y1: f64, z0: f64, z1: f64) -> TriMesh {
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

/// A closed wall body overlapping a closed space in plan is on the envelope.
#[test]
fn closed_bodies_are_measured_by_their_footprint() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), closed_box(0.0, 10.0, 0.0, 10.0, 0.0, 3.0))
        .with_mesh(id("wall"), closed_box(0.0, 10.0, -0.2, 0.2, 0.0, 3.0));
    let service = AxiolidEnvelopeMembershipService::new(geometry, source())
        .with_declared_internal(id("wall"));
    let derived = measure(&service, EnvelopeDerivation::AllSpaces, &["space"]).expect("measurable");
    assert_eq!(derived, vec!["wall".to_string()]);
}

/// A gross-area space drawn to the outer faces, with an external wall inside
/// it along its edge and an internal wall across its middle.
fn gross_area_model() -> AxiolidEnvelopeMembershipService {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0))
        .with_mesh(id("wall-edge"), quad(0.0, 10.0, 0.0, 0.3, 0.0))
        .with_mesh(id("wall-inner"), quad(0.3, 9.7, 4.9, 5.1, 0.0));
    AxiolidEnvelopeMembershipService::new(geometry, source())
}

/// Being inside the covered region is not being on its envelope: a wall
/// wholly inside is internal, and a wall that reaches the outline from inside
/// is external. A mutation that drops the outline test derives both.
#[test]
fn only_objects_reaching_the_outline_are_on_the_envelope() {
    let service = gross_area_model()
        .with_declared_internal(id("wall-edge"))
        .with_declared_internal(id("wall-inner"));
    let derived =
        measure(&service, EnvelopeDerivation::GrossAreaGroups, &["space"]).expect("measurable");
    assert_eq!(derived, vec!["wall-edge".to_string()]);
}

/// An object the model declares neither way is reported undeclared, and
/// takes no part in the comparison: absent is not internal.
#[test]
fn undeclared_objects_are_reported_not_compared() {
    let service = gross_area_model().with_declared_internal(id("wall-inner"));
    let request = request(EnvelopeDerivation::GrossAreaGroups, &["space"]);
    let measured = service.measure_envelope_membership(&request).unwrap();
    assert!(measured.agrees(), "{measured:?}");
    assert_eq!(measured.undeclared(), &[id("wall-edge")]);
}

/// An unmeasured wall has no known membership, so it is reported, never
/// silently dropped; an unmeasured bounding space leaves no region at all.
#[test]
fn unmeasured_bodies_are_not_compared() {
    let geometry = AxiolidGeometry::new()
        .with_mesh(id("space"), quad(0.0, 10.0, 0.0, 10.0, 0.0))
        .with_unmeasured(id("wall"), "no body representation");
    let service = AxiolidEnvelopeMembershipService::new(geometry.clone(), source())
        .with_declared_external(id("wall"));
    let measured = service
        .measure_envelope_membership(&request(EnvelopeDerivation::GrossAreaGroups, &["space"]))
        .unwrap();
    assert!(measured.agrees());
    assert_eq!(measured.undeclared(), &[id("wall")]);

    let service = AxiolidEnvelopeMembershipService::new(
        geometry.with_unmeasured(id("other-space"), "no body representation"),
        source(),
    );
    assert_eq!(
        service.measure_envelope_membership(&request(
            EnvelopeDerivation::GrossAreaGroups,
            &["space", "other-space"]
        )),
        Err(EnvelopeMembershipError::Unavailable)
    );
}
