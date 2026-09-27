use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axioval_ir::{Discipline, Project, SourceId};
use thiserror::Error;

use crate::derived_relationships::{DerivedRelationshipServiceHandle, RoutedRelationships};
use crate::{RelationshipSelectionServiceHandle, ServiceRegistry, ServiceRegistryError};

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
    type_systems: Vec<Arc<str>>,
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
            type_systems: Vec::new(),
        })
    }
    /// Declares one type system this source's vocabulary uses.
    ///
    /// Package concepts bind to source data only through an external name in
    /// a declared type system. A source usually speaks one release-bound
    /// system (an IFC4 model: IFC4 entities and its property templates) and
    /// may add a project namespace for custom property sets. A source that
    /// declares none cannot bind any concept, so package rules over it are
    /// not evaluated rather than passed. Declaring a system twice is a no-op.
    pub fn with_type_system(
        mut self,
        type_system: impl Into<Arc<str>>,
    ) -> Result<Self, EvidenceSessionError> {
        let type_system = type_system.into();
        if type_system.trim().is_empty() {
            return Err(EvidenceSessionError::InvalidSnapshotIdentity);
        }
        if let Err(index) = self.type_systems.binary_search(&type_system) {
            self.type_systems.insert(index, type_system);
        }
        Ok(self)
    }
    /// Declared type systems for concept binding, sorted and unique.
    pub fn type_systems(&self) -> &[Arc<str>] {
        &self.type_systems
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

/// The disciplines a run's sources play, as the session declared them.
///
/// Only the engine constructs this, from the evidence session, and registers
/// it for the duration of one run, replacing any host-registered copy.
/// Capabilities find it in the service registry. A source without an entry
/// declares no discipline; that is unknown, never "no discipline matches".
#[derive(Clone, Debug, Default)]
pub struct SourceDisciplines(BTreeMap<SourceId, Discipline>);

impl SourceDisciplines {
    pub(crate) fn new(disciplines: BTreeMap<SourceId, Discipline>) -> Self {
        Self(disciplines)
    }

    /// The discipline declared for `source`, if any.
    #[must_use]
    pub fn of(&self, source: &SourceId) -> Option<&Discipline> {
        self.0.get(source)
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
    /// A discipline was declared for a source the session does not hold.
    #[error("discipline declared for a source outside the session: {0}")]
    UnknownSource(SourceId),
    /// A source's discipline was declared twice.
    #[error("source already declares a discipline: {0}")]
    DuplicateDiscipline(SourceId),
    /// A federated member holds a service the engine cannot route by source.
    ///
    /// Federation composes the semantic services adapters register; a host
    /// service (geometry, derived relationships) is registered on the
    /// federated session instead, bound to every snapshot it was built from.
    #[error("evidence session holds a service that cannot be federated")]
    UnfederableService,
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
    disciplines: BTreeMap<SourceId, Discipline>,
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
            disciplines: BTreeMap::new(),
            services: ServiceRegistry::new(),
        })
    }

    /// Combines sessions over disjoint sources into one session.
    ///
    /// The project holds every member's objects under their own
    /// source-qualified identities, and every snapshot and declared
    /// discipline is kept. Each semantic service the members registered
    /// (property resolution, relationship selection, type hierarchy, object
    /// frames, classifications, integrity) becomes one service bound to the
    /// snapshots of the members that had it, answering each request from
    /// the member that owns the request's source; a source no member
    /// covers is refused, never answered empty. A relationship request is
    /// answered from its anchor's member over the part of the candidate
    /// universe in that member's sources, since a member cannot relate
    /// objects it does not hold.
    ///
    /// One member is returned unchanged. Federate before registering host
    /// services such as geometry: they are built over the federated
    /// project and bound to all its snapshots.
    ///
    /// # Errors
    ///
    /// Returns an error when two members hold the same source, or a member
    /// holds a service other than the semantic ones above.
    pub fn federate(
        members: impl IntoIterator<Item = EvidenceSession>,
    ) -> Result<Self, EvidenceSessionError> {
        let mut members: Vec<Self> = members.into_iter().collect();
        if members.len() == 1 {
            return Ok(members.remove(0));
        }
        for member in &members {
            if member.services.len() > crate::federation::routed(&member.services) {
                return Err(EvidenceSessionError::UnfederableService);
            }
        }
        let objects = members
            .iter()
            .flat_map(|member| member.project.objects().cloned())
            .collect();
        let snapshots: Vec<SourceSnapshot> = members
            .iter()
            .flat_map(|member| member.snapshots.values().cloned())
            .collect();
        let project = Project::new(objects).map_err(|error| match error {
            // Objects of one source only ever come from its own member, so a
            // duplicate object means two members hold the same source.
            axioval_ir::IrError::DuplicateObject(object) => {
                EvidenceSessionError::DuplicateSource(object.source)
            }
            _ => EvidenceSessionError::InvalidSnapshotIdentity,
        })?;
        let mut federated = Self::try_new(project, snapshots)?;
        for member in &members {
            federated.disciplines.extend(
                member
                    .disciplines
                    .iter()
                    .map(|(source, discipline)| (source.clone(), discipline.clone())),
            );
        }
        let registries: Vec<&ServiceRegistry> =
            members.iter().map(|member| &member.services).collect();
        crate::federation::register(&mut federated.services, &registries)?;
        Ok(federated)
    }

    /// Declares the discipline `source` plays in this check.
    ///
    /// A discipline is a host declaration, not part of the snapshot
    /// identity: services bound to a snapshot stay valid whatever role the
    /// source plays. Capabilities read it through
    /// [`crate::SourceDisciplines`]; the `discipline` selector matches on it.
    ///
    /// # Errors
    ///
    /// Returns an error when the session holds no such source or it already
    /// declares a discipline.
    pub fn with_discipline(
        mut self,
        source: &SourceId,
        discipline: Discipline,
    ) -> Result<Self, EvidenceSessionError> {
        if !self.snapshots.contains_key(source) {
            return Err(EvidenceSessionError::UnknownSource(source.clone()));
        }
        if self.disciplines.contains_key(source) {
            return Err(EvidenceSessionError::DuplicateDiscipline(source.clone()));
        }
        self.disciplines.insert(source.clone(), discipline);
        Ok(self)
    }

    /// The discipline declared for `source`, if any.
    #[must_use]
    pub fn discipline(&self, source: &SourceId) -> Option<&Discipline> {
        self.disciplines.get(source)
    }

    /// Every declared discipline, by source.
    #[must_use]
    pub fn disciplines(&self) -> &BTreeMap<SourceId, Discipline> {
        &self.disciplines
    }

    /// Registers one non-replaceable typed evidence service.
    pub fn with_service<T: SnapshotBoundService>(
        mut self,
        service: T,
    ) -> Result<Self, EvidenceSessionError> {
        self.check_bindings(service.source_snapshots())?;
        self.services.register(service)?;
        Ok(self)
    }

    /// Registers a host service that records no snapshot of its own.
    ///
    /// Some services are built by the host from data it read alongside the
    /// session, such as geometry meshed from the same file, and carry only a
    /// source identity. The host states which of this session's snapshots
    /// the service was built from, and the same checks as
    /// [`Self::with_service`] apply: at least one binding, no source twice,
    /// and every binding equal to the session's snapshot for that source.
    /// A service built from another revision of a source is refused.
    ///
    /// # Errors
    ///
    /// Returns an error when a binding is missing, repeated or stale, or a
    /// service of this type is already registered.
    pub fn with_host_service<T: std::any::Any + Send + Sync>(
        mut self,
        service: T,
        built_from: &[SourceSnapshot],
    ) -> Result<Self, EvidenceSessionError> {
        self.check_bindings(built_from)?;
        self.services.register(service)?;
        Ok(self)
    }

    /// Registers a derived-relationship service and routes the session's
    /// relationship selection through it.
    ///
    /// Afterwards the session's [`RelationshipSelectionServiceHandle`]
    /// answers identities starting with
    /// [`crate::DERIVED_RELATIONSHIP_PREFIX`] from `service` and every other
    /// identity from the semantic service registered before, so capabilities
    /// taking a `relationship` or `path` use derived relationships unchanged.
    /// Register the semantic service first; one registered afterwards is a
    /// duplicate. `built_from` is checked as in [`Self::with_host_service`].
    ///
    /// # Errors
    ///
    /// Returns an error when a binding is missing, repeated or stale, or a
    /// derived-relationship service is already registered.
    pub fn with_derived_relationships(
        mut self,
        service: DerivedRelationshipServiceHandle,
        built_from: &[SourceSnapshot],
    ) -> Result<Self, EvidenceSessionError> {
        self.check_bindings(built_from)?;
        let semantic = self
            .services
            .get::<RelationshipSelectionServiceHandle>()
            .cloned();
        self.services.register(service.clone())?;
        let snapshots = semantic.as_ref().map_or_else(
            || built_from.to_vec(),
            |semantic| semantic.source_snapshots().to_vec(),
        );
        self.services
            .replace(RelationshipSelectionServiceHandle::new(Arc::new(
                RoutedRelationships {
                    semantic,
                    derived: service,
                    snapshots,
                },
            )));
        Ok(self)
    }

    fn check_bindings(&self, bindings: &[SourceSnapshot]) -> Result<(), EvidenceSessionError> {
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
        Ok(())
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
