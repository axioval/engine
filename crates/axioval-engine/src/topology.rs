//! Deterministic source-neutral connectivity and route contracts.
//!
//! Geometry adapters may nominate edges only after proving the connection at
//! the declared evidence exactness. This module never infers connectivity from
//! source-format relationships or geometry backend handles.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// A fail-closed topology construction or query error.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TopologyError {
    /// The declared node universe contains the same source-qualified identity twice.
    #[error("duplicate topology node `{0}`")]
    DuplicateNode(Box<ObjectId>),
    /// A connection references an object outside the declared node universe.
    #[error("connection endpoint `{0}` is outside the declared topology universe")]
    UnknownEndpoint(Box<ObjectId>),
    /// A query references an object outside the declared node universe.
    #[error("topology query references unknown node `{0}`")]
    UnknownNode(Box<ObjectId>),
    /// A connection was asserted without exact adapter evidence.
    #[error("connectivity evidence is not exact")]
    InexactConnection,
    /// A connection joins an object to itself.
    #[error("self connections are invalid for `{0}`")]
    SelfConnection(Box<ObjectId>),
    /// A clear width or query threshold was non-finite or negative.
    #[error("invalid clear width `{0}`")]
    InvalidWidth(String),
    /// The same undirected connection was supplied more than once.
    #[error("duplicate connection between `{left}` and `{right}`")]
    DuplicateConnection {
        left: Box<ObjectId>,
        right: Box<ObjectId>,
    },
    /// Exact evidence did not carry a usable provenance locator.
    #[error("connectivity evidence locator must not be blank")]
    BlankEvidenceLocator,
    /// The adapter could not prove the declared topology universe complete.
    #[error("topology coverage evidence is not exact")]
    InexactTopologyCoverage,
}

/// Exact adapter evidence that all nodes and candidate transitions in scope were assessed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteTopologyEvidence(Evidence);

impl CompleteTopologyEvidence {
    /// Promotes adapter evidence only when it explicitly proves exact coverage.
    pub fn try_new(evidence: Evidence) -> Result<Self, TopologyError> {
        if !evidence.exact {
            return Err(TopologyError::InexactTopologyCoverage);
        }
        validate_evidence_locator(&evidence)?;
        Ok(Self(evidence))
    }

    /// Provenance for the complete topology projection.
    pub fn evidence(&self) -> &Evidence {
        &self.0
    }
}

/// A connection asserted exact by a trusted host adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedConnection {
    left: ObjectId,
    right: ObjectId,
    clear_width_metres: f64,
    evidence: Evidence,
}

impl VerifiedConnection {
    /// Creates a source-neutral exact connection.
    pub fn try_new(
        left: ObjectId,
        right: ObjectId,
        clear_width_metres: f64,
        evidence: Evidence,
    ) -> Result<Self, TopologyError> {
        if left == right {
            return Err(TopologyError::SelfConnection(Box::new(left)));
        }
        validate_width(clear_width_metres)?;
        if !evidence.exact {
            return Err(TopologyError::InexactConnection);
        }
        validate_evidence_locator(&evidence)?;
        let (left, right) = ordered_pair(left, right);
        Ok(Self {
            left,
            right,
            clear_width_metres,
            evidence,
        })
    }

    /// First endpoint in source-qualified identity order.
    pub fn left(&self) -> &ObjectId {
        &self.left
    }

    /// Second endpoint in source-qualified identity order.
    pub fn right(&self) -> &ObjectId {
        &self.right
    }

    /// Exact clear width in metres.
    pub fn clear_width_metres(&self) -> f64 {
        self.clear_width_metres
    }

    /// Adapter evidence proving this connection.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Result of a deterministic shortest-hop route query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteOutcome {
    /// Ordered nodes from origin through destination, including both endpoints.
    Route(Vec<ObjectId>),
    /// Both nodes are known and proven disconnected at the requested clear width.
    Unreachable,
}

/// Complete topology over a declared source-qualified object universe.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectivityGraph {
    nodes: BTreeSet<ObjectId>,
    adjacency: BTreeMap<ObjectId, BTreeMap<ObjectId, VerifiedConnection>>,
    coverage: CompleteTopologyEvidence,
}

impl ConnectivityGraph {
    /// Constructs a graph, rejecting ambiguous nodes and partial connection endpoints.
    pub fn try_new(
        nodes: impl IntoIterator<Item = ObjectId>,
        connections: impl IntoIterator<Item = VerifiedConnection>,
        coverage: CompleteTopologyEvidence,
    ) -> Result<Self, TopologyError> {
        let mut node_set = BTreeSet::new();
        for node in nodes {
            if !node_set.insert(node.clone()) {
                return Err(TopologyError::DuplicateNode(Box::new(node)));
            }
        }
        let mut adjacency = node_set
            .iter()
            .cloned()
            .map(|node| (node, BTreeMap::new()))
            .collect::<BTreeMap<_, _>>();
        for connection in connections {
            add_connection(&node_set, &mut adjacency, connection)?;
        }
        Ok(Self {
            nodes: node_set,
            adjacency,
            coverage,
        })
    }

