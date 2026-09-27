//! Contract tests for relationships derived from geometry: routing through
//! the session's relationship selection, and evidence that names the
//! derivation.

use std::sync::{Arc, Mutex};

use axioval_engine::{
    CompleteRelationshipSelection, Derivation, DerivedRelationshipService,
    DerivedRelationshipServiceHandle, EvidenceSession, EvidenceSessionError, RelationshipQuery,
    RelationshipSelectionError, RelationshipSelectionRequest, RelationshipSelectionService,
    RelationshipSelectionServiceHandle, SemanticRelationship, ServiceRegistryError, SourceSnapshot,
    TraversalDirection,
};
use axioval_ir::{Evidence, Object, ObjectId, Project, SourceId};

fn source() -> SourceId {
    SourceId::new("test", "model").unwrap()
}

fn object(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn snapshot() -> SourceSnapshot {
    SourceSnapshot::try_new(source(), "revision-1", "sha256:01").unwrap()
}

fn project() -> Project {
    Project::new(vec![
        Object::new(object("door"), "DOOR"),
        Object::new(object("room"), "SPACE"),
    ])
    .unwrap()
}

fn request(relationship: &str) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        object("door"),
        vec![object("room")],
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new(relationship).unwrap(),
            direction: TraversalDirection::Forward,
            follow_chain: false,
        },
    )
    .unwrap()
}

/// Answers every request with `room`, citing `locator`, and records the
/// derivation it was asked for.
struct Deriving {
    locator: &'static str,
    asked: Mutex<Vec<Derivation>>,
    snapshots: Vec<SourceSnapshot>,
}

impl DerivedRelationshipService for Deriving {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn derive(
        &self,
        derivation: &Derivation,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        self.asked.lock().unwrap().push(*derivation);
        CompleteRelationshipSelection::try_new(
            request.clone(),
            vec![object("room")],
            vec![Evidence::exact(source(), self.locator)],
        )
    }
}

struct Semantic(Vec<SourceSnapshot>);

impl RelationshipSelectionService for Semantic {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        CompleteRelationshipSelection::try_new(
            request.clone(),
            Vec::new(),
            vec![Evidence::exact(source(), "relationship-scan:semantic")],
        )
    }
}

fn deriving(locator: &'static str) -> (Arc<Deriving>, DerivedRelationshipServiceHandle) {
    let service = Arc::new(Deriving {
        locator,
        asked: Mutex::new(Vec::new()),
        snapshots: vec![snapshot()],
    });
    (
        service.clone(),
        DerivedRelationshipServiceHandle::new(service),
    )
}

#[test]
fn the_session_routes_derived_identities_and_keeps_the_semantic_ones() {
    let (service, handle) = deriving("axioval:derived.adjacent-space:door->room:side=+");
    let session = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_service(RelationshipSelectionServiceHandle::new(Arc::new(Semantic(
            vec![snapshot()],
        ))))
        .unwrap()
        .with_derived_relationships(handle, &[snapshot()])
        .unwrap();
    let relationships = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap();

    let derived = relationships
        .select(&request("axioval:derived.adjacent-space;reach=0.5"))
        .unwrap();
    assert_eq!(derived.candidates(), [object("room")]);
    assert_eq!(
        service.asked.lock().unwrap().as_slice(),
        [Derivation::AdjacentSpace { reach_metres: 0.5 }]
    );

    let semantic = relationships
        .select(&request("IfcRelContainedInSpatialStructure"))
        .unwrap();
    assert!(semantic.candidates().is_empty());
    assert_eq!(
        service.asked.lock().unwrap().len(),
        1,
        "not routed to geometry"
    );
    assert!(
        session
            .service::<DerivedRelationshipServiceHandle>()
            .is_some()
    );
}

#[test]
fn without_a_semantic_service_only_derived_identities_are_answered() {
    let (_, handle) = deriving("axioval:derived.contained-in-space:door->room:contains");
    let session = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_derived_relationships(handle, &[snapshot()])
        .unwrap();
    let relationships = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap();
    assert!(
        relationships
            .select(&request("axioval:derived.contained-in-space"))
            .is_ok()
    );
    assert!(matches!(
        relationships.select(&request("IfcRelAggregates")),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
}

#[test]
fn evidence_that_does_not_name_the_derivation_is_refused() {
    let (_, handle) = deriving("relationship-scan:somewhere-else");
    assert_eq!(
        handle
            .select(&request("axioval:derived.contained-in-space"))
            .unwrap_err(),
        RelationshipSelectionError::InexactEvidence
    );
    let (_, handle) = deriving("axioval:derived.adjacent-space:door->room");
    assert_eq!(
        handle
            .select(&request("axioval:derived.contained-in-space"))
            .unwrap_err(),
        RelationshipSelectionError::InexactEvidence,
        "another derivation's evidence"
    );
}

#[test]
fn an_identity_naming_no_derivation_is_an_invalid_request() {
    let (service, handle) = deriving("axioval:derived.contained-in-space");
    for identity in ["IfcRelAggregates", "axioval:derived.unknown"] {
        assert_eq!(
            handle.select(&request(identity)).unwrap_err(),
            RelationshipSelectionError::InvalidRequest,
            "{identity}"
        );
    }
    assert!(service.asked.lock().unwrap().is_empty());
}

#[test]
fn a_second_derived_service_and_a_stale_binding_are_refused() {
    let (_, first) = deriving("axioval:derived.contained-in-space");
    let (_, second) = deriving("axioval:derived.contained-in-space");
    let twice = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_derived_relationships(first.clone(), &[snapshot()])
        .unwrap()
        .with_derived_relationships(second, &[snapshot()]);
    assert!(matches!(
        twice,
        Err(EvidenceSessionError::ServiceRegistry(
            ServiceRegistryError::Duplicate
        ))
    ));
    let stale = SourceSnapshot::try_new(source(), "revision-0", "sha256:00").unwrap();
    let refused = EvidenceSession::try_new(project(), [snapshot()])
        .unwrap()
        .with_derived_relationships(first, &[stale]);
    assert!(matches!(
        refused,
        Err(EvidenceSessionError::ServiceSnapshotMismatch(_))
    ));
}
