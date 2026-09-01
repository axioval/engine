use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axioval_ir::{Project, SourceId};
use thiserror::Error;

use crate::{ServiceRegistry, ServiceRegistryError};

/// Trusted service that declares the immutable source snapshots it can resolve.
///
/// Session registration validates these identities against the session before
/// exposing the service to evaluation. A service may cover a subset of a
/// multi-source session, but every declared binding must match exactly.
pub trait SnapshotBoundService: Any + Send + Sync {
    /// Exact source snapshots used to construct this service.
    fn source_snapshots(&self) -> &[SourceSnapshot];
}

/// Immutable identity of one source revision in an evidence session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshot {
    source: SourceId,
    revision: Arc<str>,
    fingerprint: Arc<str>,
    schema: Option<Arc<str>>,
}

impl SourceSnapshot {
    /// Creates an exact source snapshot identity.
    pub fn try_new(
        source: SourceId,
        revision: impl Into<Arc<str>>,
        fingerprint: impl Into<Arc<str>>,
    ) -> Result<Self, EvidenceSessionError> {
        let revision = revision.into();
        let fingerprint = fingerprint.into();
        if revision.trim().is_empty() || fingerprint.trim().is_empty() {
            return Err(EvidenceSessionError::InvalidSnapshotIdentity);
        }
        Ok(Self {
            source,
            revision,
            fingerprint,
            schema: None,
        })
    }
    /// Binds a source-declared semantic schema to the immutable snapshot.
    pub fn with_schema(
        mut self,
        schema: impl Into<Arc<str>>,
    ) -> Result<Self, EvidenceSessionError> {
        let schema = schema.into();
        if schema.trim().is_empty() {
            return Err(EvidenceSessionError::InvalidSnapshotIdentity);
        }
        self.schema = Some(schema);
        Ok(self)
    }
    /// Stable source identity.
    pub fn source(&self) -> &SourceId {
        &self.source
    }
    /// Adapter-defined immutable revision.
    pub fn revision(&self) -> &str {
        &self.revision
    }
    /// Content fingerprint, including its algorithm when applicable.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    /// Source-declared semantic schema, when the adapter has one.
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }
}

/// Invalid project/source snapshot binding.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EvidenceSessionError {
    /// Snapshot revision or fingerprint is empty.
    #[error("source snapshot revision and fingerprint must be non-empty")]
    InvalidSnapshotIdentity,
    /// Two snapshot declarations name the same source.
    #[error("duplicate source snapshot: {0}")]
    DuplicateSource(SourceId),
    /// A project source has no immutable snapshot declaration.
    #[error("project source has no snapshot declaration: {0}")]
    MissingSource(SourceId),
    /// A snapshot does not correspond to any project object.
    #[error("snapshot source is not present in the project: {0}")]
    UnexpectedSource(SourceId),
    /// A service declared no immutable source binding.
    #[error("evidence service has no source snapshot binding")]
    UnboundService,
    /// A service declared the same source binding more than once.
    #[error("evidence service has duplicate source snapshot binding: {0}")]
    DuplicateServiceSource(SourceId),
    /// A service source is absent from the session or has a different identity.
    #[error("evidence service snapshot does not match the session: {0}")]
    ServiceSnapshotMismatch(SourceId),
    /// Typed service registration failed.
    #[error(transparent)]
    ServiceRegistry(#[from] ServiceRegistryError),
}

/// Immutable project snapshot bound to the exact host services that produced
/// and can resolve its evidence.
///
/// Adapters build a session once per source snapshot. Runtime evaluation then
/// consumes the project and services as one unit, preventing accidental use of
/// a resolver from a different model revision.
pub struct EvidenceSession {
    project: Arc<Project>,
    snapshots: BTreeMap<SourceId, SourceSnapshot>,
    services: ServiceRegistry,
}

impl EvidenceSession {
    /// Starts a session after proving every project source has exactly one snapshot.
    pub fn try_new(
        project: Project,
        snapshots: impl IntoIterator<Item = SourceSnapshot>,
    ) -> Result<Self, EvidenceSessionError> {
        let project_sources = project
            .objects()
            .map(|object| object.id.source.clone())
            .collect::<BTreeSet<_>>();
        let mut indexed = BTreeMap::new();
        for snapshot in snapshots {
            let source = snapshot.source.clone();
            if indexed.insert(source.clone(), snapshot).is_some() {
                return Err(EvidenceSessionError::DuplicateSource(source));
            }
        }
        if let Some(source) = project_sources
            .difference(&indexed.keys().cloned().collect())
            .next()
        {
            return Err(EvidenceSessionError::MissingSource(source.clone()));
        }
        if let Some(source) = indexed
            .keys()
            .find(|source| !project_sources.contains(*source))
        {
            return Err(EvidenceSessionError::UnexpectedSource((*source).clone()));
        }
        Ok(Self {
            project: Arc::new(project),
            snapshots: indexed,
            services: ServiceRegistry::new(),
        })
    }

    /// Registers one non-replaceable typed evidence service.
    pub fn with_service<T: SnapshotBoundService>(
        mut self,
        service: T,
    ) -> Result<Self, EvidenceSessionError> {
        let bindings = service.source_snapshots();
        if bindings.is_empty() {
            return Err(EvidenceSessionError::UnboundService);
        }
        let mut sources = BTreeSet::new();
        for binding in bindings {
            if !sources.insert(binding.source.clone()) {
                return Err(EvidenceSessionError::DuplicateServiceSource(
                    binding.source.clone(),
                ));
            }
            if self.snapshots.get(&binding.source) != Some(binding) {
                return Err(EvidenceSessionError::ServiceSnapshotMismatch(
                    binding.source.clone(),
                ));
            }
        }
        self.services.register(service)?;
        Ok(self)
    }

    /// Returns the immutable project snapshot.
    #[must_use]
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Returns all immutable source identities bound to the project.
    pub fn snapshots(&self) -> impl ExactSizeIterator<Item = &SourceSnapshot> {
        self.snapshots.values()
    }

    /// Returns one source snapshot identity.
    #[must_use]
    pub fn snapshot(&self, source: &SourceId) -> Option<&SourceSnapshot> {
        self.snapshots.get(source)
    }

    /// Returns all services bound to this snapshot.
    #[must_use]
    pub fn services(&self) -> &ServiceRegistry {
        &self.services
    }

    /// Returns one typed service bound to this snapshot.
    #[must_use]
    pub fn service<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.services.get::<T>()
    }
}
