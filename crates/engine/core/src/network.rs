//! Connected building services networks: the graph of distribution
//! elements a source's ports and connections state, and the derived
//! relationship `axioval:derived.flow` that walks it.
//!
//! A source states its network through a [`DistributionNetworkService`]
//! as [`DistributionNetwork`]s, one per source, in source-neutral terms:
//! ports, the one element each belongs to, the direction flow takes
//! through each, and which ports are connected. The engine joins them into
//! a graph of elements: two elements whose ports are connected are
//! neighbours (through the element realizing the connection, where the
//! source names one), and flow runs from the element whose port is a
//! `source` to the one whose port is a `sink`.
//!
//! `axioval:derived.flow` runs in flow direction: `forward` from an element
//! reaches the elements its flow passes to (downstream), `backward` those
//! whose flow reaches it (upstream), and `either` every element connected
//! to it whatever the direction; `+` follows the chain. A direction-bound
//! read never guesses: a connection whose ports state no single direction
//! (a port without one, one passing flow either way, or two ports facing
//! the same way), and flow running in a cycle, refuse every directed read
//! that meets them, naming them. A connection to a port that belongs to no
//! element (or to several) refuses every read through its element, since
//! where it leads is unknown. Reachability `either` way needs no direction
//! and is answered across undecided connections and cycles.
//!
//! A port connected to nothing is an open end: the source states that
//! nothing is connected there, so nothing is reached through it. Every
//! answer reaching an element with an open end cites the port, so a
//! fixture reaching no collector is found through its open outlet.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use axioval_ir::network::NetworkError;
use axioval_ir::{DistributionNetwork, Evidence, FlowDirection, ObjectId, SourceId};
use thiserror::Error;

use crate::relationships::{
    CompleteRelationshipEdges, CompleteRelationshipSelection, RelationshipEdgesRequest,
    RelationshipQuery, RelationshipSelectionError, RelationshipSelectionRequest,
    RelationshipSelectionService, RelationshipSelectionServiceHandle, TraversalDirection,
};
use crate::services::ServiceRegistry;
use crate::session::{SnapshotBoundService, SourceSnapshot};

/// The derived relationship running in flow direction through a network.
pub const FLOW_RELATIONSHIP: &str = "axioval:derived.flow";

/// Why a source's network cannot be listed.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NetworkServiceError {
    /// The service holds no such source.
    #[error("no distribution network service covers source `{0}`")]
    UncoveredSource(SourceId),
    /// The source states its network in a way that cannot be read exactly.
    #[error("the distribution network cannot be read: {0}")]
    Unavailable(String),
    /// The answer is no complete, exact network of the source asked for.
    #[error("the distribution network answer is invalid: {0}")]
    Invalid(String),
}

impl From<NetworkError> for NetworkServiceError {
    fn from(error: NetworkError) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// Trusted adapter seam listing the distribution network of one source.
pub trait DistributionNetworkService: Send + Sync {
    /// Exact source snapshots used to construct this service.
    // gate: reads
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &[]
    }

    /// Every port and port connection `source` states.
    ///
    /// # Errors
    ///
    /// A source the service does not cover, or a network it cannot read
    /// exactly and completely, is refused, never answered empty.
    // gate: reads
    fn network(&self, source: &SourceId) -> Result<DistributionNetwork, NetworkServiceError>;
}

/// Cloneable, type-erased network service registered by the host.
#[derive(Clone)]
pub struct DistributionNetworkServiceHandle(Arc<dyn DistributionNetworkService>);

impl DistributionNetworkServiceHandle {
    /// Wraps a trusted network service.
    #[must_use]
    pub fn new(service: Arc<dyn DistributionNetworkService>) -> Self {
        Self(service)
    }

    /// The network of `source`, checked to be that source's.
    ///
    /// # Errors
    ///
    /// The service's refusal, or [`NetworkServiceError::Invalid`] when it
    /// answers for another source.
    pub fn network(&self, source: &SourceId) -> Result<DistributionNetwork, NetworkServiceError> {
        let network = self.0.network(source)?;
        if network.source() != source {
            return Err(NetworkServiceError::Invalid(format!(
                "asked for `{source}`, answered for `{}`",
                network.source()
            )));
        }
        Ok(network)
    }
}

