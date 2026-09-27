//! `local-circulation` over circulation maps given by hand.
//!
//! The geometry is exercised end to end in the facade's
//! `axiolid_local_circulation.rs`; these pin the request the capability
//! sends and how it reads a map: proven and possible pieces, link mode and
//! refusals.
#![allow(missing_docs)]

mod common;

use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityEvaluation, CirculationContact, CirculationMap, CirculationRequest, ClearanceOutcome,
    ClearanceRequest, FreeAreaEvidence, FreeAreaRequest, FreeSpaceError, FreeSpaceService,
    FreeSpaceServiceHandle, PlacementOutcome, PlacementRequest,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, NotEvaluatedReason};
use axioval_rules::LocalCirculation;
use common::{
    Model, findings, id, kind, number, rule, selector, source, string, strings, unevaluated,
};

const ID: &str = "axioval:capability.local-circulation";

/// A space `s` with a door `d` and components `a` and `b`.
fn model() -> Model {
    Model::default()
        .object("s", "space")
        .object("d", "door")
        .object("a", "wc")
        .object("b", "wc")
        .object("x", "cabinet")
        .edge("bounds", "d", "s")
        .edge("in", "a", "s")
        .edge("in", "b", "s")
}

/// Contacts by subject: proven pieces, possible pieces.
struct Stub {
    pieces: usize,
    contacts: Vec<(&'static str, Vec<usize>, Vec<usize>)>,
    refuse: bool,
    seen: Mutex<Vec<CirculationRequest>>,
}

impl Stub {
    fn new(pieces: usize, contacts: Vec<(&'static str, Vec<usize>, Vec<usize>)>) -> Self {
        Self {
            pieces,
            contacts,
            refuse: false,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl FreeSpaceService for Stub {
    fn assess_clearance(&self, _: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        panic!("no clearance is asked")
    }

    fn find_placement(&self, _: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
        panic!("no placement is asked without ends or passing spaces")
    }

    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        panic!("no free area is asked")
    }

    fn map_circulation(
        &self,
        request: &CirculationRequest,
    ) -> Result<CirculationMap, FreeSpaceError> {
        self.seen.lock().unwrap().push(request.clone());
        if self.refuse {
            return Err(FreeSpaceError::Unavailable("no floor".into()));
        }
        let contacts = request
            .subjects()
            .into_iter()
            .map(|subject| {
                let (_, reached, possible) = self
                    .contacts
                    .iter()
                    .find(|(local, _, _)| *local == subject.local_id)
                    .cloned()
                    .unwrap_or(("", Vec::new(), Vec::new()));
                CirculationContact::new(
                    subject,
                    reached.into_iter().map(|piece| (piece, None)).collect(),
                    possible,
                )
            })
            .collect();
        CirculationMap::try_new(
            request.clone(),
            self.pieces,
            self.pieces,
            Vec::new(),
            Vec::new(),
            0.05,
            contacts,
            Vec::new(),
            Evidence::exact(source(), "circulation:s"),
        )
    }
}

fn run(stub: Arc<Stub>, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("component_selector", selector(kind("wc"))),
        ("space_path", strings(&["in"])),
        ("access_path", strings(&["bounds"])),
        ("door_selector", selector(kind("door"))),
        (
            "space_selector",
            selector(Selector::EntityType {
                object_type: "space".into(),
                include_subtypes: false,
            }),
        ),
        ("width_metres", number(0.9)),
        ("clear_height_metres", number(2.0)),
    ];
    parameters.extend(extra);
    model().evaluate_with(
        &LocalCirculation,
        &rule(ID, kind("space"), parameters),
        |services| {
            services
                .register(FreeSpaceServiceHandle::new(stub))
                .unwrap();
        },
    )
}

#[test]
fn the_request_names_the_space_its_entrances_components_and_obstacles() {
    let stub = Arc::new(Stub::new(
        1,
        vec![
            ("d", vec![0], vec![0]),
            ("a", vec![0], vec![0]),
            ("b", vec![0], vec![0]),
        ],
    ));
    let evaluation = run(stub.clone(), Vec::new());
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:#?}");
    let seen = stub.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let request = &seen[0];
    assert_eq!(request.scope(), &id("s"));
    assert_eq!(request.entrances(), [id("d")]);
    assert_eq!(request.components(), [id("a"), id("b")]);
    // Every other object obstructs by default, the entrance never.
    assert_eq!(request.obstacles(), [id("a"), id("b"), id("x")]);
    assert!((request.tolerance_metres() - 0.05).abs() < f64::EPSILON);
}

#[test]
fn a_component_only_possibly_near_the_entrance_is_not_evaluated() {
    let stub = Arc::new(Stub::new(
        2,
        vec![
            ("d", vec![0], vec![0]),
            // `a` is proven apart, `b` shares a possible piece.
            ("a", vec![1], vec![1]),
            ("b", vec![1], vec![0, 1]),
        ],
    ));
    let evaluation = run(stub, Vec::new());
    assert_eq!(
        findings(&evaluation),
        [(
            "a".into(),
            format!("no entrance of {} reaches it on a path 0.9 m wide", id("s"))
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("b".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(evaluation.findings()[0].related, [id("d"), id("s")]);
}

#[test]
fn linked_components_need_no_entrance() {
    let stub = || {
        Arc::new(Stub::new(
            2,
            vec![
                ("d", vec![], vec![]),
                ("a", vec![1], vec![1]),
                ("b", vec![0], vec![0]),
            ],
        ))
    };
    let evaluation = run(stub(), vec![("component_mode", string("link"))]);
    assert_eq!(
        findings(&evaluation),
        [
            (
                "a".into(),
                format!(
                    "no path 0.9 m wide in {} links it with {}",
                    id("s"),
                    id("b")
                )
            ),
            (
                "b".into(),
                format!(
                    "no path 0.9 m wide in {} links it with {}",
                    id("s"),
                    id("a")
                )
            ),
        ]
    );
    let evaluation = run(
        Arc::new(Stub::new(
            1,
            vec![
                ("d", vec![], vec![]),
                ("a", vec![0], vec![0]),
                ("b", vec![0], vec![0]),
            ],
        )),
        vec![("component_mode", string("link"))],
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(unevaluated(&evaluation).is_empty(), "{evaluation:#?}");
}

#[test]
fn refusals_and_missing_services_are_not_evaluated() {
    let mut refusing = Stub::new(1, Vec::new());
    refusing.refuse = true;
    let evaluation = run(Arc::new(refusing), Vec::new());
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("s".into(), NotEvaluatedReason::BackendUnavailable),
            ("a".into(), NotEvaluatedReason::BackendUnavailable),
            ("b".into(), NotEvaluatedReason::BackendUnavailable),
        ]
    );
    let evaluation = model().evaluate(
        &LocalCirculation,
        &rule(
            ID,
            kind("space"),
            vec![
                ("component_selector", selector(kind("wc"))),
                ("space_path", strings(&["in"])),
                ("access_path", strings(&["bounds"])),
                ("door_selector", selector(kind("door"))),
                ("width_metres", number(0.9)),
                ("clear_height_metres", number(2.0)),
            ],
        ),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("s".into(), NotEvaluatedReason::MissingService)]
    );
    // The width, the height and the entrances are required.
    let evaluation = run(
        Arc::new(Stub::new(1, Vec::new())),
        vec![("width_metres", number(-1.0))],
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
    );
}
