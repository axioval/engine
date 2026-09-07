#![allow(clippy::doc_markdown)]

//! Geometry-evidence adapter contracts independent of `OpenBIM`.
//!
//! Axiolid consumes source-qualified subjects, so proprietary CAD, mesh, and B-rep
//! producers can participate without an IFC dependency. No geometry kernel is silently
//! substituted: missing backends return [`AxiolidError::IntegrationUnavailable`].

use std::collections::BTreeMap;

use thiserror::Error;

/// Stable identity scope for a geometry-producing source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceIdentity(String);

impl SourceIdentity {
    /// Creates a source identity.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the source identity text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Declares whether an item is exact B-rep evidence or an approximation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exactness {
    /// The producer asserts that this is exact geometry evidence.
    Exact,
    /// The producer asserts that this evidence is an approximation.
    Approximate,
}

/// Geometry evidence for one source-qualified subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeometryEvidence {
    subject_id: String,
    exactness: Exactness,
    provenance: String,
}

impl GeometryEvidence {
    /// Creates evidence. `subject_id` may be local or already source-qualified.
    #[must_use]
    pub fn new(
        subject_id: impl Into<String>,
        exactness: Exactness,
        provenance: impl Into<String>,
    ) -> Self {
        Self {
            subject_id: subject_id.into(),
            exactness,
            provenance: provenance.into(),
        }
    }

    /// Returns the source-qualified subject identifier after insertion.
    #[must_use]
    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }

    /// Returns the fidelity declaration without upgrading approximate geometry.
    #[must_use]
    pub fn exactness(&self) -> Exactness {
        self.exactness
    }

    /// Returns the opaque origin locator for the supplied evidence.
    #[must_use]
    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

/// Read-only geometry evidence lookup.
pub trait GeometrySource {
    /// Returns the configured identity scope.
    fn source_identity(&self) -> &SourceIdentity;
    /// Looks up evidence by a source-local subject ID.
    fn geometry_for(&self, local_id: &str) -> Option<&GeometryEvidence>;
}

/// In-memory geometry adapter used to conformance-test any CAD source.
#[derive(Clone, Debug)]
pub struct InMemoryGeometrySource {
    identity: SourceIdentity,
    evidence: BTreeMap<String, GeometryEvidence>,
}

impl InMemoryGeometrySource {
    /// Creates an initially empty evidence source.
    #[must_use]
    pub fn new(identity: SourceIdentity) -> Self {
        Self {
            identity,
            evidence: BTreeMap::new(),
        }
    }

    /// Inserts evidence only when its subject belongs to this source.
    ///
    /// # Errors
    ///
    /// Returns [`AxiolidError::ForeignSubject`] for an ID qualified by another
    /// source, or [`AxiolidError::EmptySubjectId`] for an empty local ID.
    pub fn insert(&mut self, mut evidence: GeometryEvidence) -> Result<(), AxiolidError> {
        let prefix = format!("{}:", self.identity.as_str());
        if evidence.subject_id.contains(':') && !evidence.subject_id.starts_with(&prefix) {
            return Err(AxiolidError::ForeignSubject {
                subject_id: evidence.subject_id,
            });
        }
        let local_id = evidence
            .subject_id
            .strip_prefix(&prefix)
            .unwrap_or(&evidence.subject_id)
            .to_owned();
        if local_id.is_empty() {
            return Err(AxiolidError::EmptySubjectId);
        }
        evidence.subject_id = format!("{prefix}{local_id}");
        self.evidence.insert(local_id, evidence);
        Ok(())
    }
}

impl GeometrySource for InMemoryGeometrySource {
    fn source_identity(&self) -> &SourceIdentity {
        &self.identity
    }
    fn geometry_for(&self, local_id: &str) -> Option<&GeometryEvidence> {
        self.evidence.get(local_id)
    }
}

/// Isolated seam for a real geometry kernel or CAD SDK.
pub trait GeometryBackend {
    /// Resolves one subject's evidence through the external backend.
    ///
    /// # Errors
    ///
    /// Returns a backend failure, including [`AxiolidError::IntegrationUnavailable`].
    fn resolve(&self, subject_id: &str) -> Result<GeometryEvidence, AxiolidError>;
}

/// Explicit placeholder used while no external geometry backend is linked.
#[derive(Clone, Debug, Default)]
pub struct UnavailableGeometryBackend;

impl GeometryBackend for UnavailableGeometryBackend {
    fn resolve(&self, _subject_id: &str) -> Result<GeometryEvidence, AxiolidError> {
        Err(AxiolidError::IntegrationUnavailable {
            integration: "Axiolid geometry kernel",
        })
    }
}

/// Errors from geometry evidence adaptation.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum AxiolidError {
    /// An empty source-local subject cannot be safely qualified.
    #[error("geometry subject ID must not be empty")]
    EmptySubjectId,
    /// Evidence belonged to a different source scope.
    #[error("geometry evidence belongs to a foreign subject: {subject_id}")]
    ForeignSubject {
        /// The supplied source-qualified subject identity.
        subject_id: String,
    },
    /// A required kernel or CAD SDK integration is deliberately not implemented.
    #[error("external integration unavailable: {integration}")]
    IntegrationUnavailable {
        /// Named external component that has not been linked.
        integration: &'static str,
    },
}