impl SnapshotBoundService for DistributionNetworkServiceHandle {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.0.source_snapshots()
    }
}

/// How flow runs along one edge, seen from the element holding it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Flow {
    /// From this element to the other.
    Out,
    /// From the other element to this one.
    In,
    /// The ports state no single direction, for this reason.
    Undecided(String),
}

#[derive(Clone, Debug)]
struct Edge {
    other: ObjectId,
    flow: Flow,
    evidence: Vec<Evidence>,
}

/// The element graph of one source's network.
#[derive(Debug, Default)]
pub struct FlowGraph {
    edges: BTreeMap<ObjectId, Vec<Edge>>,
    /// Elements a connection leads from to no known element, and why.
    broken: BTreeMap<ObjectId, String>,
    /// Elements on a cycle of flow, and the cycle.
    cyclic: BTreeMap<ObjectId, String>,
    /// Ports of each element connected to nothing, with their evidence.
    open: BTreeMap<ObjectId, Vec<(ObjectId, Vec<Evidence>)>>,
    /// Every port of each element, with the flow direction it states.
    ports: BTreeMap<ObjectId, Vec<(ObjectId, Option<FlowDirection>)>>,
    evidence: Vec<Evidence>,
}

/// The objects a read reached, and the evidence it rests on.
pub type Reached = (BTreeSet<ObjectId>, Vec<Evidence>);

impl FlowGraph {
    /// Joins a source's ports and connections into its element graph.
    #[must_use]
    pub fn build(network: &DistributionNetwork) -> Self {
        let mut graph = Self {
            evidence: network.evidence().to_vec(),
            ..Self::default()
        };
        let ports: BTreeMap<&ObjectId, _> = network
            .ports()
            .iter()
            .map(|port| (&port.port, port))
            .collect();
        let owner = |port: &ObjectId| -> Result<ObjectId, String> {
            match ports.get(port).map(|port| port.elements.as_slice()) {
                Some([element]) => Ok(element.clone()),
                Some([]) | None => Err(format!("port {port} belongs to no element")),
                Some(several) => Err(format!(
                    "port {port} belongs to {} elements ({})",
                    several.len(),
                    joined(several)
                )),
            }
        };
        let mut connected = BTreeSet::new();
        for connection in network.connections() {
            connected.insert(&connection.relating);
            connected.insert(&connection.related);
            let ends = (owner(&connection.relating), owner(&connection.related));
            let (from, to) = match ends {
                (Ok(from), Ok(to)) if from != to => (from, to),
                (Ok(element), Ok(_)) => {
                    graph.broken.entry(element.clone()).or_insert_with(|| {
                        format!(
                            "ports {} and {} connect {element} to itself",
                            connection.relating, connection.related
                        )
                    });
                    continue;
                }
                (Ok(element), Err(why)) | (Err(why), Ok(element)) => {
                    graph.broken.entry(element.clone()).or_insert_with(|| {
                        format!(
                            "{element} is connected to a port where the network is unknown: {why}"
                        )
                    });
                    continue;
                }
                (Err(_), Err(_)) => continue,
            };
            let flow = |port: &ObjectId| ports.get(port).and_then(|port| port.flow);
            let decided = direction(
                (&connection.relating, flow(&connection.relating)),
                (&connection.related, flow(&connection.related)),
            );
            let mut chain = vec![from];
            chain.extend(connection.realizing.clone());
            chain.push(to);
            let mut evidence = vec![connection.evidence.clone()];
            for port in [&connection.relating, &connection.related] {
                if let Some(port) = ports.get(port) {
                    evidence.extend(port.evidence.iter().cloned());
                }
            }
            for pair in chain.windows(2) {
                let (here, there) = (&pair[0], &pair[1]);
                let (out, back) = match &decided {
                    Ok(true) => (Flow::Out, Flow::In),
                    Ok(false) => (Flow::In, Flow::Out),
                    Err(why) => (Flow::Undecided(why.clone()), Flow::Undecided(why.clone())),
                };
                graph.edges.entry(here.clone()).or_default().push(Edge {
                    other: there.clone(),
                    flow: out,
                    evidence: evidence.clone(),
                });
                graph.edges.entry(there.clone()).or_default().push(Edge {
                    other: here.clone(),
                    flow: back,
                    evidence: evidence.clone(),
                });
            }
        }
        for port in network.ports() {
            if let [element] = port.elements.as_slice() {
                graph
                    .ports
                    .entry(element.clone())
                    .or_default()
                    .push((port.port.clone(), port.flow));
            }
            if connected.contains(&port.port) {
                continue;
            }
            if let [element] = port.elements.as_slice() {
                graph
                    .open
                    .entry(element.clone())
                    .or_default()
                    .push((port.port.clone(), port.evidence.clone()));
            }
        }
        graph.find_cycles();
        graph
    }

