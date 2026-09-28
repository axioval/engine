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
    CapabilityEvaluation, ElevationInterval, Headroom, HeadroomRequest, LengthInterval,
    MetricDirection, SlopedRun, SlopedSurface, StretchLimit, TreadFlight, TreadFlightRequest,
    VerifiedWalkablePassage, VerticalConnector, VerticalConnectorKind, WalkabilityError,
    WalkabilityRegion, WalkabilityRegionId, WalkabilityRequest, WalkabilityService,
    WalkabilityServiceHandle, WalkabilitySnapshot, WalkableStretch, WalkingSurfaceError,
    WalkingSurfaceService, WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension};
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
                let kind = if stair.starts_with("ramp") {
                    VerticalConnectorKind::Ramp
                } else {
                    VerticalConnectorKind::Stair
                };
                passage = passage.with_connector(VerticalConnector::new(id(stair), kind))?;
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

#[test]
fn the_obstruction_depth_and_surface_gap_go_into_the_request() {
    let graph = house();
    run(
        model(),
        &graph,
        vec![
            ("obstruction_depth_metres", number(0.01)),
            ("surface_gap_metres", number(0.05)),
        ],
    );
    let request = graph.seen.lock().unwrap().clone().unwrap();
    assert_eq!(request.obstruction_depth_metres(), 0.01);
    assert_eq!(request.surface_gap_metres(), 0.05);
    // Without them, nothing is tolerated.
    let graph = house();
    run(model(), &graph, vec![]);
    let request = graph.seen.lock().unwrap().clone().unwrap();
    assert_eq!(request.obstruction_depth_metres(), 0.0);
    assert_eq!(request.surface_gap_metres(), 0.0);
    for name in ["obstruction_depth_metres", "surface_gap_metres"] {
        let invalid = run(model(), &house(), vec![(name, number(-0.01))]);
        assert_eq!(
            unevaluated(&invalid),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{name}"
        );
    }
}

/// The lobby `a` reaches the west part of room `c`; the east part, which
/// room `b` opens from, lies past a stretch of `c` of the given limit.
struct Pinched {
    limit: StretchLimit,
    obstacles: Vec<&'static str>,
}

impl WalkabilityService for Pinched {
    fn snapshot(
        &self,
        request: &WalkabilityRequest,
    ) -> Result<WalkabilitySnapshot, WalkabilityError> {
        let regions = vec![
            WalkabilityRegion::new(region("a"), vec![id("a")]),
            WalkabilityRegion::new(region("b"), vec![id("b")]),
            WalkabilityRegion::new(region("c-west"), vec![id("c")]),
            WalkabilityRegion::new(region("c-east"), vec![id("c")]),
        ];
        let open = |a: &str, b: &str| {
            VerifiedWalkablePassage::try_new(
                region(a),
                region(b),
                None,
                LengthInterval::try_new(0.8, OPEN).unwrap(),
                Evidence::exact(source(), format!("passage:{a}-{b}")),
            )
        };
        let mut stretch = WalkableStretch::try_new(
            id("c"),
            self.limit,
            [6.1, 2.0, 0.0],
            self.obstacles.iter().map(|local| id(local)).collect(),
        )?;
        if self.limit == StretchLimit::Low {
            stretch = stretch.with_headroom(LengthInterval::try_new(1.8, 1.8).unwrap())?;
        }
        let pinch = VerifiedWalkablePassage::try_new(
            region("c-west"),
            region("c-east"),
            None,
            LengthInterval::try_new(0.0, 0.4).unwrap(),
            Evidence::exact(source(), "stretch:c"),
        )?
        .with_stretch(stretch)?;
        WalkabilitySnapshot::try_new(
            request.clone(),
            regions,
            vec![open("a", "c-west")?, pinch, open("c-east", "b")?],
            Evidence::exact(source(), "graph:complete"),
        )
    }
}

fn pinched(limit: StretchLimit, obstacles: Vec<&'static str>) -> CapabilityEvaluation {
    let service = Arc::new(Pinched { limit, obstacles });
    model().evaluate_with(
        &AccessibleRoute,
        &rule(ID, kind("room"), parameters(vec![])),
        |services| {
            services
                .register(WalkabilityServiceHandle::new(service))
                .unwrap();
        },
    )
}

