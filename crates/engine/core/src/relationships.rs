//! Exact source-neutral relationship-selection host-service contracts.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::session::{SnapshotBoundService, SourceSnapshot};

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

/// Prefix of the source-neutral [`RelationshipKind`] identities.
pub const RELATIONSHIP_KIND_PREFIX: &str = "axioval:relationship.";

/// A source-neutral kind of stated relationship.
///
/// Each kind is a directed relationship from a relating end to its related
/// ends, as a source states it. A source answers a kind under its identity
/// ([`RelationshipKind::relationship`], `axioval:relationship.<name>`) through
/// the same [`RelationshipSelectionService`] as its own relationships,
/// mapping it onto its native relationships; one that does not know a kind
/// refuses it, never answers it empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RelationshipKind {
    /// A spatial structure element to the elements it contains.
    Containment,
    /// A whole to its parts.
    Aggregation,
    /// An element to the openings voiding it.
    Voids,
    /// An opening to the elements filling it.
    Fills,
    /// A space to the elements bounding it.
    SpaceBoundary,
    /// A type to the occurrences it is assigned to.
    TypeAssignment,
    /// A group, system or zone to its members.
    GroupMembership,
    /// An element to the elements it is connected to.
    Connection,
}

impl RelationshipKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 8] = [
        Self::Containment,
        Self::Aggregation,
        Self::Voids,
        Self::Fills,
        Self::SpaceBoundary,
        Self::TypeAssignment,
        Self::GroupMembership,
        Self::Connection,
    ];

    /// Stable lowercase name, used in identities and reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Containment => "containment",
            Self::Aggregation => "aggregation",
            Self::Voids => "voids",
            Self::Fills => "fills",
            Self::SpaceBoundary => "space-boundary",
            Self::TypeAssignment => "type",
            Self::GroupMembership => "group",
            Self::Connection => "connection",
        }
    }

    /// The identity a source answers the kind under:
    /// `axioval:relationship.<name>`.
    #[must_use]
    pub fn relationship(self) -> SemanticRelationship {
        SemanticRelationship(format!("{RELATIONSHIP_KIND_PREFIX}{}", self.name()))
    }

    /// The kind an identity names, if it names one.
    #[must_use]
    pub fn of(relationship: &str) -> Option<Self> {
        let name = relationship.strip_prefix(RELATIONSHIP_KIND_PREFIX)?;
        Self::ALL.into_iter().find(|kind| kind.name() == name)
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

impl RelationshipQuery {
    /// The relationship or grouping identity the query names.
    #[must_use]
    pub fn relationship(&self) -> &SemanticRelationship {
        match self {
            Self::SharedGroup { relationship } | Self::Related { relationship, .. } => relationship,
        }
    }
}

/// What a relationship service does with an instance whose required end is absent.
///
/// A source can carry relationship instances that omit an end the schema
/// requires, for example a virtual space boundary with no bounding element.
/// Such an instance contributes no edge through the absent end, so treating it
/// as a complete answer would silently read "not related" into a gap. The
/// default refuses; a rule opts into skipping explicitly, and the service then
/// names every skipped instance it passed over in the selection's evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AbsentEndPolicy {
    /// Refuse the whole answer while any instance of the type lacks a required end.
    #[default]
    Refuse,
    /// Answer from the edges that exist and cite each skipped instance.
    Skip,
}

/// Request for relationship-selected objects within a caller-bound universe.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationshipSelectionRequest {
    anchor: ObjectId,
    candidate_universe: Vec<ObjectId>,
    query: RelationshipQuery,
    absent_ends: AbsentEndPolicy,
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
            absent_ends: AbsentEndPolicy::default(),
        })
    }

    /// Sets how instances with an absent required end are treated.
    #[must_use]
    pub fn with_absent_ends(mut self, policy: AbsentEndPolicy) -> Self {
        self.absent_ends = policy;
        self
    }

    /// How instances with an absent required end are treated.
    #[must_use]
    pub fn absent_ends(&self) -> AbsentEndPolicy {
        self.absent_ends
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

/// Request for every edge of one relationship within a caller-bound universe.
///
/// Where a [`RelationshipSelectionRequest`] asks what one anchor reaches, an
/// edges request lists every stated edge whose two ends both lie in the
/// universe, so a caller relating every object of a model asks once per
/// relationship rather than once per object.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationshipEdgesRequest {
    universe: Vec<ObjectId>,
    relationship: SemanticRelationship,
    absent_ends: AbsentEndPolicy,
}

