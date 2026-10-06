//! Effective coverage of rooms by the effect areas of their devices.
//!
//! `effective-coverage` runs as a template (#282); every fixture runs it
//! and the implementation it replaced
//! (`axioval_rules::reference::EffectiveCoverage`) and holds the template
//! to the whole outside contract (`Parity::contract()`).
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axioval_engine::{
    Bounds3, CapabilityEvaluation, CompiledRule, CoverageEvidence, CoverageRequest, EffectMeets,
    EffectReach, GeometryFidelity, ObjectBounds, Participant, PlanArea, PlanAreaError,
    PlanAreaService, PlanAreaServiceHandle, ProjectedDistanceEvidence, ProximityError,
    ProximityEvidence, ProximityProjection, ProximityRequest, ProximityService,
    ProximityServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue};
use axioval_rules::EffectiveCoverage;
use axioval_rules::reference::EffectiveCoverage as Reference;
use common::{
    Model, id, kind, number, property, rule, selector, source, string, strings, unevaluated,
};

const ID: &str = "axioval:capability.effective-coverage";

/// Plan rectangles `(x0, y0, x1, y1)` per object, and per source the
/// rectangles its effect covers at least and at most: a stub that knows
/// its answers instead of growing, routing or looking.
#[derive(Default)]
struct Plan {
    rects: BTreeMap<ObjectId, [f64; 4]>,
    effects: BTreeMap<ObjectId, ([f64; 4], [f64; 4])>,
    /// A source's effect through the request's passages: surely through a
    /// certain one, at most through any.
    through: BTreeMap<ObjectId, ([f64; 4], [f64; 4])>,
    asked: Mutex<Vec<CoverageRequest>>,
}

impl Plan {
    fn with(mut self, local: &str, rect: [f64; 4]) -> Self {
        self.rects.insert(id(local), rect);
        self
    }

    /// A source at `rect` whose effect covers `inner` surely and `outer`
    /// at most.
    fn source(mut self, local: &str, rect: [f64; 4], inner: [f64; 4], outer: [f64; 4]) -> Self {
        self.effects.insert(id(local), (inner, outer));
        self.with(local, rect)
    }

    /// A source whose effect reaches through a door: `inner` surely when
    /// the door surely is one, `outer` at most.
    fn through(mut self, local: &str, inner: [f64; 4], outer: [f64; 4]) -> Self {
        self.through.insert(id(local), (inner, outer));
        self
    }

    fn rect(&self, object: &ObjectId) -> Result<[f64; 4], PlanAreaError> {
        self.rects
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))
    }
}

fn exact_area(value: f64, locator: String) -> Result<PlanArea, PlanAreaError> {
    PlanArea::try_new(value, value, Evidence::exact(source(), locator))
}

/// The area of the union of `rects` within `plan`.
fn covered(plan: [f64; 4], rects: &[[f64; 4]]) -> f64 {
    let mut xs = vec![plan[0], plan[2]];
    let mut ys = vec![plan[1], plan[3]];
    for rect in rects {
        xs.extend([
            rect[0].clamp(plan[0], plan[2]),
            rect[2].clamp(plan[0], plan[2]),
        ]);
        ys.extend([
            rect[1].clamp(plan[1], plan[3]),
            rect[3].clamp(plan[1], plan[3]),
        ]);
    }
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let mut sum = 0.0;
    for x in xs.windows(2) {
        for y in ys.windows(2) {
            let (cx, cy) = (f64::midpoint(x[0], x[1]), f64::midpoint(y[0], y[1]));
            if rects
                .iter()
                .any(|r| r[0] <= cx && cx <= r[2] && r[1] <= cy && cy <= r[3])
            {
                sum += (x[1] - x[0]) * (y[1] - y[0]);
            }
        }
    }
    sum
}

impl PlanAreaService for Plan {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let [x0, y0, x1, y1] = self.rect(object)?;
        exact_area((x1 - x0) * (y1 - y0), format!("footprint:{object}"))
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        unreachable!("coverage is measured as a whole")
    }

    fn measure_coverage(
        &self,
        request: &CoverageRequest,
    ) -> Result<CoverageEvidence, PlanAreaError> {
        self.asked.lock().unwrap().push(request.clone());
        let plan = self.rect(request.subject())?;
        let (mut inner, mut outer) = (Vec::new(), Vec::new());
        let mut effects = Vec::new();
        let passing = request.passages().iter().any(Participant::is_certain);
        let passable = !request.passages().is_empty();
        for source in request.sources() {
            // An object with no effect of its own (a wall a selection
            // cannot rule out) reaches nothing.
            let nowhere = [-1e3, -1e3, -1e3, -1e3];
            let (mut sure, mut most) = self
                .effects
                .get(source.object())
                .copied()
                .unwrap_or((nowhere, nowhere));
            if let Some((inner, outer)) = self.through.get(source.object()) {
                if passing {
                    sure = *inner;
                }
                if passable {
                    most = *outer;
                }
            }
            let meets = if covered(plan, &[sure]) > 0.0 {
                EffectMeets::Surely
            } else if covered(plan, &[most]) > 0.0 {
                EffectMeets::Possibly
            } else {
                EffectMeets::No
            };
            if source.is_certain() {
                inner.push(sure);
            }
            outer.push(most);
            effects.push((source.object().clone(), meets));
        }
        let (lower, upper) = (covered(plan, &inner), covered(plan, &outer));
        let mut evidence = Evidence::exact(source(), format!("coverage:{}", request.subject()));
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        evidence.exact = exact;
        CoverageEvidence::try_new(
            request.subject().clone(),
            self.measure_footprint(request.subject())?,
            PlanArea::try_new(lower, upper, evidence)?,
            effects,
        )
    }
}

