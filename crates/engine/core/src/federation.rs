//! Routing of federated sessions' semantic services by source.
//!
//! A federated session holds several sources, each with the services its own
//! adapter registered. The registry holds one service per interface, so each
//! interface gets one router that sends every request to the member covering
//! the request's source. A router never answers for a source itself: a source
//! no member covers is refused, never answered empty.

use std::{collections::BTreeSet, sync::Arc};

use axioval_ir::{Object, ObjectId, SourceId};

use crate::{
    ClassificationAssignment, ClassificationError, ClassificationService,
    ClassificationServiceHandle, CompleteRelationshipEdges, CompleteRelationshipSelection,
    CoordinateSystemError, CoordinateSystemService, CoordinateSystemServiceHandle, DoorLeaves,
    DoorLeavesError, EvidenceSessionError, IntegrityError, IntegrityIssue, ObjectFrame,
    ObjectFrameError, ObjectFrameService, ObjectFrameServiceHandle, PropertyEnumeration,
    PropertyEnumerationRequest, PropertyRequest, PropertyResolution, PropertyResolutionError,
    PropertyResolutionService, PropertyResolutionServiceHandle, RelationshipEdgesRequest,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionService,
    RelationshipSelectionServiceHandle, ResourceError, ResourceRequest, ResourceService,
    ResourceServiceHandle, ServiceRegistry, SnapshotBoundService, SourceCoordinateSystem,
    SourceIntegrityService, SourceIntegrityServiceHandle, SourceSnapshot,
    TypeHierarchyServiceHandle,
};

/// How many of `registry`'s services federation can route.
pub(crate) fn routed(registry: &ServiceRegistry) -> usize {
    usize::from(registry.get::<PropertyResolutionServiceHandle>().is_some())
        + usize::from(
            registry
                .get::<RelationshipSelectionServiceHandle>()
                .is_some(),
        )
        + usize::from(registry.get::<TypeHierarchyServiceHandle>().is_some())
        + usize::from(registry.get::<ClassificationServiceHandle>().is_some())
        + usize::from(registry.get::<SourceIntegrityServiceHandle>().is_some())
        + usize::from(registry.get::<ObjectFrameServiceHandle>().is_some())
        + usize::from(registry.get::<CoordinateSystemServiceHandle>().is_some())
        + usize::from(registry.get::<ResourceServiceHandle>().is_some())
}

/// Registers one router per semantic interface any member provides.
pub(crate) fn register(
    target: &mut ServiceRegistry,
    members: &[&ServiceRegistry],
) -> Result<(), EvidenceSessionError> {
    if let Some(router) = Router::<PropertyResolutionServiceHandle>::of(members) {
        target.register(PropertyResolutionServiceHandle::new(Arc::new(router)))?;
    }
    if let Some(router) = Router::<RelationshipSelectionServiceHandle>::of(members) {
        target.register(RelationshipSelectionServiceHandle::new(Arc::new(router)))?;
    }
    if let Some(router) = Router::<ClassificationServiceHandle>::of(members) {
        target.register(ClassificationServiceHandle::new(Arc::new(router)))?;
    }
    if let Some(router) = Router::<SourceIntegrityServiceHandle>::of(members) {
        target.register(SourceIntegrityServiceHandle::new(Arc::new(router)))?;
    }
    if let Some(router) = Router::<ObjectFrameServiceHandle>::of(members) {
        target.register(ObjectFrameServiceHandle::new(Arc::new(router)))?;
    }
    if let Some(router) = Router::<CoordinateSystemServiceHandle>::of(members) {
        target.register(CoordinateSystemServiceHandle::new(Arc::new(router)))?;
    }
    if let Some(router) = Router::<ResourceServiceHandle>::of(members) {
        target.register(ResourceServiceHandle::new(Arc::new(router)))?;
    }
    let hierarchies: Vec<&TypeHierarchyServiceHandle> = members
        .iter()
        .filter_map(|registry| registry.get::<TypeHierarchyServiceHandle>())
        .collect();
    if !hierarchies.is_empty() {
        target.register(TypeHierarchyServiceHandle::federated(&hierarchies))?;
    }
    Ok(())
}

/// Members' handles of one interface, with the snapshots each covers.
struct Router<T> {
    members: Vec<(BTreeSet<SourceId>, T)>,
    snapshots: Vec<SourceSnapshot>,
}

impl<T: SnapshotBoundService + Clone> Router<T> {
    fn of(registries: &[&ServiceRegistry]) -> Option<Self> {
        let members: Vec<(BTreeSet<SourceId>, T)> = registries
            .iter()
            .filter_map(|registry| registry.get::<T>())
            .map(|handle| {
                let sources = handle
                    .source_snapshots()
                    .iter()
                    .map(|snapshot| snapshot.source().clone())
                    .collect();
                (sources, handle.clone())
            })
            .collect();
        if members.is_empty() {
            return None;
        }
        let snapshots = members
            .iter()
            .flat_map(|(_, handle)| handle.source_snapshots().iter().cloned())
            .collect();
        Some(Self { members, snapshots })
    }

