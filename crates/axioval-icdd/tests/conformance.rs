//! Conformance tests for the ICDD project-assembly contract.

use axioval_icdd::{
    Document, ICDDAssemblyError, IcddContainerReader, InMemoryProjectAssembler, ProjectAssembler,
    ProjectLink, UnavailableIcddContainerReader,
};

#[test]
fn assembly_retains_documents_and_declared_links_without_federating_them() {
    let assembly = InMemoryProjectAssembler::new()
        .assemble(
            [
                Document::new("model", "application/ifc", "sha256:model"),
                Document::new("issues", "application/bcf+zip", "sha256:issues"),
            ],
            [ProjectLink::new("issues", "model", "references")],
        )
        .expect("a link among declared documents is a valid project assembly");

    assert_eq!(assembly.documents().len(), 2);
    assert_eq!(assembly.links().len(), 1);
    assert_eq!(assembly.links()[0].relation(), "references");
}

#[test]
fn assembly_rejects_links_to_undeclared_documents() {
    let error = InMemoryProjectAssembler::new()
        .assemble(
            [Document::new("model", "application/ifc", "sha256:model")],
            [ProjectLink::new("issues", "model", "references")],
        )
        .expect_err("ICDD assembly must not invent missing documents");

    assert!(matches!(error, ICDDAssemblyError::UnknownDocument { .. }));
}

#[test]
fn unavailable_container_reader_is_an_explicit_failure() {
    let error = UnavailableIcddContainerReader
        .read("project.icdd")
        .expect_err("missing ICDD parser must not fabricate an assembly");

    assert!(matches!(
        error,
        ICDDAssemblyError::IntegrationUnavailable { .. }
    ));
}