impl ProximityService for Plan {
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        let [x0, y0, x1, y1] = self.rect(object).map_err(|_| ProximityError::Unavailable)?;
        ObjectBounds::try_new(
            object.clone(),
            Bounds3::try_new([x0, y0, 0.0], [x1, y1, 3.0])?,
            GeometryFidelity::Exact,
        )
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        unreachable!("coverage measures plan distances only")
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        assert_eq!(request.projection(), ProximityProjection::Horizontal);
        let a = self
            .rect(request.subject())
            .map_err(|_| ProximityError::Unavailable)?;
        let b = self
            .rect(request.counterpart())
            .map_err(|_| ProximityError::Unavailable)?;
        let dx = (b[0] - a[2]).max(a[0] - b[2]).max(0.0);
        let dy = (b[1] - a[3]).max(a[1] - b[3]).max(0.0);
        let gap = dx.hypot(dy);
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            gap,
            gap,
            GeometryFidelity::Exact,
            Evidence::exact(source(), "distance"),
        )
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

/// Rooms must be 90 % covered by the effect areas of their devices.
fn coverage(mode: &str, extra: Vec<(&'static str, ParameterValue)>) -> CompiledRule {
    let mut parameters = vec![
        ("sources", selector(devices())),
        ("mode", string(mode)),
        ("range", metres(3.0)),
        ("minimum_ratio", number(0.9)),
    ];
    parameters.extend(extra);
    rule(ID, kind("room"), parameters)
}

/// Devices, or anything stating `Pset.Device`.
fn devices() -> Selector {
    stated_or(kind("device"), "Device")
}

fn stated_or(kind: Selector, name: &str) -> Selector {
    Selector::AnyOf {
        operands: vec![
            kind,
            Selector::Property {
                property_set: Some("Pset".into()),
                property: name.into(),
                operator: ComparisonOperator::Exists,
                value: None,
                case_sensitive: true,
                trim: false,
                quantifier: None,
                precision: None,
            },
        ],
    }
}

/// Room `r` (x 0 to 10, y 0 to 4). Device `a` covers its west half, `b`
/// its east half, `c` none of it, and wall `w` stands in it.
fn plan() -> Plan {
    Plan::default()
        .with("r", [0.0, 0.0, 10.0, 4.0])
        .with("w", [5.0, 0.0, 5.2, 3.5])
        .source(
            "a",
            [2.4, 1.9, 2.6, 2.1],
            [0.0, 0.0, 5.0, 4.0],
            [0.0, 0.0, 5.0, 4.0],
        )
        .source(
            "b",
            [7.4, 1.9, 7.6, 2.1],
            [5.0, 0.0, 10.0, 4.0],
            [5.0, 0.0, 10.0, 4.0],
        )
        .source(
            "c",
            [11.0, 1.9, 11.2, 2.1],
            [11.0, 0.0, 12.0, 4.0],
            [11.0, 0.0, 12.0, 4.0],
        )
}

fn model(plan: &Plan) -> Model {
    plan.rects.keys().fold(Model::default(), |model, object| {
        let kind = match object.local_id.as_str() {
            "r" => "room",
            "w" | "x" => "wall",
            "p" => "pillar",
            _ => "device",
        };
        model.object(&object.local_id, kind)
    })
}

/// The template's evaluation, held to the replaced implementation's whole
/// contract; the template asks the plan nothing the capability did not.
#[allow(clippy::needless_pass_by_value)]
fn run_with(model: Model, plan: Arc<Plan>, rule: &CompiledRule) -> CapabilityEvaluation {
    let register = |services: &mut axioval_engine::ServiceRegistry| {
        services
            .register(PlanAreaServiceHandle::new(plan.clone()))
            .unwrap();
        services
            .register(ProximityServiceHandle::new(plan.clone()))
            .unwrap();
    };
    let distinct = |from: usize| -> std::collections::BTreeSet<String> {
        plan.asked.lock().unwrap()[from..]
            .iter()
            .map(|request| format!("{request:?}"))
            .collect()
    };
    let before = plan.asked.lock().unwrap().len();
    model.clone().evaluate_with(&Reference, rule, register);
    let asked = distinct(before);
    let both = plan.asked.lock().unwrap().len();
    let evaluation =
        model.holding_contract(&EffectiveCoverage, &Reference, rule, register, &[], 0.0);
    assert_eq!(
        distinct(both),
        asked,
        "the template asks what the capability asked"
    );
    // What the tests read first is what the template asked.
    let mut recorded = plan.asked.lock().unwrap();
    let tail = recorded.split_off(both);
    recorded.truncate(before);
    recorded.extend(tail);
    drop(recorded);
    evaluation
}

fn run(plan: Plan, rule: &CompiledRule) -> CapabilityEvaluation {
    let model = model(&plan);
    run_with(model, Arc::new(plan), rule)
}

fn without(mut plan: Plan, local: &str) -> Plan {
    plan.rects.remove(&id(local));
    plan.effects.remove(&id(local));
    plan
}

#[test]
fn grown_effects_covering_half_a_room_fail_and_both_halves_pass() {
    let evaluation = run(plan(), &coverage("grown", vec![]));
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    let evaluation = run(without(plan(), "b"), &coverage("grown", vec![]));
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            "0.5 of the footprint (20 of 40 m²) lies within the sources' effect areas (grown by \
             3 m); required at least 0.9"
                .to_owned()
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("a")]);
}