    /// Evidence proving that the graph is a complete projection for its declared scope.
    pub fn coverage(&self) -> &CompleteTopologyEvidence {
        &self.coverage
    }

    /// Returns every node reachable through edges meeting the clear-width threshold.
    pub fn reachable_from(
        &self,
        origin: &ObjectId,
        minimum_clear_width_metres: f64,
    ) -> Result<Vec<ObjectId>, TopologyError> {
        self.require_node(origin)?;
        validate_width(minimum_clear_width_metres)?;
        let mut seen = BTreeSet::from([origin.clone()]);
        let mut queue = VecDeque::from([origin.clone()]);
        while let Some(current) = queue.pop_front() {
            for (neighbor, edge) in &self.adjacency[&current] {
                if edge.clear_width_metres >= minimum_clear_width_metres
                    && seen.insert(neighbor.clone())
                {
                    queue.push_back(neighbor.clone());
                }
            }
        }
        Ok(seen.into_iter().collect())
    }

    /// Finds the deterministic shortest-hop route meeting the clear-width threshold.
    pub fn route(
        &self,
        origin: &ObjectId,
        destination: &ObjectId,
        minimum_clear_width_metres: f64,
    ) -> Result<RouteOutcome, TopologyError> {
        self.require_node(origin)?;
        self.require_node(destination)?;
        validate_width(minimum_clear_width_metres)?;
        if origin == destination {
            return Ok(RouteOutcome::Route(vec![origin.clone()]));
        }
        let parents = self.search(origin, destination, minimum_clear_width_metres);
        if !parents.contains_key(destination) {
            return Ok(RouteOutcome::Unreachable);
        }
        Ok(RouteOutcome::Route(reconstruct_route(
            origin,
            destination,
            &parents,
        )))
    }

    fn require_node(&self, node: &ObjectId) -> Result<(), TopologyError> {
        if self.nodes.contains(node) {
            Ok(())
        } else {
            Err(TopologyError::UnknownNode(Box::new(node.clone())))
        }
    }

    fn search(
        &self,
        origin: &ObjectId,
        destination: &ObjectId,
        minimum_clear_width_metres: f64,
    ) -> BTreeMap<ObjectId, ObjectId> {
        let mut parents = BTreeMap::new();
        let mut seen = BTreeSet::from([origin.clone()]);
        let mut queue = VecDeque::from([origin.clone()]);
        while let Some(current) = queue.pop_front() {
            for (neighbor, edge) in &self.adjacency[&current] {
                if edge.clear_width_metres < minimum_clear_width_metres
                    || !seen.insert(neighbor.clone())
                {
                    continue;
                }
                parents.insert(neighbor.clone(), current.clone());
                if neighbor == destination {
                    return parents;
                }
                queue.push_back(neighbor.clone());
            }
        }
        parents
    }
}

fn add_connection(
    nodes: &BTreeSet<ObjectId>,
    adjacency: &mut BTreeMap<ObjectId, BTreeMap<ObjectId, VerifiedConnection>>,
    connection: VerifiedConnection,
) -> Result<(), TopologyError> {
    for endpoint in [&connection.left, &connection.right] {
        if !nodes.contains(endpoint) {
            return Err(TopologyError::UnknownEndpoint(Box::new(endpoint.clone())));
        }
    }
    if adjacency[&connection.left].contains_key(&connection.right) {
        return Err(TopologyError::DuplicateConnection {
            left: Box::new(connection.left),
            right: Box::new(connection.right),
        });
    }
    adjacency
        .get_mut(&connection.left)
        .expect("validated node")
        .insert(connection.right.clone(), connection.clone());
    adjacency
        .get_mut(&connection.right)
        .expect("validated node")
        .insert(connection.left.clone(), connection);
    Ok(())
}

fn reconstruct_route(
    origin: &ObjectId,
    destination: &ObjectId,
    parents: &BTreeMap<ObjectId, ObjectId>,
) -> Vec<ObjectId> {
    let mut route = vec![destination.clone()];
    let mut current = destination;
    while current != origin {
        current = &parents[current];
        route.push(current.clone());
    }
    route.reverse();
    route
}

fn ordered_pair(left: ObjectId, right: ObjectId) -> (ObjectId, ObjectId) {
    if left < right {
        (left, right)
    } else {
        (right, left)
    }
}

fn validate_width(width: f64) -> Result<(), TopologyError> {
    if width.is_finite() && width >= 0.0 {
        Ok(())
    } else {
        Err(TopologyError::InvalidWidth(width.to_string()))
    }
}

fn validate_evidence_locator(evidence: &Evidence) -> Result<(), TopologyError> {
    if evidence.locator.trim().is_empty() {
        Err(TopologyError::BlankEvidenceLocator)
    } else {
        Ok(())
    }
}
