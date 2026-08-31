//! Conformance tests for the `OpenBIM` semantic adapter contract.

use axioval_openbim::{
    InMemorySemanticSource, OpenBimError, OpenBimImportRequest, OpenBimImporter, SemanticEntity,
    SemanticSource, SourceDescriptor, UnavailableOpenBimImporter,
};

#[test]
fn semantic_source_preserves_source_qualified_ids_and_stable_entity_order() {
    let source = InMemorySemanticSource::new(
        SourceDescriptor::new("ifc-a", "IFC4"),
        [
            SemanticEntity::new("wall-2", "IfcWall"),
            SemanticEntity::new("wall-1", "IfcWall"),
        ],
    )
    .expect("distinct local identifiers are valid");

    assert_eq!(source.descriptor().source_id(), "ifc-a");
    assert_eq!(source.descriptor().schema(), "IFC4");
    assert_eq!(
        source
            .entities()
            .map(|entity| entity.qualified_id().to_string())
            .collect::<Vec<_>>(),
        ["ifc-a:wall-2", "ifc-a:wall-1"]
    );
}

#[test]
fn semantic_source_rejects_duplicate_local_ids() {
    let error = InMemorySemanticSource::new(
        SourceDescriptor::new("ifc-a", "IFC4"),
        [
            SemanticEntity::new("wall-1", "IfcWall"),
            SemanticEntity::new("wall-1", "IfcWall"),
        ],
    )
    .expect_err("ambiguous source identities must fail closed");

    assert!(matches!(error, OpenBimError::DuplicateEntityId { .. }));
}

#[test]
fn unavailable_importer_is_an_explicit_failure() {
    let error = UnavailableOpenBimImporter
        .import(&OpenBimImportRequest::new("model.ifc"))
        .expect_err("missing upstream parser must not pretend to import");

    assert!(matches!(error, OpenBimError::IntegrationUnavailable { .. }));
}