    /// Marks every element on a cycle of decided flow (Tarjan's strongly
    /// connected components, iteratively).
    fn find_cycles(&mut self) {
        let nodes: Vec<&ObjectId> = self.edges.keys().collect();
        let position: BTreeMap<&ObjectId, usize> =
            nodes.iter().enumerate().map(|(at, id)| (*id, at)).collect();
        let successors: Vec<Vec<usize>> = nodes
            .iter()
            .map(|node| {
                self.edges[*node]
                    .iter()
                    .filter(|edge| edge.flow == Flow::Out)
                    .map(|edge| position[&edge.other])
                    .collect()
            })
            .collect();
        let count = nodes.len();
        let mut index = vec![usize::MAX; count];
        let mut low = vec![0; count];
        let mut on_stack = vec![false; count];
        let mut stack = Vec::new();
        let mut next = 0;
        let mut components: Vec<Vec<usize>> = Vec::new();
        for root in 0..count {
            if index[root] != usize::MAX {
                continue;
            }
            let mut work = vec![(root, 0usize)];
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root);
            on_stack[root] = true;
            while let Some((node, child)) = work.last_mut() {
                let node = *node;
                if let Some(&successor) = successors[node].get(*child) {
                    *child += 1;
                    if index[successor] == usize::MAX {
                        index[successor] = next;
                        low[successor] = next;
                        next += 1;
                        stack.push(successor);
                        on_stack[successor] = true;
                        work.push((successor, 0));
                    } else if on_stack[successor] {
                        low[node] = low[node].min(index[successor]);
                    }
                    continue;
                }
                work.pop();
                if let Some((parent, _)) = work.last() {
                    low[*parent] = low[*parent].min(low[node]);
                }
                if low[node] == index[node] {
                    let mut component = Vec::new();
                    while let Some(member) = stack.pop() {
                        on_stack[member] = false;
                        component.push(member);
                        if member == node {
                            break;
                        }
                    }
                    if component.len() > 1 {
                        components.push(component);
                    }
                }
            }
        }
        for component in components {
            let mut members: Vec<ObjectId> =
                component.iter().map(|at| nodes[*at].clone()).collect();
            members.sort();
            let why = format!("flow runs in a cycle through {}", joined(&members));
            for member in members {
                self.cyclic.insert(member, why.clone());
            }
        }
    }

    /// Every port of `element` that belongs to it alone, sorted, with the
    /// flow direction it states.
    #[must_use]
    pub fn ports(&self, element: &ObjectId) -> &[(ObjectId, Option<FlowDirection>)] {
        self.ports.get(element).map_or(&[], Vec::as_slice)
    }

    /// Whether the direction of flow at `element` is decided: none of its
    /// connections is undecided or leads to an unknown end, and it lies on
    /// no cycle of flow.
    ///
    /// # Errors
    ///
    /// Why it is undecided, as a directed read from it is.
    pub fn settled(&self, element: &ObjectId) -> Result<(), String> {
        self.reach(element, TraversalDirection::Forward, false)
            .map(|_| ())
    }

    /// The network's completeness proof.
    #[must_use]
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    /// The elements `anchor` reaches in `direction` (with `chain`, one or
    /// more steps), and the evidence: the network's completeness proof, the
    /// connections passed and the open ends met.
    ///
    /// # Errors
    ///
    /// Why the answer is undecided: an element reached through a
    /// connection to an unknown end, and, for a directed read, a
    /// connection of undecided direction or a cycle of flow it meets.
    pub fn reach(
        &self,
        anchor: &ObjectId,
        direction: TraversalDirection,
        chain: bool,
    ) -> Result<Reached, String> {
        let directed = direction != TraversalDirection::Either;
        let mut evidence = self.evidence.clone();
        let mut reached = BTreeSet::new();
        let mut seen = BTreeSet::from([anchor.clone()]);
        let mut queue = VecDeque::from([anchor.clone()]);
        while let Some(current) = queue.pop_front() {
            if let Some(why) = self.broken.get(&current) {
                return Err(why.clone());
            }
            if directed && let Some(why) = self.cyclic.get(&current) {
                return Err(format!(
                    "the flow direction at {current} is undecided: {why}"
                ));
            }
            for (port, cited) in self.open.get(&current).into_iter().flatten() {
                evidence.push(Evidence::exact(
                    port.source.clone(),
                    format!("{FLOW_RELATIONSHIP}:open-end:{port}"),
                ));
                evidence.extend(cited.iter().cloned());
            }
            for edge in self.edges.get(&current).into_iter().flatten() {
                let follow = match (&edge.flow, direction) {
                    (_, TraversalDirection::Either)
                    | (Flow::Out, TraversalDirection::Forward)
                    | (Flow::In, TraversalDirection::Backward) => true,
                    (Flow::Undecided(why), _) => {
                        return Err(format!(
                            "the flow direction between {current} and {} is undecided: {why}",
                            edge.other
                        ));
                    }
                    _ => false,
                };
                if !follow {
                    continue;
                }
                evidence.extend(edge.evidence.iter().cloned());
                reached.insert(edge.other.clone());
                if chain && seen.insert(edge.other.clone()) {
                    queue.push_back(edge.other.clone());
                }
            }
            if !chain {
                break;
            }
        }
        reached.remove(anchor);
        let mut unique = BTreeSet::new();
        evidence.retain(|item| unique.insert((item.source.clone(), item.locator.clone())));
        Ok((reached, evidence))
    }
}

