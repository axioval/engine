//! Source-neutral connectivity graph contract tests.

use axioval_engine::{
    CompleteTopologyEvidence, ConnectivityGraph, RouteOutcome, TopologyError, VerifiedConnection,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

fn source(document: &str) -> SourceId {
    SourceId::new("test", document).unwrap()
}

fn object(document: &str, local_id: &str) -> ObjectId {
    ObjectId::new(source(document), local_id).unwrap()
}

fn connection(left: &ObjectId, right: &ObjectId, width: f64) -> VerifiedConnection {
    VerifiedConnection::try_new(
        left.clone(),
        right.clone(),
        width,
        Evidence::exact(source("topology"), format!("{left}--{right}")),
    )
    .unwrap()
}

fn graph(
    nodes: impl IntoIterator<Item = ObjectId>,
    connections: impl IntoIterator<Item = VerifiedConnection>,
) -> Result<ConnectivityGraph, TopologyError> {
    ConnectivityGraph::try_new(
        nodes,
        connections,
        CompleteTopologyEvidence::try_new(Evidence::exact(
            source("topology"),
            "complete-projection",
        ))
        .unwrap(),
    )
}

#[test]
fn graph_preserves_source_qualified_identity() {
    let left = object("model-a", "same-local-id");
    let right = object("model-b", "same-local-id");
    let graph = graph(
        [left.clone(), right.clone()],
        [connection(&left, &right, 1.0)],
    )
    .unwrap();

    assert_eq!(
        graph.route(&left, &right, 0.9).unwrap(),
        RouteOutcome::Route(vec![left, right])
    );
}

#[test]
fn route_is_deterministic_when_shortest_paths_tie() {
    let start = object("model", "start");
    let first = object("model", "a");
    let second = object("model", "b");
    let end = object("model", "end");
    let graph = graph(
        [start.clone(), first.clone(), second.clone(), end.clone()],
        [
            connection(&start, &second, 1.0),
            connection(&second, &end, 1.0),
            connection(&start, &first, 1.0),
            connection(&first, &end, 1.0),
        ],
    )
    .unwrap();

    assert_eq!(
        graph.route(&start, &end, 0.8).unwrap(),
        RouteOutcome::Route(vec![start, first, end])
    );
}

#[test]
fn width_constraint_does_not_promote_narrow_connection() {
    let left = object("model", "left");
    let right = object("model", "right");
    let graph = graph(
        [left.clone(), right.clone()],
        [connection(&left, &right, 0.79)],
    )
    .unwrap();

    assert_eq!(
        graph.route(&left, &right, 0.8).unwrap(),
        RouteOutcome::Unreachable
    );
}

#[test]
fn unknown_node_is_an_error_not_unreachable() {
    let known = object("model", "known");
    let unknown = object("model", "unknown");
    let graph = graph([known.clone()], []).unwrap();

    assert_eq!(
        graph.route(&known, &unknown, 0.0),
        Err(TopologyError::UnknownNode(Box::new(unknown)))
    );
}

#[test]
fn inexact_connection_is_rejected() {
    let left = object("model", "left");
    let right = object("model", "right");
    let evidence = Evidence {
        source: source("topology"),
        locator: "candidate-only".into(),
        exact: false,
    };

    assert_eq!(
        VerifiedConnection::try_new(left, right, 1.0, evidence),
        Err(TopologyError::InexactConnection)
    );
}

#[test]
fn connection_outside_declared_universe_is_rejected() {
    let declared = object("model", "declared");
    let missing = object("model", "missing");

    assert_eq!(
        graph([declared.clone()], [connection(&declared, &missing, 1.0)]),
        Err(TopologyError::UnknownEndpoint(Box::new(missing)))
    );
}

#[test]
fn reachable_component_is_sorted_and_width_constrained() {
    let one = object("model", "1");
    let two = object("model", "2");
    let three = object("model", "3");
    let graph = graph(
        [three.clone(), one.clone(), two.clone()],
        [connection(&two, &three, 0.6), connection(&one, &two, 1.0)],
    )
    .unwrap();

    assert_eq!(graph.reachable_from(&one, 0.8).unwrap(), vec![one, two]);
}

#[test]
fn duplicate_undirected_connection_is_rejected() {
    let one = object("model", "1");
    let two = object("model", "2");
    let result = graph(
        [one.clone(), two.clone()],
        [connection(&one, &two, 1.0), connection(&two, &one, 1.0)],
    );
    assert_eq!(
        result,
        Err(TopologyError::DuplicateConnection {
            left: Box::new(one),
            right: Box::new(two),
        })
    );
}

#[test]
fn invalid_width_threshold_fails_instead_of_widening_route() {
    let one = object("model", "1");
    let graph = graph([one.clone()], []).unwrap();
    assert_eq!(
        graph.reachable_from(&one, f64::NAN),
        Err(TopologyError::InvalidWidth("NaN".into()))
    );
}

#[test]
fn duplicate_nodes_are_rejected() {
    let one = object("model", "1");
    assert_eq!(
        graph([one.clone(), one.clone()], []),
        Err(TopologyError::DuplicateNode(Box::new(one)))
    );
}

#[test]
fn incomplete_topology_coverage_cannot_construct_a_graph() {
    let evidence = Evidence {
        source: source("topology"),
        locator: "partial-projection".into(),
        exact: false,
    };
    assert_eq!(
        CompleteTopologyEvidence::try_new(evidence),
        Err(TopologyError::InexactTopologyCoverage)
    );
}
