//! Contract tests for exact request-bound relationship selection.

use std::sync::Arc;

use axioval_engine::{
    CompleteRelationshipSelection, RelationshipQuery, RelationshipSelectionError,
    RelationshipSelectionRequest, RelationshipSelectionService, RelationshipSelectionServiceHandle,
    SemanticRelationship, TraversalDirection,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

fn source() -> SourceId {
    SourceId::new("test", "model").unwrap()
}

fn object(local_id: &str) -> ObjectId {
    ObjectId::new(source(), local_id).unwrap()
}

fn related_request(anchor: &str, universe: &[&str]) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        object(anchor),
        universe.iter().map(|id| object(id)).collect(),
        RelationshipQuery::Related {
            relationship: SemanticRelationship::try_new("contains").unwrap(),
            direction: TraversalDirection::Forward,
            follow_chain: false,
        },
    )
    .unwrap()
}

fn shared_group_request(anchor: &str, universe: &[&str]) -> RelationshipSelectionRequest {
    RelationshipSelectionRequest::try_new(
        object(anchor),
        universe.iter().map(|id| object(id)).collect(),
        RelationshipQuery::SharedGroup {
            relationship: SemanticRelationship::try_new("spatial-context").unwrap(),
        },
    )
    .unwrap()
}

#[derive(Clone)]
struct FixedSelection(Result<CompleteRelationshipSelection, RelationshipSelectionError>);

impl RelationshipSelectionService for FixedSelection {
    fn select(
        &self,
        _request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        self.0.clone()
    }
}

fn exact_selection(
    request: RelationshipSelectionRequest,
    candidates: Vec<ObjectId>,
) -> CompleteRelationshipSelection {
    CompleteRelationshipSelection::try_new(
        request,
        candidates,
        vec![Evidence::exact(source(), "relationship-index:1")],
    )
    .unwrap()
}

#[test]
fn blank_relationship_identity_is_rejected() {
    assert_eq!(
        SemanticRelationship::try_new("  ").unwrap_err(),
        RelationshipSelectionError::InvalidRequest
    );
}

#[test]
fn cross_anchor_selection_replay_is_rejected() {
    let requested = related_request("checked-b", &["candidate"]);
    let replayed_request = related_request("checked-a", &["candidate"]);
    let response = exact_selection(replayed_request, vec![object("candidate")]);
    let handle = RelationshipSelectionServiceHandle::new(Arc::new(FixedSelection(Ok(response))));

    assert_eq!(
        handle.select(&requested),
        Err(RelationshipSelectionError::ResponseRequestMismatch)
    );
}

#[test]
fn candidate_outside_bound_universe_is_rejected() {
    let request = related_request("checked", &["allowed"]);
    assert_eq!(
        CompleteRelationshipSelection::try_new(
            request,
            vec![object("other")],
            vec![Evidence::exact(source(), "relationship-index:2")],
        ),
        Err(RelationshipSelectionError::ResponseRequestMismatch)
    );
}

#[test]
fn candidates_and_universe_are_canonically_ordered() {
    let request = related_request("checked", &["z", "a"]);
    assert_eq!(request.candidate_universe(), &[object("a"), object("z")]);
    let selection = exact_selection(request, vec![object("z"), object("a")]);
    assert_eq!(selection.candidates(), &[object("a"), object("z")]);
}

#[test]
fn duplicate_candidate_universe_is_rejected() {
    assert_eq!(
        RelationshipSelectionRequest::try_new(
            object("checked"),
            vec![object("candidate"), object("candidate")],
            RelationshipQuery::SharedGroup {
                relationship: SemanticRelationship::try_new("group").unwrap(),
            },
        ),
        Err(RelationshipSelectionError::DuplicateCandidate)
    );
}

#[test]
fn query_mismatch_is_rejected_by_the_trusted_handle() {
    let requested = shared_group_request("checked", &["candidate"]);
    let replayed = related_request("checked", &["candidate"]);
    let response = exact_selection(replayed, vec![object("candidate")]);
    let handle = RelationshipSelectionServiceHandle::new(Arc::new(FixedSelection(Ok(response))));

    assert_eq!(
        handle.select(&requested),
        Err(RelationshipSelectionError::ResponseRequestMismatch)
    );
}

#[test]
fn missing_or_unreviewable_completeness_evidence_is_rejected() {
    let request = related_request("checked", &["candidate"]);
    assert_eq!(
        CompleteRelationshipSelection::try_new(request.clone(), Vec::new(), Vec::new()),
        Err(RelationshipSelectionError::InexactEvidence)
    );
    assert_eq!(
        CompleteRelationshipSelection::try_new(
            request,
            Vec::new(),
            vec![Evidence::exact(source(), "")],
        ),
        Err(RelationshipSelectionError::InexactEvidence)
    );
}

#[test]
fn response_evidence_is_canonical_and_unique() {
    let request = related_request("checked", &["candidate"]);
    let selection = CompleteRelationshipSelection::try_new(
        request.clone(),
        vec![],
        vec![
            Evidence::exact(source(), "z"),
            Evidence::exact(source(), "a"),
        ],
    )
    .unwrap();
    assert_eq!(selection.evidence()[0].locator, "a");
    assert_eq!(selection.evidence()[1].locator, "z");
    assert_eq!(
        CompleteRelationshipSelection::try_new(
            request,
            vec![],
            vec![
                Evidence::exact(source(), "same"),
                Evidence::exact(source(), "same"),
            ],
        )
        .unwrap_err(),
        RelationshipSelectionError::DuplicateEvidence
    );
}