    fn member(&self, source: &SourceId) -> Option<(&BTreeSet<SourceId>, &T)> {
        self.members
            .iter()
            .find(|(sources, _)| sources.contains(source))
            .map(|(sources, handle)| (sources, handle))
    }
}

fn uncovered(source: &SourceId) -> String {
    format!("no federated service covers source `{source}`")
}

impl PropertyResolutionService for Router<PropertyResolutionServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        let source = &request.object_id().source;
        let (_, member) = self
            .member(source)
            .ok_or_else(|| PropertyResolutionError::Unavailable(uncovered(source)))?;
        member.resolve(request)
    }
    fn enumerate(
        &self,
        request: &PropertyEnumerationRequest,
    ) -> Result<PropertyEnumeration, PropertyResolutionError> {
        let source = &request.object_id().source;
        let (_, member) = self
            .member(source)
            .ok_or_else(|| PropertyResolutionError::Unavailable(uncovered(source)))?;
        member.enumerate(request)
    }
}

impl RelationshipSelectionService for Router<RelationshipSelectionServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let source = &request.anchor().source;
        let (sources, member) = self
            .member(source)
            .ok_or_else(|| RelationshipSelectionError::Unavailable(uncovered(source)))?;
        // A member cannot relate objects it does not hold, so it is asked
        // over its own part of the universe and its answer is bound back to
        // the whole request.
        let universe: Vec<ObjectId> = request
            .candidate_universe()
            .iter()
            .filter(|candidate| sources.contains(&candidate.source))
            .cloned()
            .collect();
        let narrowed = RelationshipSelectionRequest::try_new(
            request.anchor().clone(),
            universe,
            request.query().clone(),
        )?
        .with_absent_ends(request.absent_ends());
        let selection = member.select(&narrowed)?;
        CompleteRelationshipSelection::try_new(
            request.clone(),
            selection.candidates().to_vec(),
            selection.evidence().to_vec(),
        )
    }

    /// Each member lists the edges among its own part of the universe; no
    /// member relates objects it does not hold, so the union is complete.
    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        if let Some(object) = request
            .universe()
            .iter()
            .find(|object| self.member(&object.source).is_none())
        {
            return Err(RelationshipSelectionError::Unavailable(uncovered(
                &object.source,
            )));
        }
        let mut edges = Vec::new();
        let mut evidence = Vec::new();
        for (sources, member) in &self.members {
            let universe: Vec<ObjectId> = request
                .universe()
                .iter()
                .filter(|object| sources.contains(&object.source))
                .cloned()
                .collect();
            if universe.is_empty() {
                continue;
            }
            let narrowed =
                RelationshipEdgesRequest::try_new(universe, request.relationship().clone())?
                    .with_absent_ends(request.absent_ends());
            let listing = member.edges(&narrowed)?;
            edges.extend(listing.edges().iter().cloned());
            evidence.extend(listing.evidence().iter().cloned());
        }
        CompleteRelationshipEdges::try_new(request.clone(), edges, evidence)
    }
}

impl ClassificationService for Router<ClassificationServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn classifications(
        &self,
        object: &ObjectId,
    ) -> Result<Vec<ClassificationAssignment>, ClassificationError> {
        let (_, member) = self
            .member(&object.source)
            .ok_or_else(|| ClassificationError::UncoveredSource(object.source.clone()))?;
        member.classifications(object)
    }
}

impl SourceIntegrityService for Router<SourceIntegrityServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn issues(&self, source: &SourceId) -> Result<Vec<IntegrityIssue>, IntegrityError> {
        let (_, member) = self
            .member(source)
            .ok_or_else(|| IntegrityError::UncoveredSource(source.clone()))?;
        member.issues(source)
    }
}

impl ObjectFrameService for Router<ObjectFrameServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn object_frame(&self, object: &ObjectId) -> Result<ObjectFrame, ObjectFrameError> {
        let (_, member) = self
            .member(&object.source)
            .ok_or_else(|| ObjectFrameError::UncoveredSource(object.source.clone()))?;
        member.object_frame(object)
    }
    fn leaves(&self, door: &ObjectId) -> Result<DoorLeaves, DoorLeavesError> {
        let (_, member) = self
            .member(&door.source)
            .ok_or_else(|| DoorLeavesError::UncoveredSource(door.source.clone()))?;
        member.leaves(door)
    }
}

impl CoordinateSystemService for Router<CoordinateSystemServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn coordinate_system(
        &self,
        source: &SourceId,
    ) -> Result<SourceCoordinateSystem, CoordinateSystemError> {
        let (_, member) = self
            .member(source)
            .ok_or_else(|| CoordinateSystemError::UncoveredSource(source.clone()))?;
        member.coordinate_system(source)
    }
}

impl ResourceService for Router<ResourceServiceHandle> {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn resources(&self, request: &ResourceRequest) -> Result<Vec<Object>, ResourceError> {
        let (_, member) = self
            .member(request.source())
            .ok_or_else(|| ResourceError::UncoveredSource(request.source().clone()))?;
        member.resources(request)
    }
}
