//! `accessible-route` over a walkability graph given by hand.
//!
//! The Axiolid-measured cases live in the facade's
//! `axiolid_accessible_route.rs`; these pin the declaration, the request the
//! rule sends and the three-valued judgement on graphs chosen to isolate one
//! decision each.
#![allow(missing_docs)]
#![allow(clippy::float_cmp)] // widths are copied, never computed

mod common;

use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityEvaluation, LengthInterval, VerifiedWalkablePassage, VerticalConnector,
    VerticalConnectorKind, WalkabilityError, WalkabilityRegion, WalkabilityRegionId,
    WalkabilityRequest, WalkabilityService, WalkabilityServiceHandle, WalkabilitySnapshot,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::AccessibleRoute;
use common::{
    Model, boolean, findings, id, kind, number, property, rule, selector, source, unevaluated,
};

const ID: &str = "axioval:capability.accessible-route";

/// No width bound: a passage nothing narrows.
const OPEN: f64 = 1e6;

/// One passage: `(a, b, portal, stair, lower, upper)`, by local id.
type Edge = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    f64,
    f64,
);

/// A snapshot with one region per requested surface and entrance, named by
/// its local id, and the given passages; it keeps the last request.
struct Graph {
    edges: Vec<Edge>,
    seen: Mutex<Option<WalkabilityRequest>>,
}

impl Graph {
    fn new(edges: Vec<Edge>) -> Arc<Self> {
        Arc::new(Self {
            edges,
            seen: Mutex::new(None),
        })
    }
}

fn region(local: &str) -> WalkabilityRegionId {
    WalkabilityRegionId::new(local).unwrap()
}

impl WalkabilityService for Graph {
    fn snapshot(
        &self,
        request: &WalkabilityRequest,
    ) -> Result<WalkabilitySnapshot, WalkabilityError> {
        *self.seen.lock().unwrap() = Some(request.clone());
        let regions = request
            .surfaces()
            .iter()
            .chain(request.entrances())
            .map(|object| WalkabilityRegion::new(region(&object.local_id), vec![object.clone()]))
            .collect();
        let mut passages = Vec::new();
        for &(a, b, portal, stair, lower, upper) in &self.edges {
            let mut passage = VerifiedWalkablePassage::try_new(
                region(a),
                region(b),
                portal.map(id),
                LengthInterval::try_new(lower, upper).unwrap(),
                Evidence::exact(source(), format!("passage:{a}-{b}")),
            )?;
            if let Some(stair) = stair {
                passage = passage.with_connector(VerticalConnector::new(
                    id(stair),
                    VerticalConnectorKind::Stair,
                ))?;
            }
            passages.push(passage);
        }
        WalkabilitySnapshot::try_new(
            request.clone(),
            regions,
            passages,
            Evidence::exact(source(), "graph:complete"),
        )
    }
}

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

/// Lobby `a`, rooms `b` and `c`, door `door`, stair `stair`, wall `wall`.
fn model() -> Model {
    Model::default()
        .object("a", "lobby")
        .object("b", "room")
        .object("c", "room")
        .object("door", "door")
        .object("stair", "stair")
        .object("wall", "wall")
}

fn parameters(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut all = vec![
        (
            "route_selector",
            selector(Selector::AnyOf {
                operands: vec![kind("lobby"), kind("room")],
            }),
        ),
        ("start_selector", selector(kind("lobby"))),
        ("portal_selector", selector(kind("door"))),
        ("stair_selector", selector(kind("stair"))),
        ("obstacle_selector", selector(kind("wall"))),
        ("width_metres", number(0.8)),
        ("door_width_metres", number(0.8)),
        (
            "clear_width_property",
            property(Some("Access"), "ClearWidth"),
        ),
    ];
    for (name, value) in extra {
        all.retain(|(existing, _)| *existing != name);
        all.push((name, value));
    }
    all
}

