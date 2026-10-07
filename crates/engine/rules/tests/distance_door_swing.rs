//! Distance from and to door swings: the floor sector a door's hinged
//! leaves sweep, bracketed between inscribed and circumscribed polygons.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    Bounds3, CapabilityEvaluation, ConvexPlanRegion, DoorLeavesError, GeometryFidelity,
    NotEvaluatedReason, ObjectBounds, ProximityError, ProximityEvidence, ProximityRequest,
    ProximityService, ProximityServiceHandle, RegionDistanceEvidence, RegionDistanceRequest,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, ObjectId};
use axioval_rules::Distance;
use common::doors::{Doors, bottom_hung, hinged, sliding};
use common::{Model, kind, number, rule, selector, source, string, unevaluated};

const CAPABILITY: &str = "axioval:capability.distance";

/// `distance` runs as a template, held to the implementation it replaced
/// on every fixture.
static HELD: common::Held = common::Held(&Distance, &axioval_rules::reference::Distance);

/// Plan boxes per object, measured exactly.
#[derive(Default)]
struct Boxes(BTreeMap<String, ([f64; 2], [f64; 2])>);

impl Boxes {
    fn at(mut self, local: &str, low: [f64; 2], high: [f64; 2]) -> Self {
        self.0.insert(local.into(), (low, high));
        self
    }
}