#[test]
fn a_block_inside_a_route_space_is_reported_where_it_lies() {
    let expect = |outcome: &CapabilityEvaluation, reason: &str, related: Vec<&str>| {
        assert!(unevaluated(outcome).is_empty(), "{outcome:#?}");
        assert_eq!(
            findings(outcome),
            [(
                "b".to_owned(),
                format!("no accessible route reaches test:model/b for a body 0.8 m wide: {reason}")
            )]
        );
        assert_eq!(
            outcome.findings()[0].related,
            related.into_iter().map(id).collect::<Vec<_>>()
        );
        assert!(
            outcome.findings()[0]
                .evidence
                .iter()
                .any(|evidence| evidence.locator == "stretch:c"),
            "{outcome:#?}"
        );
    };
    expect(
        &pinched(StretchLimit::Narrow, vec![]),
        "test:model/c is too narrow near (6.1, 2, 0) for a body 0.8 m wide",
        vec!["c"],
    );
    expect(
        &pinched(StretchLimit::Obstructed, vec!["wall"]),
        "test:model/c is obstructed near (6.1, 2, 0) for a body 0.8 m wide by test:model/wall",
        vec!["c", "wall"],
    );
    expect(
        &pinched(StretchLimit::Low, vec!["wall"]),
        "test:model/c is too low near (6.1, 2, 0) for a body 0.8 m wide: the headroom under \
         test:model/wall is 1.8 m",
        vec!["c", "wall"],
    );
}

/// Measures every ramp as one run of the given width.
struct Ramps {
    width: f64,
}

impl WalkingSurfaceService for Ramps {
    fn measure_tread_flight(
        &self,
        request: &TreadFlightRequest,
    ) -> Result<TreadFlight, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "{} is no flight",
            request.object()
        )))
    }

    fn measure_sloped_runs(&self, object: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError> {
        let at = |value: f64| ElevationInterval::exact(value).unwrap();
        let run = SlopedRun::try_new(
            MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
            at(0.0),
            at(0.5),
            at(0.0),
            at(6.0),
        )?
        .with_sides(at(0.0), at(self.width))?;
        SlopedSurface::try_new(
            object.clone(),
            vec![run],
            Evidence::exact(source(), format!("sloped-runs:{object}")),
        )
    }

    fn measure_headroom(&self, request: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "no headroom over {}",
            request.subject()
        )))
    }
}

/// `c` is reached from the lobby by `ramp` alone.
fn ramped(measured: Option<f64>, stated: Option<f64>) -> CapabilityEvaluation {
    let graph = Graph::new(vec![("a", "c", None, Some("ramp"), 0.0, OPEN)]);
    let mut model = model().object("ramp", "ramp");
    if let Some(stated) = stated {
        model = model.value("ramp", "Access", "ClearWidth", metres(stated));
    }
    model.evaluate_with(
        &AccessibleRoute,
        &rule(
            ID,
            kind("room"),
            parameters(vec![
                ("ramp_selector", selector(kind("ramp"))),
                ("ramp_width_metres", number(1.2)),
            ]),
        ),
        |services| {
            services
                .register(WalkabilityServiceHandle::new(graph))
                .unwrap();
            if let Some(width) = measured {
                services
                    .register(WalkingSurfaceServiceHandle::new(Arc::new(Ramps { width })))
                    .unwrap();
            }
        },
    )
}

#[test]
fn a_ramp_stating_no_width_is_judged_by_its_measured_runs() {
    // Measured 1 m wide: narrower than the 1.2 m required.
    let narrow = ramped(Some(1.0), None);
    let found = findings(&narrow);
    assert!(
        found.contains(&(
            "c".to_owned(),
            "no accessible route reaches test:model/c for a body 0.8 m wide: ramp \
             test:model/ramp is at most 1 m wide in the geometry, less than the 1.2 m required"
                .to_owned()
        )),
        "{found:#?}"
    );
    let finding = narrow
        .findings()
        .iter()
        .find(|finding| finding.message.contains("test:model/c "))
        .unwrap();
    assert_eq!(finding.related, vec![id("ramp")]);
    assert!(
        finding
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "sloped-runs:test:model/ramp"),
        "{finding:#?}"
    );
    let open = |outcome: &CapabilityEvaluation| {
        outcome
            .not_evaluated_outcomes()
            .iter()
            .find(|outcome| outcome.object_id() == Some(&id("c")))
            .map(|outcome| outcome.message().to_owned())
            .unwrap()
    };
    // Wide enough: the climb itself is never proven, but the width is not
    // why.
    let wide = ramped(Some(1.5), None);
    assert!(findings(&wide).iter().all(|(object, _)| object != "c"));
    assert!(
        !open(&wide).contains("test:model/ramp is"),
        "{}",
        open(&wide)
    );
    // A stated width wins over the geometry.
    let stated = ramped(Some(1.0), Some(1.3));
    assert!(findings(&stated).iter().all(|(object, _)| object != "c"));
    // Without the walking-surface service the width is not measured.
    let unmeasured = ramped(None, None);
    assert!(
        open(&unmeasured).contains(
            "ramp test:model/ramp states no clear width to compare with 1.2 m, and its width \
             is not measured: the walking-surface service is not registered"
        ),
        "{}",
        open(&unmeasured)
    );
}