fn run(
    model: Model,
    graph: &Arc<Graph>,
    extra: Vec<(&'static str, ParameterValue)>,
) -> CapabilityEvaluation {
    let graph = graph.clone();
    model.evaluate_with(
        &AccessibleRoute,
        &rule(ID, kind("room"), parameters(extra)),
        |services| {
            services
                .register(WalkabilityServiceHandle::new(graph))
                .unwrap();
        },
    )
}

/// `a` – `door` – `b`, the crossing proven 0.8 m wide; `a` – `stair` – `c`.
fn house() -> Arc<Graph> {
    Graph::new(vec![
        ("a", "door", None, None, 0.8, OPEN),
        ("door", "b", Some("door"), None, 0.8, 0.85),
        ("a", "c", None, Some("stair"), 0.0, OPEN),
    ])
}

#[test]
fn a_proven_route_passes_and_a_stairs_only_room_is_found() {
    let graph = house();
    let outcome = run(
        model().value("door", "Access", "ClearWidth", metres(0.85)),
        &graph,
        vec![],
    );
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    assert_eq!(
        findings(&outcome),
        [(
            "c".to_owned(),
            "test:model/c is connected to the starts by stairs only: test:model/stair".to_owned()
        )]
    );
    assert_eq!(
        outcome.findings()[0].related,
        vec![id("stair")],
        "{outcome:#?}"
    );
    // The request carries the profile, the roles and the stated width.
    let request = graph.seen.lock().unwrap().clone().unwrap();
    assert_eq!(request.minimum_width_metres(), 0.8);
    assert_eq!(request.surfaces(), [id("a"), id("b"), id("c")]);
    assert_eq!(request.entrances(), [id("door")]);
    assert_eq!(request.obstacles(), [id("wall")]);
    assert_eq!(
        request.connectors(),
        [VerticalConnector::new(
            id("stair"),
            VerticalConnectorKind::Stair
        )]
    );
    assert_eq!(request.stated_clear_width(&id("door")), Some(0.85));
    assert!(request.traverses_verified_portals());
    assert_eq!(request.elevation_band(), None);
}

#[test]
fn a_clear_height_bounds_the_headroom_band() {
    let graph = house();
    run(model(), &graph, vec![("clear_height_metres", number(2.1))]);
    let request = graph.seen.lock().unwrap().clone().unwrap();
    assert_eq!(
        request.elevation_band(),
        Some(LengthInterval::try_new(0.0, 2.1).unwrap())
    );
}

#[test]
fn a_door_stating_too_little_clear_width_blocks_the_room_behind_it() {
    let outcome = run(
        model().value("door", "Access", "ClearWidth", metres(0.75)),
        &house(),
        vec![("forbid_stairs", boolean(false))],
    );
    let found = findings(&outcome);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, "b");
    assert!(
        found[0].1.contains(
            "test:model/door states a clear width of 0.75 m, less than the 0.8 m required"
        ),
        "{found:#?}"
    );
    assert_eq!(outcome.findings()[0].related, vec![id("door")]);
    // The stair is allowed, but a climb is never proven.
    assert_eq!(
        unevaluated(&outcome),
        [("c".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_door_that_states_no_width_is_undecided_unless_the_geometry_decides() {
    // Proven 0.8 m for the body, and 0.8 m is all the door needs.
    let outcome = run(model(), &house(), vec![]);
    assert!(findings(&outcome).iter().all(|(object, _)| object != "b"));
    assert!(unevaluated(&outcome).is_empty(), "{outcome:#?}");
    // A 0.82 m door minimum is neither proven nor ruled out by 0.8..0.85.
    let outcome = run(model(), &house(), vec![("door_width_metres", number(0.82))]);
    assert!(findings(&outcome).iter().all(|(object, _)| object != "b"));
    assert!(
        outcome.not_evaluated_outcomes().iter().any(|outcome| {
            outcome.object_id() == Some(&id("b"))
                && outcome
                    .message()
                    .contains("test:model/door states no clear width")
        }),
        "{outcome:#?}"
    );
    // 0.86 m is more than the geometry allows: blocked.
    let outcome = run(model(), &house(), vec![("door_width_metres", number(0.86))]);
    assert!(
        findings(&outcome)
            .iter()
            .any(|(object, message)| object == "b"
                && message.contains("at most 0.85 m wide in the geometry")),
        "{outcome:#?}"
    );
}

#[test]
fn a_room_nothing_joins_is_found_without_a_blocking_element() {
    let graph = Graph::new(vec![("a", "door", None, None, 0.8, OPEN)]);
    let outcome = run(model(), &graph, vec![]);
    let found = findings(&outcome);
    assert!(
        found.contains(&(
            "b".to_owned(),
            "no route space connects test:model/b to a start".to_owned()
        )),
        "{found:#?}"
    );
}

#[test]
fn a_door_too_narrow_for_the_body_blocks_as_a_start() {
    // The door is the start; its own crossing is too narrow.
    let graph = Graph::new(vec![
        ("door", "b", Some("door"), None, 0.0, 0.7),
        ("door", "c", None, None, 0.8, OPEN),
    ]);
    let outcome = run(
        model(),
        &graph,
        vec![
            ("start_selector", selector(kind("door"))),
            ("door_width_metres", number(0.5)),
        ],
    );
    let found = findings(&outcome);
    assert_eq!(found.len(), 2, "{found:#?}");
    for finding in outcome.findings() {
        assert_eq!(finding.related, vec![id("door")], "{finding:#?}");
    }
    assert!(
        found[0]
            .1
            .contains("test:model/door is at most 0.7 m wide, less than the 0.8 m body"),
        "{found:#?}"
    );
}

#[test]
fn a_room_off_the_route_is_not_crossed() {
    // `c` lies between the lobby and `b`, but only the lobby is a route
    // space: `b` is blocked by `c`, and `c` itself is reached.
    let graph = Graph::new(vec![
        ("a", "c", None, None, 0.8, OPEN),
        ("c", "b", None, None, 0.8, OPEN),
    ]);
    let outcome = run(
        model(),
        &graph,
        vec![("route_selector", selector(kind("lobby")))],
    );
    assert_eq!(
        findings(&outcome),
        [(
            "b".to_owned(),
            "no accessible route reaches test:model/b for a body 0.8 m wide: \
             test:model/c is not a route space"
                .to_owned()
        )]
    );
    assert_eq!(outcome.findings()[0].related, vec![id("c")]);
}

#[test]
fn declarations_and_missing_evidence_are_not_evaluated() {
    let invalid = run(model(), &house(), vec![("width_metres", number(-1.0))]);
    assert_eq!(
        unevaluated(&invalid),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    let twice = run(
        model(),
        &house(),
        vec![("ramp_selector", selector(kind("stair")))],
    );
    assert!(
        unevaluated(&twice)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration),
        "{twice:#?}"
    );
    assert_eq!(unevaluated(&twice).len(), 2);
    let missing = model().evaluate(
        &AccessibleRoute,
        &rule(ID, kind("room"), parameters(vec![])),
    );
    assert_eq!(
        unevaluated(&missing),
        [
            ("b".to_owned(), NotEvaluatedReason::MissingService),
            ("c".to_owned(), NotEvaluatedReason::MissingService),
        ]
    );
    // An obstacle whose selection is undecided could block or free any route.
    let undecided = run(
        model().unreadable("wall"),
        &house(),
        vec![(
            "obstacle_selector",
            selector(Selector::Property {
                property_set: Some("Access".into()),
                property: "Obstacle".into(),
                operator: axioval_ir::contract::ComparisonOperator::Exists,
                value: None,
                case_sensitive: true,
                trim: false,
                quantifier: None,
                precision: None,
            }),
        )],
    );
    assert!(
        unevaluated(&undecided)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::IncompleteEvidence),
        "{undecided:#?}"
    );
    assert!(undecided.findings().is_empty());
}