impl RelationshipEdgesRequest {
    /// Creates a request with a canonical, duplicate-free universe.
    ///
    /// # Errors
    ///
    /// Returns [`RelationshipSelectionError::DuplicateCandidate`] when the
    /// universe holds an object twice.
    pub fn try_new(
        mut universe: Vec<ObjectId>,
        relationship: SemanticRelationship,
    ) -> Result<Self, RelationshipSelectionError> {
        universe.sort();
        if universe.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RelationshipSelectionError::DuplicateCandidate);
        }
        Ok(Self {
            universe,
            relationship,
            absent_ends: AbsentEndPolicy::default(),
        })
    }

    /// Sets how instances with an absent required end are treated.
    #[must_use]
    pub fn with_absent_ends(mut self, policy: AbsentEndPolicy) -> Self {
        self.absent_ends = policy;
        self
    }

    /// How instances with an absent required end are treated.
    #[must_use]
    pub fn absent_ends(&self) -> AbsentEndPolicy {
        self.absent_ends
    }

    /// The canonically ordered objects both ends of every edge lie in.
    #[must_use]
    pub fn universe(&self) -> &[ObjectId] {
        &self.universe
    }

    /// The relationship whose edges are listed.
    #[must_use]
    pub fn relationship(&self) -> &SemanticRelationship {
        &self.relationship
    }

    fn contains(&self, object: &ObjectId) -> bool {
        self.universe.binary_search(object).is_ok()
    }
}

/// One stated edge, from its relating end to one related end.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelationshipEdge {
    /// The relating end.
    pub relating: ObjectId,
    /// The related end.
    pub related: ObjectId,
}

/// Every edge of one relationship within a universe, bound to its request.
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteRelationshipEdges {
    request: RelationshipEdgesRequest,
    edges: Vec<RelationshipEdge>,
    evidence: Vec<Evidence>,
}

impl CompleteRelationshipEdges {
    /// Creates a request-bound complete listing with canonical edge order.
    ///
    /// # Errors
    ///
    /// Returns an error when an edge repeats or leaves the universe, or the
    /// evidence is empty, inexact or repeated.
    pub fn try_new(
        request: RelationshipEdgesRequest,
        mut edges: Vec<RelationshipEdge>,
        mut evidence: Vec<Evidence>,
    ) -> Result<Self, RelationshipSelectionError> {
        edges.sort();
        if edges.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RelationshipSelectionError::DuplicateCandidate);
        }
        if edges.iter().any(|edge| !request.holds(edge)) {
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
            edges,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &RelationshipEdgesRequest {
        &self.request
    }

    /// Canonically ordered edges.
    #[must_use]
    pub fn edges(&self) -> &[RelationshipEdge] {
        &self.edges
    }

    /// Exact reviewable evidence proving the listing is complete.
    #[must_use]
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
}

impl RelationshipEdgesRequest {
    /// Whether both ends of `edge` lie in the universe.
    fn holds(&self, edge: &RelationshipEdge) -> bool {
        self.contains(&edge.relating) && self.contains(&edge.related)
    }
}

/// Trusted adapter seam for complete relationship-based candidate selection.
pub trait RelationshipSelectionService: Send + Sync {
    /// Exact source snapshots used to construct this service.
    ///
    /// The default is intentionally unbound for services used only through a
    /// raw [`crate::ServiceRegistry`]; an [`crate::EvidenceSession`] rejects it.
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &[]
    }
    /// Selects candidates or reports why the result is not conclusive.
    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError>;

    /// Lists every edge of a relationship within the request's universe.
    ///
    /// The default refuses: a service that cannot list edges is never read
    /// as one stating none.
    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        Err(RelationshipSelectionError::Unavailable(format!(
            "the relationship service cannot list the edges of `{}`",
            request.relationship().as_str()
        )))
    }
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
        validate_selection(request, self.0.select(request)?)
    }

    /// Lists edges and validates their request binding, order and evidence.
    ///
    /// # Errors
    ///
    /// Returns the service's refusal, or why its answer does not answer the
    /// request exactly.
    pub fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        let listing = self.0.edges(request)?;
        if listing.request() != request || listing.edges().iter().any(|e| !request.holds(e)) {
            return Err(RelationshipSelectionError::ResponseRequestMismatch);
        }
        if listing.edges().windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RelationshipSelectionError::DuplicateCandidate);
        }
        if listing.evidence().is_empty() || listing.evidence().iter().any(|item| !reviewable(item))
        {
            return Err(RelationshipSelectionError::InexactEvidence);
        }
        Ok(listing)
    }
}

/// Checks that `selection` answers exactly `request`, stays inside its
/// universe in canonical order, and carries exact reviewable evidence.
pub(crate) fn validate_selection(
    request: &RelationshipSelectionRequest,
    selection: CompleteRelationshipSelection,
) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
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
    if selection.evidence().is_empty() || selection.evidence().iter().any(|item| !reviewable(item))
    {
        return Err(RelationshipSelectionError::InexactEvidence);
    }
    Ok(selection)
}

impl SnapshotBoundService for RelationshipSelectionServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

fn reviewable(evidence: &Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}
