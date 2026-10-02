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

/// Edges `(from, to)` of one relationship, answered in either direction.
fn walk(
    edges: &[(&str, &str)],
    request: &RelationshipSelectionRequest,
    locator: &str,
) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
    let RelationshipQuery::Related { direction, .. } = request.query() else {
        return Err(RelationshipSelectionError::InvalidRequest);
    };
    let anchor = request.anchor().local_id.as_str();
    let reached: Vec<ObjectId> = edges
        .iter()
        .filter_map(|(from, to)| match direction {
            TraversalDirection::Forward => (*from == anchor).then_some(*to),
            TraversalDirection::Backward => (*to == anchor).then_some(*from),
            TraversalDirection::Either => None,
        })
        .map(object)
        .filter(|candidate| request.candidate_universe().contains(candidate))
        .collect();
    CompleteRelationshipSelection::try_new(
        request.clone(),
        reached,
        vec![Evidence::exact(source(), format!("{locator}:{anchor}"))],
    )
}

/// The stated space boundaries: `room-1` is bounded by `wall-1`.
struct Boundaries {
    snapshots: Vec<SourceSnapshot>,
    refuse: bool,
}

impl RelationshipSelectionService for Boundaries {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        assert_eq!(
            request.query().relationship().as_str(),
            "axioval:relationship.space-boundary"
        );
        if self.refuse {
            return Err(RelationshipSelectionError::Unavailable(
                "a boundary omits its element".into(),
            ));
        }
        walk(&[("room-1", "wall-1")], request, "stated")
    }
}

/// Geometry places `room-1` and `room-2` beside `wall-1`, `room-2` beside
/// `wall-2`.
struct Beside(Vec<SourceSnapshot>, Mutex<Vec<String>>);

impl DerivedRelationshipService for Beside {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn derive(
        &self,
        derivation: &Derivation,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        self.1
            .lock()
            .unwrap()
            .push(request.anchor().local_id.clone());
        walk(
            &[
                ("wall-1", "room-1"),
                ("wall-1", "room-2"),
                ("wall-2", "room-2"),
            ],
            request,
            &format!("{derivation}:geometry"),
        )
    }
}

fn across_session(refuse: bool) -> (EvidenceSession, Arc<Beside>) {
    let objects = ["wall-1", "wall-2", "room-1", "room-2"]
        .map(|local| Object::new(object(local), "THING"))
        .to_vec();
    let beside = Arc::new(Beside(vec![snapshot()], Mutex::new(Vec::new())));
    let session = EvidenceSession::try_new(Project::new(objects).unwrap(), [snapshot()])
        .unwrap()
        .with_service(RelationshipSelectionServiceHandle::new(Arc::new(
            Boundaries {
                snapshots: vec![snapshot()],
                refuse,
            },
        )))
        .unwrap()
        .with_derived_relationships(
            DerivedRelationshipServiceHandle::new(beside.clone()),
            &[snapshot()],
        )
        .unwrap();
    (session, beside)
}

fn across(
    anchor: &str,
    universe: &[&str],
    direction: TraversalDirection,
) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        object(anchor),
        universe.iter().map(|local| object(local)).collect(),
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("axioval:derived.adjacent-across").unwrap(),
            direction,
            follow_chain: false,
        },
    )
    .unwrap()
}

#[test]
fn stated_space_boundaries_win_over_the_derived_adjacency_across_an_element() {
    let (session, beside) = across_session(false);
    let relationships = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap();
    let names = |selection: &CompleteRelationshipSelection| -> Vec<String> {
        selection
            .candidates()
            .iter()
            .map(|candidate| candidate.local_id.clone())
            .collect()
    };

    // `wall-1` states a boundary, so geometry is not asked about it.
    let stated = relationships
        .select(&across(
            "wall-1",
            &["room-1", "room-2"],
            TraversalDirection::Forward,
        ))
        .unwrap();
    assert_eq!(names(&stated), ["room-1"]);
    assert!(beside.1.lock().unwrap().is_empty());
    assert!(
        stated
            .evidence()
            .iter()
            .any(|item| axioval_engine::across_stated(&item.locator, &object("wall-1")))
    );

    // `wall-2` states none: geometry answers, even when the universe holds
    // no space at all for the stated check.
    let derived = relationships
        .select(&across("wall-2", &["room-2"], TraversalDirection::Forward))
        .unwrap();
    assert_eq!(names(&derived), ["room-2"]);
    assert_eq!(beside.1.lock().unwrap().as_slice(), ["wall-2"]);

    // Backward from `room-2`: geometry places it beside both walls, but
    // `wall-1`'s stated boundaries do not name it.
    let walls = relationships
        .select(&across(
            "room-2",
            &["wall-1", "wall-2"],
            TraversalDirection::Backward,
        ))
        .unwrap();
    assert_eq!(names(&walls), ["wall-2"]);
    let walls = relationships
        .select(&across(
            "room-1",
            &["wall-1", "wall-2"],
            TraversalDirection::Backward,
        ))
        .unwrap();
    assert_eq!(names(&walls), ["wall-1"]);
}

#[test]
fn a_refused_stated_boundary_refuses_the_derived_adjacency_across_an_element() {
    let (session, beside) = across_session(true);
    let relationships = session
        .service::<RelationshipSelectionServiceHandle>()
        .unwrap();
    assert!(matches!(
        relationships.select(&across("wall-2", &["room-2"], TraversalDirection::Forward)),
        Err(RelationshipSelectionError::Unavailable(_))
    ));
    assert!(beside.1.lock().unwrap().is_empty());
}