/// `e` stands against the room's east wall, `f` half a metre off it.
fn plan_with_neighbours() -> Plan {
    plan()
        .source(
            "e",
            [10.0, 0.0, 10.2, 1.0],
            [7.0, 0.0, 10.0, 4.0],
            [7.0, 0.0, 10.0, 4.0],
        )
        .source(
            "f",
            [10.5, 0.0, 10.7, 1.0],
            [7.0, 0.0, 10.0, 4.0],
            [7.0, 0.0, 10.0, 4.0],
        )
}

#[test]
fn touching_counts_only_sources_whose_footprint_meets_the_room() {
    let plan = Arc::new(plan_with_neighbours());
    let evaluation = run_with(model(&plan), plan.clone(), &coverage("touching", vec![]));
    let asked = plan.asked.lock().unwrap();
    let sent: Vec<&ObjectId> = asked[0].sources().iter().map(Participant::object).collect();
    assert_eq!(sent, [&id("a"), &id("b"), &id("e")]);
    assert_eq!(asked[0].reach(), EffectReach::Grown);
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");

    // With a 0.6 m tolerance, `f` touches too; the two cover 30 %.
    let plan = Arc::new(without(without(plan_with_neighbours(), "a"), "b"));
    let tolerant = coverage("touching", vec![("touch_tolerance", metres(0.6))]);
    let evaluation = run_with(model(&plan), plan.clone(), &tolerant);
    assert_eq!(plan.asked.lock().unwrap()[0].sources().len(), 2);
    assert_eq!(common::flagged(&evaluation), ["r"]);
}

#[test]
fn travel_and_sight_send_the_blockers_near_the_room_with_their_certainty() {
    let plan = Arc::new(
        plan()
            .with("p", [3.0, 3.0, 3.2, 3.2])
            .with("x", [30.0, 0.0, 31.0, 1.0]),
    );
    // `p` might be a wall; `x` is one, far away.
    let model = model(&plan).unreadable("p");
    let blockers = ("blockers", selector(stated_or(kind("wall"), "Wall")));
    let devices = ("sources", selector(kind("device")));
    let evaluation = run_with(
        model,
        plan.clone(),
        &coverage("travel", vec![blockers, devices]),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    let asked = plan.asked.lock().unwrap();
    assert_eq!(asked[0].reach(), EffectReach::Travel);
    let sent: Vec<(&ObjectId, bool)> = asked[0]
        .blockers()
        .iter()
        .map(|blocker| (blocker.object(), blocker.is_certain()))
        .collect();
    assert_eq!(sent, [(&id("p"), false), (&id("w"), true)]);
}

#[test]
fn a_share_straddling_the_minimum_is_not_evaluated() {
    // `a` surely covers the west half and at most the whole room.
    let plan = without(plan(), "b").source(
        "a",
        [2.4, 1.9, 2.6, 2.1],
        [0.0, 0.0, 5.0, 4.0],
        [0.0, 0.0, 10.0, 4.0],
    );
    let evaluation = run(plan, &coverage("visible", vec![]));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .starts_with("between 0.5 and 1 of the footprint"),
        "{evaluation:#?}"
    );
}

#[test]
fn an_undecided_source_counts_only_towards_the_upper_bound() {
    // Fixture `b` might be a device: without it the room fails, with it it
    // passes.
    let model = Model::default()
        .object("r", "room")
        .object("w", "wall")
        .object("a", "device")
        .object("b", "fixture")
        .object("c", "device")
        .unreadable("b");
    let evaluation = run_with(model, Arc::new(plan()), &coverage("grown", vec![]));
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)],
        "{evaluation:#?}"
    );
}

