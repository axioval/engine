//! ICDD project-assembly contracts.
//!
//! Assembly records project documents and declared inter-document links. It deliberately
//! does not define federation, semantic identity, or rule execution; those remain engine
//! concerns once their stable contracts exist.

use std::collections::BTreeSet;

use thiserror::Error;

/// A project document with a stable assembly-local identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    id: String,
    media_type: String,
    digest: String,
}

impl Document {
    /// Creates a document descriptor without reading its bytes.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        media_type: impl Into<String>,
        digest: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            media_type: media_type.into(),
            digest: digest.into(),
        }
    }
    /// Returns the assembly-local document identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Returns the declared media type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }
    /// Returns the producer-supplied content digest string.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// A declared directed relationship between two documents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectLink {
    from: String,
    to: String,
    relation: String,
}

impl ProjectLink {
    /// Creates a link; endpoint existence is verified by assembly.
    #[must_use]
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        relation: impl Into<String>,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            relation: relation.into(),
        }
    }
    /// Returns the origin document ID.
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }
    /// Returns the target document ID.
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }
    /// Returns the un-interpreted declared relationship.
    #[must_use]
    pub fn relation(&self) -> &str {
        &self.relation
    }
}

/// Validated project assembly with no federation behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAssembly {
    documents: Vec<Document>,
    links: Vec<ProjectLink>,
}

impl ProjectAssembly {
    /// Returns documents in the supplied manifest order.
    #[must_use]
    pub fn documents(&self) -> &[Document] {
        &self.documents
    }
    /// Returns links in the supplied manifest order.
    #[must_use]
    pub fn links(&self) -> &[ProjectLink] {
        &self.links
    }
}

/// Assembles an ICDD project manifest from external container data.
pub trait ProjectAssembler {
    /// Validates references without parsing or federating project contents.
    ///
    /// # Errors
    ///
    /// Returns [`ICDDAssemblyError::EmptyDocumentId`],
    /// [`ICDDAssemblyError::DuplicateDocument`], or
    /// [`ICDDAssemblyError::UnknownDocument`] for an invalid manifest.
    fn assemble(
        &self,
        documents: impl IntoIterator<Item = Document>,
        links: impl IntoIterator<Item = ProjectLink>,
    ) -> Result<ProjectAssembly, ICDDAssemblyError>;
}

/// In-memory conformance double for ICDD manifest assembly.
#[derive(Clone, Debug, Default)]
pub struct InMemoryProjectAssembler;

impl InMemoryProjectAssembler {
    /// Creates an in-memory manifest assembler.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ProjectAssembler for InMemoryProjectAssembler {
    fn assemble(
        &self,
        documents: impl IntoIterator<Item = Document>,
        links: impl IntoIterator<Item = ProjectLink>,
    ) -> Result<ProjectAssembly, ICDDAssemblyError> {
        let documents: Vec<_> = documents.into_iter().collect();
        let mut ids = BTreeSet::new();
        for document in &documents {
            if document.id.is_empty() {
                return Err(ICDDAssemblyError::EmptyDocumentId);
            }
            if !ids.insert(document.id.clone()) {
                return Err(ICDDAssemblyError::DuplicateDocument {
                    id: document.id.clone(),
                });
            }
        }
        let links: Vec<_> = links.into_iter().collect();
        for link in &links {
            if !ids.contains(&link.from) {
                return Err(ICDDAssemblyError::UnknownDocument {
                    id: link.from.clone(),
                });
            }
            if !ids.contains(&link.to) {
                return Err(ICDDAssemblyError::UnknownDocument {
                    id: link.to.clone(),
                });
            }
        }
        Ok(ProjectAssembly { documents, links })
    }
}

/// Isolated seam for an ISO 21597-1 container reader.
pub trait IcddContainerReader {
    /// Reads external container data into an already-validated assembly.
    ///
    /// # Errors
    ///
    /// Returns a reader failure, including [`ICDDAssemblyError::IntegrationUnavailable`].
    fn read(&self, locator: &str) -> Result<ProjectAssembly, ICDDAssemblyError>;
}

/// Explicit placeholder used until a container parser is selected and linked.
#[derive(Clone, Debug, Default)]
pub struct UnavailableIcddContainerReader;

impl IcddContainerReader for UnavailableIcddContainerReader {
    fn read(&self, _locator: &str) -> Result<ProjectAssembly, ICDDAssemblyError> {
        Err(ICDDAssemblyError::IntegrationUnavailable {
            integration: "ISO 21597-1 ICDD container reader",
        })
    }
}

/// Errors produced by project assembly.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ICDDAssemblyError {
    /// Document IDs are required for deterministic link validation.
    #[error("project document ID must not be empty")]
    EmptyDocumentId,
    /// Multiple descriptors claimed an assembly-local ID.
    #[error("duplicate project document: {id}")]
    DuplicateDocument {
        /// Duplicate assembly-local document identifier.
        id: String,
    },
    /// A declared relationship points at no declared document.
    #[error("declared link references unknown project document: {id}")]
    UnknownDocument {
        /// Referenced but undeclared assembly-local document identifier.
        id: String,
    },
    /// A real ISO container reader has not been integrated.
    #[error("external integration unavailable: {integration}")]
    IntegrationUnavailable {
        /// Named external component that has not been linked.
        integration: &'static str,
    },
}
