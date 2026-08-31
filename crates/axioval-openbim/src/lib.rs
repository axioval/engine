#![allow(clippy::doc_markdown)]

//! Source-neutral `OpenBIM` semantic adapter contracts.
//!
//! This crate intentionally does not parse IFC/STEP yet. Concrete importers live behind
//! [`OpenBimImporter`], and the supplied unavailable importer fails explicitly until an
//! upstream parser is selected and integrated.

use std::collections::BTreeSet;

use thiserror::Error;

/// A source's stable identity and declared semantic schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDescriptor {
    source_id: String,
    schema: String,
}

impl SourceDescriptor {
    /// Creates a descriptor for one independently-addressable semantic source.
    #[must_use]
    pub fn new(source_id: impl Into<String>, schema: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            schema: schema.into(),
        }
    }

    /// Returns the stable source identifier used to qualify entity identities.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the schema declaration supplied by the source.
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }
}

/// One semantic entity exposed by an OpenBIM source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticEntity {
    local_id: String,
    type_name: String,
}

impl SemanticEntity {
    /// Creates an entity with an identifier unique within its containing source.
    #[must_use]
    pub fn new(local_id: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            local_id: local_id.into(),
            type_name: type_name.into(),
        }
    }

    /// Returns the source-local identifier.
    #[must_use]
    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    /// Returns the source-qualified identifier set by its containing source.
    #[must_use]
    pub fn qualified_id(&self) -> &str {
        &self.local_id
    }

    /// Returns the declared OpenBIM entity type.
    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }
}

/// Read-only semantic view that preserves input ordering and identity scope.
pub trait SemanticSource {
    /// Describes the source and its semantic schema.
    fn descriptor(&self) -> &SourceDescriptor;

    /// Iterates entities in the source's declared order.
    fn entities(&self) -> Box<dyn Iterator<Item = &SemanticEntity> + '_>;
}

/// In-memory conformance double for semantic sources.
#[derive(Clone, Debug)]
pub struct InMemorySemanticSource {
    descriptor: SourceDescriptor,
    entities: Vec<SemanticEntity>,
}

impl InMemorySemanticSource {
    /// Builds a source after qualifying each local entity identity with its source ID.
    ///
    /// # Errors
    ///
    /// Returns [`OpenBimError::EmptyEntityId`] for an empty local ID or
    /// [`OpenBimError::DuplicateEntityId`] for an ambiguous local identity.
    pub fn new(
        descriptor: SourceDescriptor,
        entities: impl IntoIterator<Item = SemanticEntity>,
    ) -> Result<Self, OpenBimError> {
        if descriptor.source_id.is_empty() {
            return Err(OpenBimError::EmptySourceId);
        }
        let mut ids = BTreeSet::new();
        let mut qualified = Vec::new();
        for entity in entities {
            if entity.local_id.is_empty() {
                return Err(OpenBimError::EmptyEntityId);
            }
            if !ids.insert(entity.local_id.clone()) {
                return Err(OpenBimError::DuplicateEntityId {
                    source_id: descriptor.source_id.clone(),
                    local_id: entity.local_id,
                });
            }
            qualified.push(SemanticEntity {
                local_id: format!("{}:{}", descriptor.source_id, entity.local_id),
                type_name: entity.type_name,
            });
        }
        Ok(Self {
            descriptor,
            entities: qualified,
        })
    }
}

impl SemanticSource for InMemorySemanticSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &self.descriptor
    }

    fn entities(&self) -> Box<dyn Iterator<Item = &SemanticEntity> + '_> {
        Box::new(self.entities.iter())
    }
}

/// A request for an external OpenBIM import implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenBimImportRequest {
    locator: String,
}

impl OpenBimImportRequest {
    /// Creates a request whose locator is interpreted by the concrete importer.
    #[must_use]
    pub fn new(locator: impl Into<String>) -> Self {
        Self {
            locator: locator.into(),
        }
    }

    /// Returns the opaque source locator.
    #[must_use]
    pub fn locator(&self) -> &str {
        &self.locator
    }
}

/// Integration seam for IFC/STEP or other OpenBIM parser implementations.
pub trait OpenBimImporter {
    /// Imports one source or returns an explicit integration failure.
    ///
    /// # Errors
    ///
    /// Returns an error reported by the external importer, including
    /// [`OpenBimError::IntegrationUnavailable`].
    fn import(
        &self,
        request: &OpenBimImportRequest,
    ) -> Result<InMemorySemanticSource, OpenBimError>;
}

/// Explicit placeholder used while no external OpenBIM parser is wired in.
#[derive(Clone, Debug, Default)]
pub struct UnavailableOpenBimImporter;

impl OpenBimImporter for UnavailableOpenBimImporter {
    fn import(
        &self,
        _request: &OpenBimImportRequest,
    ) -> Result<InMemorySemanticSource, OpenBimError> {
        Err(OpenBimError::IntegrationUnavailable {
            integration: "OpenBIM IFC/STEP importer",
        })
    }
}

/// Errors produced while forming or importing a semantic source.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum OpenBimError {
    /// A source ID is required to prevent cross-source identity collisions.
    #[error("semantic source ID must not be empty")]
    EmptySourceId,
    /// An entity must have a nonempty source-local ID.
    #[error("semantic entity ID must not be empty")]
    EmptyEntityId,
    /// Two entities shared the same local ID in one source.
    #[error("duplicate entity ID {local_id:?} in source {source_id:?}")]
    DuplicateEntityId {
        /// Source whose local identity was duplicated.
        source_id: String,
        /// Ambiguous identifier within `source_id`.
        local_id: String,
    },
    /// A concrete external parser has not been selected or linked.
    #[error("external integration unavailable: {integration}")]
    IntegrationUnavailable {
        /// Named external component that has not been linked.
        integration: &'static str,
    },
}