#[test]
fn capacity_compares_the_summed_property_times_the_multiplier_with_the_area() {
    let capacity = |multiplier: f64| {
        coverage(
            "grown",
            vec![
                ("capacity_property", property(Some("Pset"), "Units")),
                ("capacity_multiplier", number(multiplier)),
            ],
        )
    };
    let with_units = |model: Model| {
        model
            .value("a", "Pset", "Units", PropertyValue::Integer(2))
            .value("b", "Pset", "Units", PropertyValue::Decimal(1.0))
    };
    // 3 units of 15 m² serve 45 m² of a 40 m² room.
    let plan = Arc::new(plan());
    let evaluation = run_with(with_units(model(&plan)), plan.clone(), &capacity(15.0));
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{evaluation:#?}"
    );
    // 3 units of 10 m² serve only 30 m².
    let evaluation = run_with(with_units(model(&plan)), plan.clone(), &capacity(10.0));
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            "capacity: Pset.Units summed over the sources reaching it, times 10, is 30 m² for a \
             footprint of 40 m²"
                .to_owned()
        )]
    );
    // A source stating no units is a missing value of its own, and leaves
    // the capacity undecided.
    let evaluation = run_with(
        model(&plan).value("a", "Pset", "Units", PropertyValue::Integer(2)),
        plan.clone(),
        &capacity(15.0),
    );
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            format!("missing value: {}'s Pset.Units is not stated", id("b"))
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("b")]);
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // A value of another kind is no missing value, only undecided.
    let evaluation = run_with(
        with_units(model(&plan)).text("b", "Pset", "Units", "many"),
        plan,
        &capacity(15.0),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn capacity_multiplies_each_source_by_its_own_multiplier() {
    let capacity = coverage(
        "grown",
        vec![
            ("capacity_property", property(Some("Pset"), "Units")),
            (
                "capacity_multiplier_property",
                property(Some("Pset"), "Serves"),
            ),
        ],
    );
    let stated = |model: Model, serves: f64| {
        model
            .value("a", "Pset", "Units", PropertyValue::Integer(2))
            .value("a", "Pset", "Serves", PropertyValue::Decimal(10.0))
            .value("b", "Pset", "Units", PropertyValue::Integer(1))
            .value("b", "Pset", "Serves", PropertyValue::Decimal(serves))
    };
    // 2 × 10 + 1 × 25 = 45 m² for 40.
    let plan = Arc::new(plan());
    let evaluation = run_with(stated(model(&plan), 25.0), plan.clone(), &capacity);
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{evaluation:#?}"
    );
    // 2 × 10 + 1 × 10 = 30 m².
    let evaluation = run_with(stated(model(&plan), 10.0), plan.clone(), &capacity);
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            "capacity: Pset.Units times Pset.Serves summed over the sources reaching it is 30 m² \
             for a footprint of 40 m²"
                .to_owned()
        )]
    );
    // A multiplier not stated is a missing value too.
    let evaluation = run_with(
        model(&plan)
            .value("a", "Pset", "Units", PropertyValue::Integer(2))
            .value("a", "Pset", "Serves", PropertyValue::Decimal(10.0))
            .value("b", "Pset", "Units", PropertyValue::Integer(1)),
        plan,
        &capacity,
    );
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            format!("missing value: {}'s Pset.Serves is not stated", id("b"))
        )]
    );
}