impl ProximityService for Boxes {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let (low, high) = self
            .0
            .get(&object.local_id)
            .ok_or(ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([low[0], low[1], 0.0], [high[0], high[1], 1.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        panic!("door swings are measured as regions")
    }

    fn measure_region_distance(
        &self,
        request: &RegionDistanceRequest,
    ) -> Result<RegionDistanceEvidence, ProximityError> {
        let counterpart = request.counterpart();
        let (low, high) = self
            .0
            .get(&counterpart.local_id)
            .ok_or(ProximityError::Unavailable)?;
        let plan =
            ConvexPlanRegion::try_new(vec![*low, [high[0], low[1]], *high, [low[0], high[1]]])
                .unwrap();
        let distance = request.region().separation(&plan).max(0.0);
        RegionDistanceEvidence::try_new(
            request.clone(),
            distance,
            distance,
            GeometryFidelity::Exact,
            Evidence::exact(source(), format!("region:{}", counterpart.local_id)),
        )
    }
}

/// Door `a` hinged at the origin, swinging from +x to +y, 0.9 m; door `c`
/// hinged at (2, 0), swinging from -x to +y, so the two closed leaves end
/// 0.2 m apart; `b` slides and sweeps nothing; `d`'s leaves are unknown.
/// Column `column` stands 0.3166 m beyond `a`'s arc, `far` 4.1 m away.
fn doors() -> Doors {
    Doors::default()
        .door(
            "a",
            vec![hinged(
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                0.9,
                false,
            )],
            1.0,
            None,
        )
        .door(
            "b",
            vec![sliding([4.0, -3.0, 0.0], [1.0, 0.0, 0.0], 0.9)],
            1.0,
            None,
        )
        .door(
            "c",
            vec![hinged(
                [2.0, 0.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                0.9,
                false,
            )],
            1.0,
            None,
        )
        .unknown("d", DoorLeavesError::NotStated("no panels".into()))
}

fn boxes() -> Boxes {
    Boxes::default()
        .at("column", [0.2, 1.2], [0.4, 1.4])
        .at("far", [5.0, 0.0], [6.0, 1.0])
}

fn model() -> Model {
    ["a", "b", "c", "d"]
        .into_iter()
        .fold(Model::default(), |model, door| model.object(door, "door"))
        .object("column", "column")
        .object("far", "column")
}

fn run(
    subjects: &str,
    counterparts: &str,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut all = vec![
        ("counterparts", selector(kind(counterparts))),
        ("projection", string("horizontal")),
    ];
    all.extend(parameters);
    let proximity = Arc::new(boxes());
    let frames = doors().handle();
    model().evaluate_measured(
        &HELD,
        &rule(CAPABILITY, kind(subjects), all),
        move |services| {
            services
                .register(ProximityServiceHandle::new(proximity.clone()))
                .unwrap();
            services.register(frames.clone()).unwrap();
        },
    )
}

fn findings(outcome: &CapabilityEvaluation) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = outcome
        .findings()
        .iter()
        .map(|finding| (common::subject(finding), finding.message.clone()))
        .collect();
    found.sort();
    found
}

#[test]
fn a_column_too_close_to_a_door_swing_is_found() {
    let outcome = run(
        "door",
        "column",
        vec![
            ("subject_extent", string("door_swing")),
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.5)),
        ],
    );
    let found = findings(&outcome);
    let [(door, message)] = &found[..] else {
        panic!("one finding: {found:?}")
    };
    assert_eq!(door, "a");
    // (0.2, 1.2) lies √1.48 ≈ 1.2166 m from the hinge, 0.3166 m past the
    // arc; the polygons bracket it within a tenth of a millimetre.
    assert!(
        message.contains("/column at horizontal distance between 0.316"),
        "{message}"
    );
    // The sliding door sweeps nothing; the door without leaves is open.
    assert_eq!(
        unevaluated(&outcome),
        vec![("d".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn two_door_swings_too_close_find_each_other() {
    let outcome = run(
        "door",
        "door",
        vec![
            ("subject_extent", string("door_swing")),
            ("counterpart_extent", string("door_swing")),
            ("mode", string("none_closer_than")),
            ("minimum_metres", number(0.5)),
        ],
    );
    let found = findings(&outcome);
    assert_eq!(
        found
            .iter()
            .map(|(door, _)| door.as_str())
            .collect::<Vec<_>>(),
        ["a", "c"],
        "{found:?}"
    );
    assert!(
        found[0].1.contains("horizontal distance 0.2000 m"),
        "{}",
        found[0].1
    );
}

#[test]
fn a_body_is_measured_to_the_nearest_door_swing() {
    let within = |maximum: f64| {
        run(
            "column",
            "door",
            vec![
                ("counterpart_extent", string("door_swing")),
                ("maximum_metres", number(maximum)),
            ],
        )
    };
    let outcome = within(0.5);
    // `column` has `a` within reach; `far` has nothing within 0.5 m, which
    // no unknown door can change unless it may lie within: `d` may.
    assert!(outcome.findings().is_empty(), "{:?}", outcome.findings());
    let open: Vec<String> = unevaluated(&outcome)
        .into_iter()
        .map(|(object, _)| object)
        .collect();
    assert!(open.contains(&"far".to_owned()) && !open.contains(&"column".to_owned()));
    let outcome = within(0.2);
    assert!(
        !unevaluated(&outcome)
            .iter()
            .any(|(object, _)| object == "a" || object == "c")
    );
    assert!(outcome.findings().is_empty());
}

#[test]
fn a_door_swing_is_measured_in_plan_only() {
    let outcome = run(
        "door",
        "column",
        vec![
            ("subject_extent", string("door_swing")),
            ("projection", string("minimum_3d")),
            ("minimum_metres", number(0.5)),
        ],
    );
    assert!(
        unevaluated(&outcome)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration)
    );
}

#[test]
fn a_window_swing_is_measured_by_its_casement_and_tilt() {
    // Window `w` has a casement hinged at (0, 3, 0.9) swinging from +x to
    // +y, 0.6 m wide, and a bottom-hung panel beside it from (0.6, 3, 0.9),
    // 0.5 m wide and 0.8 m high: in plan it tilts over x 0.6 to 1.1, y 3
    // to 3.8. Column `post` stands at x 1.3 to 1.5, 0.2 m beyond it.
    let windows = Doors::default().door(
        "w",
        vec![
            hinged(
                [0.0, 3.0, 0.9],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                0.6,
                false,
            ),
            bottom_hung([0.6, 3.0, 0.9], 0.5, 0.8),
        ],
        1.1,
        None,
    );
    let proximity = Arc::new(Boxes::default().at("post", [1.3, 3.2], [1.5, 3.4]));
    let model = Model::default()
        .object("w", "window")
        .object("post", "column");
    let frames = windows.handle();
    let outcome = model.evaluate_measured(
        &HELD,
        &rule(
            CAPABILITY,
            kind("window"),
            vec![
                ("counterparts", selector(kind("column"))),
                ("projection", string("horizontal")),
                ("subject_extent", string("leaf_swing")),
                ("mode", string("none_closer_than")),
                ("minimum_metres", number(0.5)),
            ],
        ),
        move |services| {
            services
                .register(ProximityServiceHandle::new(proximity.clone()))
                .unwrap();
            services.register(frames.clone()).unwrap();
        },
    );
    let found = findings(&outcome);
    let [(window, message)] = &found[..] else {
        panic!("one finding: {found:?}")
    };
    assert_eq!(window, "w");
    assert!(
        message.contains("/post at horizontal distance 0.2000 m"),
        "{message}"
    );
    assert!(unevaluated(&outcome).is_empty());
}
