//! Exact source-neutral relationship-selection host-service contracts.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to select comparison candidates conclusively.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RelationshipSelectionError {
    /// The requested relationship or candidate universe is malformed.
    #[error("relationship selection request is invalid")]
    InvalidRequest,
    /// The candidate universe or response contains the same object more than once.
    #[error("relationship selection contains a duplicate candidate")]
    DuplicateCandidate,
    /// A response repeats one evidence locator.
    #[error("relationship selection contains duplicate evidence")]
    DuplicateEvidence,
    /// Returned data belongs to another request or escapes its candidate universe.
    #[error("relationship selection response does not match its request")]
    ResponseRequestMismatch,
    /// A conclusive selection lacks exact, reviewable completeness evidence.
    #[error("relationship selection evidence is not exact and reviewable")]
    InexactEvidence,
    /// The source cannot currently provide a conclusive selection.
    #[error("relationship selection unavailable: {0}")]
    Unavailable(String),
}

/// A host-registered semantic relationship or grouping identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticRelationship(String);

impl SemanticRelationship {
    /// Creates a non-empty source-neutral relationship identity.
    pub fn try_new(value: impl Into<String>) -> Result<Self, RelationshipSelectionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(RelationshipSelectionError::InvalidRequest);
        }
        Ok(Self(value))
    }

    /// Returns the declared semantic identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Direction used when traversing a directed semantic relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TraversalDirection {
    /// Follow edges from source to target.
    Forward,
    /// Follow edges from target to source.
    Backward,
    /// Follow edges in either direction.
    Either,
}

/// Source-neutral relationship operation used to select candidates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationshipQuery {
    /// Select members sharing at least one complete semantic group with the anchor.
    SharedGroup {
        /// Host-registered grouping identity such as a spatial or assembly context.
        relationship: SemanticRelationship,
    },
    /// Traverse a directed semantic relationship from the anchor.
    Related {
        /// Host-registered relationship identity.
        relationship: SemanticRelationship,
        /// Requested traversal direction.
        direction: TraversalDirection,
        /// Whether traversal continues beyond immediate neighbors.
        follow_chain: bool,
    },
}

/// Request for relationship-selected objects within a caller-bound universe.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationshipSelectionRequest {
    anchor: ObjectId,
    candidate_universe: Vec<ObjectId>,
    query: RelationshipQuery,
}

impl RelationshipSelectionRequest {
    /// Creates a request with a canonical, duplicate-free candidate universe.
    pub fn try_new(
        anchor: ObjectId,
        mut candidate_universe: Vec<ObjectId>,
        query: RelationshipQuery,
    ) -> Result<Self, RelationshipSelectionError> {
        candidate_universe.sort();
        if candidate_universe.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RelationshipSelectionError::DuplicateCandidate);
        }
        Ok(Self {
            anchor,
            candidate_universe,
            query,
        })
    }

    /// Anchor whose relationships determine the selection.
    #[must_use]
    pub fn anchor(&self) -> &ObjectId {
        &self.anchor
    }

    /// Complete caller-approved universe from which candidates may be returned.
    #[must_use]
    pub fn candidate_universe(&self) -> &[ObjectId] {
        &self.candidate_universe
    }

    /// Requested relationship operation.
    #[must_use]
    pub fn query(&self) -> &RelationshipQuery {
        &self.query
    }

    fn contains_candidate(&self, candidate: &ObjectId) -> bool {
        self.candidate_universe.binary_search(candidate).is_ok()
    }
}

/// Complete exact candidate selection bound to the request that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteRelationshipSelection {
    request: RelationshipSelectionRequest,
    candidates: Vec<ObjectId>,
    evidence: Vec<Evidence>,
}

impl CompleteRelationshipSelection {
    /// Creates a request-bound complete selection with canonical candidate ordering.
    pub fn try_new(
        request: RelationshipSelectionRequest,
        mut candidates: Vec<ObjectId>,
        mut evidence: Vec<Evidence>,
    ) -> Result<Self, RelationshipSelectionError> {
        candidates.sort();
        if candidates.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RelationshipSelectionError::DuplicateCandidate);
        }
        if candidates
            .iter()
            .any(|candidate| !request.contains_candidate(candidate))
        {
            return Err(RelationshipSelectionError::ResponseRequestMismatch);
        }
        if evidence.is_empty() || evidence.iter().any(|item| !reviewable(item)) {
            return Err(RelationshipSelectionError::InexactEvidence);
        }
        evidence.sort_by(|left, right| {
            (&left.source, &left.locator).cmp(&(&right.source, &right.locator))
        });
        if evidence.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RelationshipSelectionError::DuplicateEvidence);
        }
        Ok(Self {
            request,
            candidates,
            evidence,
        })
    }

    /// Complete request, including anchor, universe, and query.
    #[must_use]
    pub fn request(&self) -> &RelationshipSelectionRequest {
        &self.request
    }

    /// Canonically ordered selected candidates.
    #[must_use]
    pub fn candidates(&self) -> &[ObjectId] {
        &self.candidates
    }

    /// Exact reviewable evidence proving the selection is complete.
    #[must_use]
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
}

/// Trusted adapter seam for complete relationship-based candidate selection.
pub trait RelationshipSelectionService: Send + Sync {
    /// Selects candidates or reports why the result is not conclusive.
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError>;
}

/// Cloneable, type-erased relationship service registered by the host.
#[derive(Clone)]
pub struct RelationshipSelectionServiceHandle(Arc<dyn RelationshipSelectionService>);

impl RelationshipSelectionServiceHandle {
    /// Wraps a trusted relationship-selection service.
    #[must_use]
    pub fn new(service: Arc<dyn RelationshipSelectionService>) -> Self {
        Self(service)
    }

    /// Selects and validates complete request binding and evidence exactness.
    pub fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let selection = self.0.select(request)?;
        if selection.request() != request
            || selection
                .candidates()
                .iter()
                .any(|candidate| !request.contains_candidate(candidate))
        {
            return Err(RelationshipSelectionError::ResponseRequestMismatch);
        }
        if selection
            .candidates()
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(RelationshipSelectionError::DuplicateCandidate);
        }
        if selection.evidence().is_empty()
            || selection.evidence().iter().any(|item| !reviewable(item))
        {
            return Err(RelationshipSelectionError::InexactEvidence);
        }
        Ok(selection)
    }
}

fn reviewable(evidence: &Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}