#[test]
fn the_area_may_be_read_from_a_property() {
    let stated = coverage(
        "grown",
        vec![("area_property", property(Some("Pset"), "Area"))],
    );
    // `a` covers 20 m² of the footprint; the room states 20 m², all covered.
    let plan = Arc::new(without(plan(), "b"));
    let area = |value: f64| PropertyValue::Quantity {
        value,
        dimension: axioval_ir::QuantityDimension::Area,
    };
    let evaluation = run_with(
        model(&plan).value("r", "Pset", "Area", area(20.0)),
        plan.clone(),
        &stated,
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{evaluation:#?}"
    );
    // A stated 40 m² fails, and the message names it.
    let evaluation = run_with(
        model(&plan).value("r", "Pset", "Area", PropertyValue::Decimal(40.0)),
        plan.clone(),
        &stated,
    );
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            "0.5 of the stated area (Pset.Area) (20 of 40 m²) lies within the sources' effect \
             areas (grown by 3 m); required at least 0.9"
                .to_owned()
        )]
    );
    // Not stated: a missing value, and nothing else checked.
    let evaluation = run_with(model(&plan), plan.clone(), &stated);
    assert_eq!(
        common::findings(&evaluation),
        [(
            "r".to_owned(),
            "missing value: its Pset.Area is not stated".to_owned()
        )]
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    // A length is no area.
    let evaluation = run_with(
        model(&plan).value(
            "r",
            "Pset",
            "Area",
            PropertyValue::Quantity {
                value: 20.0,
                dimension: axioval_ir::QuantityDimension::Length,
            },
        ),
        plan,
        &stated,
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// Room `r` and, east of it behind a 0.2 m wall, space `n` holding
/// sprinkler `s`; door `d` joins them. Through the door, `s` covers the
/// whole room.
fn next_door() -> (Plan, Model) {
    let plan = Plan::default()
        .with("r", [0.0, 0.0, 10.0, 4.0])
        .with("n", [10.2, 0.0, 20.0, 4.0])
        .with("d", [10.0, 1.5, 10.2, 2.5])
        .with("x", [8.0, 0.0, 8.2, 1.0])
        .source(
            "s",
            [10.9, 1.9, 11.1, 2.1],
            [11.0, 0.0, 12.0, 4.0],
            [11.0, 0.0, 12.0, 4.0],
        )
        .through("s", [0.0, 0.0, 12.0, 4.0], [0.0, 0.0, 12.0, 4.0]);
    let model = Model::default()
        .object("r", "room")
        .object("n", "space")
        .object("d", "door")
        .object("x", "wall")
        .object("s", "device")
        .edge("bounds", "d", "r")
        .edge("bounds", "d", "n");
    (plan, model)
}

fn propagating(mode: &str) -> CompiledRule {
    coverage(
        mode,
        vec![
            ("access_path", strings(&["bounds:forward"])),
            ("door_selector", selector(kind("door"))),
            ("blockers", selector(kind("wall"))),
        ],
    )
}

#[test]
fn an_effect_propagates_through_a_door_into_the_next_room() {
    // On its own the room holds no sprinkler.
    let (plan, model) = next_door();
    let evaluation = run_with(model, Arc::new(plan), &coverage("travel", vec![]));
    assert_eq!(common::flagged(&evaluation), ["r"]);

    // Through the door the sprinkler next door covers it.
    let (plan, model) = next_door();
    let plan = Arc::new(plan);
    let evaluation = run_with(model, plan.clone(), &propagating("travel"));
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{evaluation:#?}"
    );
    let asked = plan.asked.lock().unwrap();
    assert_eq!(asked[0].connected(), [Participant::new(id("n"), true)]);
    assert_eq!(asked[0].passages(), [Participant::new(id("d"), true)]);
    // With connections, a blocker within range of the room is sent too.
    assert_eq!(asked[0].blockers(), [Participant::new(id("x"), true)]);
    drop(asked);

    // A door that might not be one widens only the upper bound.
    let (plan, model) = next_door();
    let evaluation = run_with(model.unreadable("d"), Arc::new(plan), &{
        coverage(
            "visible",
            vec![
                ("access_path", strings(&["bounds:forward"])),
                ("door_selector", selector(stated_or(kind("hatch"), "Door"))),
                ("sources", selector(kind("device"))),
            ],
        )
    });
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_door_whose_spaces_cannot_be_read_leaves_the_upper_bound_open() {
    // `b` covers the east half surely; door `e`, whose adjacency records
    // no face, might let more in.
    let plan = without(plan(), "a");
    let model = model(&plan)
        .object("e", "door")
        .edge("axioval:derived.adjacent-space", "e", "r");
    let evaluation = run_with(
        model,
        Arc::new(plan),
        &coverage(
            "travel",
            vec![
                ("access_path", strings(&["axioval:derived.adjacent-space"])),
                ("door_selector", selector(kind("door"))),
            ],
        ),
    );
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert_eq!(
        unevaluated(&evaluation),
        [("r".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("a door or opening may join more spaces"),
        "{evaluation:#?}"
    );
}

#[test]
fn a_bad_declaration_is_not_evaluated() {
    for extra in [
        vec![("blockers", selector(kind("wall")))],
        vec![("touch_tolerance", metres(0.1))],
        vec![("capacity_multiplier", number(2.0))],
        vec![("minimum_ratio", number(1.5))],
        vec![("mode", string("sideways"))],
        vec![("access_path", strings(&["bounds:forward"]))],
        vec![
            ("access_path", strings(&["bounds:forward"])),
            ("door_selector", selector(kind("door"))),
        ],
        vec![
            ("capacity_property", property(Some("Pset"), "Units")),
            ("capacity_multiplier", number(2.0)),
            (
                "capacity_multiplier_property",
                property(Some("Pset"), "Serves"),
            ),
        ],
        vec![(
            "capacity_multiplier_property",
            property(Some("Pset"), "Serves"),
        )],
    ] {
        let evaluation = run(plan(), &coverage("grown", extra));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// The measured covered share against the minimum reaches the verdicts:
/// covered, half covered and straddling.
#[test]
#[allow(clippy::type_complexity)]
fn the_measured_covered_share_reaches_the_verdicts() {
    let straddling = || {
        without(plan(), "b").source(
            "a",
            [2.4, 1.9, 2.6, 2.1],
            [0.0, 0.0, 5.0, 4.0],
            [0.0, 0.0, 10.0, 4.0],
        )
    };
    let cases: [(&str, fn() -> Plan, &str); 3] = [
        ("covered", plan, "grown"),
        ("half", || without(plan(), "b"), "grown"),
        ("straddling", straddling, "visible"),
    ];
    for (case, fixture, mode) in cases {
        let evaluation = run(fixture(), &coverage(mode, vec![]));
        let verdict = if !evaluation.findings().is_empty() {
            Some(false)
        } else if unevaluated(&evaluation).is_empty() {
            Some(true)
        } else {
            None
        };
        let (project, mut services) = model(&fixture()).services();
        services
            .register(PlanAreaServiceHandle::new(Arc::new(fixture())))
            .unwrap();
        let share = common::measured(
            &services,
            &project,
            &id("r"),
            &format!("effect_covered_share;sources=device;reach={mode};range=3"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(common::at_least(share, 0.9), verdict, "{case}: {share:?}");
    }
}

/// The share check as an expression rule over `effect_covered_share`, or
/// over `effect_covered_area` against a stated area, held to the parity
/// harness on the fixtures whose sources, blockers and reach a measured
/// value names by kind.
mod as_expressions {
    use axioval_rules::parity::ParityEvidence;
    use serde_json::{Value, json};

    use super::*;

    fn at_least_share(mode: &str, blockers: &str) -> Value {
        json!({"kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "property", "propertySet": "axioval:measured",
                "property": format!("effect_covered_share;sources=device;reach={mode};range=3{blockers}")},
            "right": {"kind": "literal", "value": {"type": "number", "value": 0.9}}})
    }

    fn parity(
        plan: &dyn Fn() -> Plan,
        model: &dyn Fn() -> Model,
        declared: &CompiledRule,
        requirement: Value,
    ) -> ParityEvidence {
        let expected = run_with(model(), Arc::new(plan()), declared);
        let rule = rule(
            "axioval:capability.expression",
            kind("room"),
            vec![("requirement", common::expression(requirement))],
        );
        let rewritten =
            model().evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
                let shared = Arc::new(plan());
                services
                    .register(PlanAreaServiceHandle::new(shared.clone()))
                    .unwrap();
                services
                    .register(ProximityServiceHandle::new(shared))
                    .unwrap();
            });
        axioval_rules::parity::compare_evaluations((ID, &expected), ("expression", &rewritten))
    }

    fn of(plan: &dyn Fn() -> Plan, declared: &CompiledRule, requirement: Value) -> ParityEvidence {
        parity(plan, &|| model(&plan()), declared, requirement)
    }

    fn holds(parity: &ParityEvidence) {
        assert!(parity.holds(), "{}", parity.diff());
    }

    #[test]
    fn covered_shares_reach_the_verdicts_in_each_reach() {
        let evidence = of(
            &plan,
            &coverage("grown", vec![]),
            at_least_share("grown", ""),
        );
        holds(&evidence);
        assert_eq!(evidence.objects, 0);
        let half = || without(plan(), "b");
        let evidence = of(
            &half,
            &coverage("grown", vec![]),
            at_least_share("grown", ""),
        );
        holds(&evidence);
        assert_eq!(evidence.found, 1);
        let straddling = || {
            without(plan(), "b").source(
                "a",
                [2.4, 1.9, 2.6, 2.1],
                [0.0, 0.0, 5.0, 4.0],
                [0.0, 0.0, 10.0, 4.0],
            )
        };
        let evidence = of(
            &straddling,
            &coverage("visible", vec![]),
            at_least_share("visible", ""),
        );
        holds(&evidence);
        assert_eq!(evidence.open, 1);
        // Blockers in travel; `x` is a wall far away.
        let blocked = || plan().with("x", [30.0, 0.0, 31.0, 1.0]);
        let evidence = of(
            &blocked,
            &coverage(
                "travel",
                vec![
                    ("blockers", selector(kind("wall"))),
                    ("sources", selector(kind("device"))),
                ],
            ),
            at_least_share("travel", ";blockers=wall"),
        );
        holds(&evidence);
        assert_eq!(evidence.objects, 0);
        // Next door, without its connections, the room holds no source.
        let evidence = parity(
            &|| next_door().0,
            &|| next_door().1,
            &coverage("travel", vec![]),
            at_least_share("travel", ""),
        );
        holds(&evidence);
        assert_eq!(evidence.found, 1);
    }

    #[test]
    fn a_stated_area_reaches_the_verdicts() {
        let declared = coverage(
            "grown",
            vec![("area_property", property(Some("Pset"), "Area"))],
        );
        let stated = json!({"kind": "property", "propertySet": "Pset", "property": "Area"});
        let requirement = json!({"kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "property", "propertySet": "axioval:measured",
                "property": "effect_covered_area;sources=device;reach=grown;range=3"},
            "right": {"kind": "multiply", "left": stated,
                "right": {"kind": "literal", "value": {"type": "number", "value": 0.9}}}});
        let half = || without(plan(), "b");
        let area = |value: f64| PropertyValue::Quantity {
            value,
            dimension: axioval_ir::QuantityDimension::Area,
        };
        let cases: [(&dyn Fn() -> Model, usize, usize); 3] = [
            // All 20 m² the room states are covered.
            (
                &|| model(&half()).value("r", "Pset", "Area", area(20.0)),
                0,
                0,
            ),
            // Half of 40 m² stated.
            (
                &|| model(&half()).value("r", "Pset", "Area", area(40.0)),
                1,
                0,
            ),
            // Not stated: a missing value.
            (&|| model(&half()), 1, 0),
        ];
        for (model, found, open) in cases {
            let evidence = parity(&half, model, &declared, requirement.clone());
            holds(&evidence);
            assert_eq!((evidence.found, evidence.open), (found, open));
        }
    }
}

/// Generated rooms and devices, held to the replaced implementation's
/// whole contract: random rooms, devices of random effects (some of
/// undecided selection, some without geometry), walls, stated areas and
/// capacities (some missing, `null` or of another kind) under every mode,
/// minimum and capacity declaration.
mod generated {
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::*;

    /// A device: where it stands along x and y (decimetres), how far its
    /// effect surely and at most reaches (decimetres), whether its
    /// selection is undecided or it has no geometry, its rating and its
    /// multiplier (0 none, 1 `null`, 2 text, else a number).
    type Device = ((u32, u32), (u32, u32), u32, (u32, u32));

    fn device() -> impl Strategy<Value = Device> {
        (
            (0u32..120, 0u32..50),
            (0u32..30, 0u32..20),
            prop_oneof![6 => Just(0u32), 1 => Just(1u32), 1 => Just(2u32)],
            (0u32..8, 0u32..8),
        )
    }

    fn value(code: u32) -> Option<PropertyValue> {
        match code {
            0 => None,
            1 => Some(PropertyValue::Null),
            2 => Some(PropertyValue::String("many".into())),
            number => Some(PropertyValue::Decimal(f64::from(number))),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn generated_rooms_hold_parity(
            rooms in vec((0u32..3, prop_oneof![3 => Just(0u32), 1 => 1u32..4, 1 => Just(9u32)]), 1..4),
            devices in vec(device(), 0..6),
            walls in vec((0u32..120, 0u32..50), 0..3),
            mode in 0u32..4,
            minimum in 1u32..11,
            capacity in 0u32..3,
            stated_area in any::<bool>(),
            unreadable_wall in any::<bool>(),
        ) {
            let mut plan = Plan::default();
            let mut model = Model::default();
            for (index, (width, stated)) in rooms.iter().enumerate() {
                let local = format!("r{index}");
                #[allow(clippy::cast_precision_loss)]
                let x = index as f64 * 5.0;
                plan = plan.with(&local, [x, 0.0, x + 4.0 + f64::from(*width), 4.0]);
                model = model.object(&local, "room");
                if stated_area {
                    model = match stated {
                        0 => model,
                        9 => model.value(&local, "Pset", "Area", PropertyValue::Null),
                        area => model.value(
                            &local,
                            "Pset",
                            "Area",
                            PropertyValue::Quantity {
                                value: f64::from(*area) * 5.0,
                                dimension: axioval_ir::QuantityDimension::Area,
                            },
                        ),
                    };
                }
            }
            for (index, ((x, y), (sure, most), kind, (rating, factor))) in devices.iter().enumerate() {
                let local = format!("d{index}");
                let (x, y) = (f64::from(*x) / 10.0, f64::from(*y) / 10.0);
                let (sure, most) = (f64::from(*sure) / 10.0, f64::from(*sure + *most) / 10.0);
                let rect = [x, y, x + 0.2, y + 0.2];
                let grow = |by: f64| [x - by, y - by, x + 0.2 + by, y + 0.2 + by];
                if *kind == 2 {
                    // In the model, but without geometry.
                    model = model.object(&local, "device");
                } else {
                    plan = plan.source(&local, rect, grow(sure), grow(most));
                    model = model.object(&local, if *kind == 1 { "other" } else { "device" });
                    if *kind == 1 {
                        model = model.unreadable(&local);
                    }
                }
                if let Some(rating) = value(*rating) {
                    model = model.value(&local, "Pset", "Rating", rating);
                }
                if let Some(factor) = value(*factor) {
                    model = model.value(&local, "Pset", "Factor", factor);
                }
            }
            for (index, (x, y)) in walls.iter().enumerate() {
                let local = format!("x{index}");
                let (x, y) = (f64::from(*x) / 10.0, f64::from(*y) / 10.0);
                plan = plan.with(&local, [x, y, x + 0.2, y + 2.0]);
                model = model.object(&local, "wall");
                if unreadable_wall && index == 0 {
                    model = model.unreadable(&local);
                }
            }
            let mode = ["grown", "touching", "travel", "visible"][mode as usize];
            let mut extra = Vec::new();
            if mode == "touching" {
                extra.push(("touch_tolerance", metres(0.5)));
            }
            if matches!(mode, "travel" | "visible") {
                extra.push(("blockers", selector(stated_or(kind("wall"), "Wall"))));
            }
            if stated_area {
                extra.push(("area_property", property(Some("Pset"), "Area")));
            }
            match capacity {
                1 => extra.extend([
                    ("capacity_property", property(Some("Pset"), "Rating")),
                    ("capacity_multiplier", number(2.5)),
                ]),
                2 => extra.extend([
                    ("capacity_property", property(Some("Pset"), "Rating")),
                    ("capacity_multiplier_property", property(Some("Pset"), "Factor")),
                ]),
                _ => {}
            }
            let mut declared = coverage(mode, extra);
            declared
                .parameters
                .insert("minimum_ratio".into(), number(f64::from(minimum) / 10.0));
            run_with(model, Arc::new(plan), &declared);
        }
    }
}

/// The rule forked from the template, an `expression` rule requiring the
/// element measured (`effective_reaching` stated: an element stating no
/// area is a finding) and its share at least the minimum, carrying the
/// sources and the declaration, reaches the template's verdicts: the
/// objects found and those left open. A capacity check, judged against a
/// sum without an upper bound and finding each missing value apart, has no
/// expression form, so such a rule is never forked.
#[test]
fn the_forked_rule_reaches_the_templates_verdicts() {
    use axioval_rules::ExpressionRequirement;
    use axioval_rules::templates::{Fork, ForkError, fork};
    let verdicts = |evaluation: &CapabilityEvaluation| {
        let mut found: Vec<ObjectId> = evaluation
            .findings()
            .iter()
            .filter_map(|finding| match &finding.scope {
                axioval_ir::Scope::Object(object) => Some(object.clone()),
                _ => None,
            })
            .collect();
        found.sort();
        found.dedup();
        let mut open: Vec<ObjectId> = evaluation
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .filter(|object| !found.contains(object))
            .collect();
        open.sort();
        open.dedup();
        (found, open)
    };
    let stated = ("area_property", property(Some("Pset"), "Area"));
    let area = PropertyValue::Quantity {
        value: 20.0,
        dimension: axioval_ir::QuantityDimension::Area,
    };
    let cases: Vec<(Plan, Model, CompiledRule)> = vec![
        (plan(), model(&plan()), coverage("grown", vec![])),
        (
            without(plan(), "b"),
            model(&without(plan(), "b")),
            coverage("grown", vec![]),
        ),
        (
            plan(),
            model(&plan()).unreadable("a"),
            coverage("travel", vec![]),
        ),
        (
            without(plan(), "b"),
            model(&without(plan(), "b")).value("r", "Pset", "Area", area),
            coverage("grown", vec![stated.clone()]),
        ),
        (
            without(plan(), "b"),
            model(&without(plan(), "b")),
            coverage("grown", vec![stated.clone()]),
        ),
    ];
    for (plan, model, bound) in cases {
        let plan = Arc::new(plan);
        let forked = fork(&EffectiveCoverage, &bound).unwrap();
        let mut expression_rule = bound.clone();
        expression_rule.capability = Fork::CAPABILITY.into();
        expression_rule.parameters = forked.parameters();
        let register = |services: &mut axioval_engine::ServiceRegistry| {
            services
                .register(PlanAreaServiceHandle::new(plan.clone()))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(plan.clone()))
                .unwrap();
        };
        let template = model
            .clone()
            .evaluate_measured(&EffectiveCoverage, &bound, register);
        let forked = model.evaluate_measured(&ExpressionRequirement, &expression_rule, register);
        assert_eq!(
            verdicts(&template),
            verdicts(&forked),
            "{:?}",
            bound.parameters
        );
    }
    let capacity = coverage(
        "grown",
        vec![
            ("capacity_property", property(Some("Pset"), "Rating")),
            ("capacity_multiplier", number(2.0)),
        ],
    );
    assert!(matches!(
        fork(&EffectiveCoverage, &capacity),
        Err(ForkError::Inexpressible(_))
    ));
}

/// Covered from an effect measured between bounds, the share and the part
/// covered are never exact; measured exactly, they are. The area and the
/// count of sources reaching are exact either way.
#[test]
fn an_effect_measured_inexactly_is_inexact() {
    for (outer, exact) in [([0.0, 0.0, 6.0, 4.0], false), ([0.0, 0.0, 5.0, 4.0], true)] {
        let plan = Plan::default().with("r", [0.0, 0.0, 10.0, 4.0]).source(
            "a",
            [2.4, 1.9, 2.6, 2.1],
            [0.0, 0.0, 5.0, 4.0],
            outer,
        );
        let (project, mut services) = model(&plan).services();
        let shared = Arc::new(plan);
        services
            .register(PlanAreaServiceHandle::new(shared.clone()))
            .unwrap();
        services
            .register(ProximityServiceHandle::new(shared))
            .unwrap();
        let read = |name: &str| {
            common::measured_cited(
                &services,
                &project,
                &id("r"),
                &format!("{name};sources=device;range=3"),
            )
            .unwrap()
            .unwrap()
        };
        let ((lower, upper), cited) = read("effective_share");
        assert!(lower <= 0.5 && 0.5 <= upper, "{lower}..{upper}");
        assert_eq!(cited, exact, "outer {outer:?}");
        assert_eq!(read("effective_covered").1, exact);
        assert!(read("effective_area").1);
        assert_eq!(read("effective_reaching"), ((1.0, 1.0), true));
    }
}
