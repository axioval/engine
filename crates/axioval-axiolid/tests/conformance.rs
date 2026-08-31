//! Conformance tests for the source-independent Axiolid geometry contract.

use axioval_axiolid::{
    AxiolidError, Exactness, GeometryBackend, GeometryEvidence, GeometrySource,
    InMemoryGeometrySource, SourceIdentity, UnavailableGeometryBackend,
};

#[test]
fn geometry_source_serves_proprietary_cad_evidence_without_openbim() {
    let mut source = InMemoryGeometrySource::new(SourceIdentity::new("cad-export-7"));
    source
        .insert(GeometryEvidence::new(
            "beam-12",
            Exactness::Exact,
            "brep://cad-export-7/beam-12",
        ))
        .expect("evidence belongs to the configured source");

    let evidence = source
        .geometry_for("beam-12")
        .expect("stored proprietary CAD evidence is available");
    assert_eq!(evidence.subject_id(), "cad-export-7:beam-12");
    assert_eq!(evidence.exactness(), Exactness::Exact);
}

#[test]
fn geometry_source_rejects_foreign_source_evidence() {
    let mut source = InMemoryGeometrySource::new(SourceIdentity::new("cad-export-7"));
    let error = source
        .insert(GeometryEvidence::new(
            "other-source:beam-12",
            Exactness::Approximate,
            "mesh://other-source/beam-12",
        ))
        .expect_err("cross-source evidence must not be silently accepted");

    assert!(matches!(error, AxiolidError::ForeignSubject { .. }));
}

#[test]
fn unavailable_kernel_is_an_explicit_failure() {
    let error = UnavailableGeometryBackend
        .resolve("cad-export-7:beam-12")
        .expect_err("missing geometry kernel must not fabricate evidence");

    assert!(matches!(error, AxiolidError::IntegrationUnavailable { .. }));
}