/// Whether flow runs from the relating port's element to the related
/// one's (`true`) or back, or why the ports state no single direction.
fn direction(
    (relating, first): (&ObjectId, Option<FlowDirection>),
    (related, second): (&ObjectId, Option<FlowDirection>),
) -> Result<bool, String> {
    match (first, second) {
        (Some(FlowDirection::Source), Some(FlowDirection::Sink)) => Ok(true),
        (Some(FlowDirection::Sink), Some(FlowDirection::Source)) => Ok(false),
        _ => {
            let shown = |port: &ObjectId, flow: Option<FlowDirection>| match flow {
                Some(flow) => format!("{port} is {}", flow.name()),
                None => format!("{port} states no flow direction"),
            };
            Err(format!(
                "of the connected ports, {} and {}",
                shown(relating, first),
                shown(related, second)
            ))
        }
    }
}

fn joined(objects: &[ObjectId]) -> String {
    objects
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Installs `axioval:derived.flow` over the run's relationship service,
/// answered from the host's network service; without either service
/// nothing is installed, so a missing relationship service stays missing.
pub(crate) fn install(services: &mut ServiceRegistry) {
    let inner = services
        .get::<RelationshipSelectionServiceHandle>()
        .cloned();
    let graphs = services
        .get::<DistributionNetworkServiceHandle>()
        .cloned()
        .map(|network| Arc::new(FlowGraphs::new(network)));
    if let Some(graphs) = &graphs {
        services.replace(graphs.clone());
    }
    if inner.is_none() && graphs.is_none() {
        return;
    }
    services.replace(RelationshipSelectionServiceHandle::new(Arc::new(
        FlowRelationships { inner, graphs },
    )));
}

type Graphs = BTreeMap<SourceId, Result<Arc<FlowGraph>, String>>;

/// The flow graph of each source, built from the network service once per
/// run on first use; the runtime installs it as `Arc<FlowGraphs>` beside
/// the host's services whenever a network service is registered.
pub struct FlowGraphs {
    network: DistributionNetworkServiceHandle,
    graphs: Mutex<Graphs>,
}

impl FlowGraphs {
    /// The graphs of `network`'s sources, none built yet.
    #[must_use]
    pub fn new(network: DistributionNetworkServiceHandle) -> Self {
        Self {
            network,
            graphs: Mutex::default(),
        }
    }

    /// The flow graph of `source`.
    ///
    /// # Errors
    ///
    /// Why the source's network cannot be read.
    pub fn graph(&self, source: &SourceId) -> Result<Arc<FlowGraph>, String> {
        let mut graphs = self
            .graphs
            .lock()
            .map_err(|_| "the flow graph cache is poisoned".to_owned())?;
        graphs
            .entry(source.clone())
            .or_insert_with(|| {
                self.network
                    .network(source)
                    .map(|network| Arc::new(FlowGraph::build(&network)))
                    .map_err(|error| error.to_string())
            })
            .clone()
    }
}

/// Answers [`FLOW_RELATIONSHIP`] from the network service, every other
/// identity from the host's relationship service.
struct FlowRelationships {
    inner: Option<RelationshipSelectionServiceHandle>,
    graphs: Option<Arc<FlowGraphs>>,
}

impl FlowRelationships {
    fn graph(&self, source: &SourceId) -> Result<Arc<FlowGraph>, String> {
        self.graphs
            .as_ref()
            .ok_or_else(|| {
                "no distribution network service is registered, so no flow can be traversed"
                    .to_owned()
            })?
            .graph(source)
    }

    fn answer(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let RelationshipQuery::Related {
            direction,
            follow_chain,
            ..
        } = request.query()
        else {
            return Err(RelationshipSelectionError::InvalidRequest);
        };
        let anchor = request.anchor();
        let graph = self
            .graph(&anchor.source)
            .map_err(RelationshipSelectionError::Unavailable)?;
        let (reached, evidence) =
            graph
                .reach(anchor, *direction, *follow_chain)
                .map_err(|why| {
                    RelationshipSelectionError::Unavailable(format!("from {anchor}: {why}"))
                })?;
        let candidates = request
            .candidate_universe()
            .iter()
            .filter(|candidate| reached.contains(*candidate))
            .cloned()
            .collect();
        CompleteRelationshipSelection::try_new(request.clone(), candidates, evidence)
    }
}

impl RelationshipSelectionService for FlowRelationships {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        self.inner
            .as_ref()
            .map_or(&[], |inner| inner.source_snapshots())
    }

    fn select(
        &self,
        request: &RelationshipSelectionRequest,
    ) -> Result<CompleteRelationshipSelection, RelationshipSelectionError> {
        let identity = request.query().relationship().as_str();
        if identity == FLOW_RELATIONSHIP {
            return self.answer(request);
        }
        if identity
            .strip_prefix(FLOW_RELATIONSHIP)
            .is_some_and(|rest| rest.starts_with(';'))
        {
            // The flow takes no parameters.
            return Err(RelationshipSelectionError::InvalidRequest);
        }
        match &self.inner {
            Some(inner) => inner.select(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no relationship service is registered".into(),
            )),
        }
    }

    /// Every other identity is the host's to close; the flow is never taken as closed.
    fn closes_chains(&self, relationship: &crate::SemanticRelationship) -> bool {
        !relationship.as_str().starts_with(FLOW_RELATIONSHIP)
            && self
                .inner
                .as_ref()
                .is_some_and(|inner| inner.closes_chains(relationship))
    }

    fn edges(
        &self,
        request: &RelationshipEdgesRequest,
    ) -> Result<CompleteRelationshipEdges, RelationshipSelectionError> {
        match &self.inner {
            Some(inner) => inner.edges(request),
            None => Err(RelationshipSelectionError::Unavailable(
                "no relationship service is registered".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::{NetworkPort, PortConnection};

    fn source() -> SourceId {
        SourceId::new("t", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn proof(locator: &str) -> Evidence {
        Evidence::exact(source(), locator)
    }

    /// Builds a network: `ports` as (port, element, flow), `connections`
    /// as (relating port, related port).
    fn network(
        ports: &[(&str, &str, Option<FlowDirection>)],
        connections: &[(&str, &str)],
    ) -> FlowGraph {
        let ports = ports
            .iter()
            .map(|(port, element, flow)| NetworkPort {
                port: id(port),
                elements: if element.is_empty() {
                    Vec::new()
                } else {
                    vec![id(element)]
                },
                flow: *flow,
                evidence: vec![proof(&format!("port:{port}"))],
            })
            .collect();
        let connections = connections
            .iter()
            .map(|(relating, related)| PortConnection {
                relating: id(relating),
                related: id(related),
                realizing: None,
                evidence: proof(&format!("connection:{relating}-{related}")),
            })
            .collect();
        FlowGraph::build(
            &DistributionNetwork::try_new(source(), ports, connections, vec![proof("scan")])
                .unwrap(),
        )
    }

    const OUT: Option<FlowDirection> = Some(FlowDirection::Source);
    const IN: Option<FlowDirection> = Some(FlowDirection::Sink);

    /// Fixtures f1 and f2 drain into pipe p1, which with fixture f3 drains
    /// into pipe p2, which drains into collector c; f4's outlet is open.
    fn drainage() -> Vec<(&'static str, &'static str, Option<FlowDirection>)> {
        vec![
            ("f1o", "f1", OUT),
            ("f2o", "f2", OUT),
            ("f3o", "f3", OUT),
            ("f4o", "f4", OUT),
            ("p1a", "p1", IN),
            ("p1b", "p1", IN),
            ("p1o", "p1", OUT),
            ("p2a", "p2", IN),
            ("p2b", "p2", IN),
            ("p2o", "p2", OUT),
            ("ci", "c", IN),
        ]
    }

    const PIPES: &[(&str, &str)] = &[
        ("f1o", "p1a"),
        ("p1b", "f2o"),
        ("p1o", "p2a"),
        ("f3o", "p2b"),
        ("p2o", "ci"),
    ];

    fn ids(locals: &[&str]) -> BTreeSet<ObjectId> {
        locals.iter().map(|local| id(local)).collect()
    }

    #[test]
    fn flow_reaches_upstream_and_downstream_and_open_ends_are_cited() {
        let graph = network(&drainage(), PIPES);
        let reach = |from: &str, direction, chain| graph.reach(&id(from), direction, chain);
        let (up, _) = reach("p2", TraversalDirection::Backward, true).unwrap();
        assert_eq!(up, ids(&["f1", "f2", "f3", "p1"]));
        let (up, _) = reach("p1", TraversalDirection::Backward, true).unwrap();
        assert_eq!(up, ids(&["f1", "f2"]));
        let (next, _) = reach("f1", TraversalDirection::Forward, false).unwrap();
        assert_eq!(next, ids(&["p1"]));
        let (down, _) = reach("f1", TraversalDirection::Forward, true).unwrap();
        assert_eq!(down, ids(&["c", "p1", "p2"]));
        let (connected, _) = reach("f3", TraversalDirection::Either, true).unwrap();
        assert_eq!(connected, ids(&["c", "f1", "f2", "p1", "p2"]));
        // The open outlet of f4 reaches nothing, and is cited.
        let (down, evidence) = reach("f4", TraversalDirection::Forward, true).unwrap();
        assert!(down.is_empty());
        assert!(
            evidence
                .iter()
                .any(|item| item.locator == format!("{FLOW_RELATIONSHIP}:open-end:{}", id("f4o")))
        );
    }

    #[test]
    fn an_undecided_direction_refuses_directed_reads_only() {
        let mut ports = drainage();
        ports[2].2 = Some(FlowDirection::SourceAndSink);
        let graph = network(&ports, PIPES);
        let why = graph
            .reach(&id("p2"), TraversalDirection::Backward, true)
            .unwrap_err();
        assert!(why.contains("is source-and-sink"), "{why}");
        // Downstream from f1 passes p2, which holds the undecided connection.
        assert!(
            graph
                .reach(&id("f1"), TraversalDirection::Forward, true)
                .is_err()
        );
        // Upstream of p1 meets no undecided connection.
        assert!(
            graph
                .reach(&id("p1"), TraversalDirection::Backward, true)
                .is_ok()
        );
        let (connected, _) = graph
            .reach(&id("f3"), TraversalDirection::Either, true)
            .unwrap();
        assert_eq!(connected, ids(&["c", "f1", "f2", "p1", "p2"]));
        // A port stating no direction is undecided alike.
        let mut ports = drainage();
        ports[2].2 = None;
        let why = network(&ports, PIPES)
            .reach(&id("p2"), TraversalDirection::Backward, true)
            .unwrap_err();
        assert!(why.contains("states no flow direction"), "{why}");
    }

    #[test]
    fn a_cycle_of_flow_refuses_directed_reads() {
        let ports = [
            ("ao", "a", OUT),
            ("ai", "a", IN),
            ("bo", "b", OUT),
            ("bi", "b", IN),
        ];
        let graph = network(&ports, &[("ao", "bi"), ("bo", "ai")]);
        let why = graph
            .reach(&id("a"), TraversalDirection::Forward, true)
            .unwrap_err();
        assert!(why.contains("cycle through"), "{why}");
        assert_eq!(
            graph
                .reach(&id("a"), TraversalDirection::Either, true)
                .unwrap()
                .0,
            ids(&["b"])
        );
    }

    #[test]
    fn a_connection_to_an_unattached_port_refuses_every_read_through_it() {
        let graph = network(&[("ao", "a", OUT), ("x", "", IN)], &[("ao", "x")]);
        for direction in [TraversalDirection::Forward, TraversalDirection::Either] {
            let why = graph.reach(&id("a"), direction, true).unwrap_err();
            assert!(why.contains("belongs to no element"), "{why}");
        }
    }

    struct Listed(DistributionNetwork);

    impl DistributionNetworkService for Listed {
        fn network(&self, source: &SourceId) -> Result<DistributionNetwork, NetworkServiceError> {
            if source == self.0.source() {
                Ok(self.0.clone())
            } else {
                Err(NetworkServiceError::UncoveredSource(source.clone()))
            }
        }
    }

    fn select(
        services: &ServiceRegistry,
        relationship: &str,
        direction: TraversalDirection,
    ) -> Result<Vec<ObjectId>, RelationshipSelectionError> {
        let universe = ["f1", "f2", "p1", "p2", "c"].map(id).to_vec();
        let request = RelationshipSelectionRequest::try_new(
            id("p2"),
            universe,
            RelationshipQuery::Related {
                relationship: crate::SemanticRelationship::try_new(relationship).unwrap(),
                direction,
                follow_chain: true,
            },
        )?;
        let handle = services
            .get::<RelationshipSelectionServiceHandle>()
            .unwrap();
        Ok(handle.select(&request)?.candidates().to_vec())
    }

    #[test]
    fn the_run_answers_the_flow_from_the_network_service_only() {
        let ports = drainage()
            .into_iter()
            .map(|(port, element, flow)| axioval_ir::NetworkPort {
                port: id(port),
                elements: vec![id(element)],
                flow,
                evidence: vec![proof(port)],
            })
            .collect();
        let connections = PIPES
            .iter()
            .map(|(relating, related)| axioval_ir::PortConnection {
                relating: id(relating),
                related: id(related),
                realizing: None,
                evidence: proof(&format!("{relating}-{related}")),
            })
            .collect();
        let listed =
            DistributionNetwork::try_new(source(), ports, connections, vec![proof("scan")])
                .unwrap();
        let mut services = ServiceRegistry::new();
        install(&mut services);
        assert!(
            services
                .get::<RelationshipSelectionServiceHandle>()
                .is_none(),
            "nothing is installed without either service"
        );
        services
            .register(DistributionNetworkServiceHandle::new(Arc::new(Listed(
                listed,
            ))))
            .unwrap();
        install(&mut services);
        assert_eq!(
            select(&services, FLOW_RELATIONSHIP, TraversalDirection::Backward).unwrap(),
            ["f1", "f2", "p1"].map(id)
        );
        assert_eq!(
            select(
                &services,
                "axioval:derived.flow;reach=1",
                TraversalDirection::Forward
            ),
            Err(RelationshipSelectionError::InvalidRequest)
        );
        // Any other identity goes to the host's relationship service: none.
        assert!(matches!(
            select(&services, "IfcRelNests", TraversalDirection::Forward),
            Err(RelationshipSelectionError::Unavailable(_))
        ));
    }
}
