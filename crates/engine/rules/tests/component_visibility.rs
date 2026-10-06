//! Visibility of targets from an eye above each component.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axioval_engine::{
    CapabilityEvaluation, CentrePlacement, CompiledRule, ElevationInterval, PlanCentre, PlanLength,
    PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle, SightError, SightEvidence,
    SightOutcome, SightRequest, SightService, SightServiceHandle, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::ComponentVisibility;
use common::{Model, id, integer, kind, rule, selector, source, string, unevaluated};

const ID: &str = "axioval:capability.component-visibility";

/// What the stub answers about one target.
#[derive(Clone, Copy)]
enum Seen {
    /// In view, whatever blocks.
    Open,
    /// Hidden when this blocker is sent, in view otherwise.
    HiddenBy(&'static str),
    Undecided,
}

/// Desk `d` stands with its base at 0.5 m over the point (1, 2); every
/// target lies at a stated distance and is seen as stated.
#[derive(Default)]
struct Scene {
    targets: BTreeMap<ObjectId, (f64, Seen)>,
    asked: Mutex<Vec<SightRequest>>,
    /// Whether the line-of-sight service answers from a tessellation.
    approximate: bool,
}

impl Scene {
    fn with(mut self, local: &str, distance: f64, seen: Seen) -> Self {
        self.targets.insert(id(local), (distance, seen));
        self
    }
}

fn evidence(locator: String) -> Evidence {
    Evidence::exact(source(), locator)
}

impl SightService for Scene {
    fn assess_sight(&self, request: &SightRequest) -> Result<SightEvidence, SightError> {
        self.asked.lock().unwrap().push(request.clone());
        let target = request.target();
        let (distance, seen) = *self
            .targets
            .get(target)
            .ok_or_else(|| SightError::UnknownObject(target.clone()))?;
        let outcome = if request
            .within_metres()
            .is_some_and(|range| distance > range)
        {
            None
        } else {
            Some(match seen {
                Seen::HiddenBy(blocker) if request.blockers().contains(&id(blocker)) => {
                    SightOutcome::Hidden {
                        occluders: vec![id(blocker)],
                    }
                }
                Seen::Open | Seen::HiddenBy(_) => SightOutcome::Visible {
                    through: [0.0, 0.0, 0.0],
                },
                Seen::Undecided => SightOutcome::Undecided,
            })
        };
        SightEvidence::try_new(
            target.clone(),
            (distance, distance),
            outcome,
            Evidence {
                exact: !self.approximate,
                ..evidence(format!("sight:{target}"))
            },
        )
    }
}

impl PlanSpanService for Scene {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        unreachable!("visibility locates centres only")
    }

    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        unreachable!("visibility locates centres only")
    }

    fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        PlanCentre::try_new(
            object.clone(),
            [1.0, 2.0],
            0.0,
            CentrePlacement::Inside,
            evidence(format!("centre:{object}")),
        )
    }
}

impl VerticalExtentService for Scene {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::exact(0.5)?,
            ElevationInterval::exact(1.2)?,
            evidence(format!("extent:{object}")),
        )
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

/// Desks must see doors within 10 m, from 1.6 m above their base, past
/// walls and columns.
fn visibility(mode: &str, minimum: Option<i64>) -> CompiledRule {
    let mut parameters = vec![
        ("targets", selector(kind("door"))),
        ("blockers", selector(walls())),
        ("eye_height", metres(1.6)),
        ("radius", metres(10.0)),
        ("mode", string(mode)),
    ];
    if let Some(minimum) = minimum {
        parameters.push(("minimum", integer(minimum)));
    }
    rule(ID, kind("desk"), parameters)
}

/// Walls and columns, or anything stating `LoadBearing`.
fn walls() -> Selector {
    Selector::AnyOf {
        operands: vec![
            kind("wall"),
            Selector::Property {
                property_set: Some("Pset".into()),
                property: "LoadBearing".into(),
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

/// Desk `d`, wall `w`, column `c` and doors `t…`.
fn model(scene: &Scene) -> Model {
    let model = Model::default()
        .object("d", "desk")
        .object("w", "wall")
        .object("c", "wall");
    scene.targets.keys().fold(model, |model, target| {
        model.object(&target.local_id, "door")
    })
}

/// `component-visibility` as it runs, held to the implementation it
/// replaced on every evaluation.
static HELD: common::Held = common::Held(
    &ComponentVisibility,
    &axioval_rules::reference::ComponentVisibility,
);

fn run_with(model: Model, scene: Arc<Scene>, rule: &CompiledRule) -> CapabilityEvaluation {
    model.evaluate_measured(&HELD, rule, move |services| {
        services
            .register(SightServiceHandle::new(scene.clone()))
            .unwrap();
        services
            .register(PlanSpanServiceHandle::new(scene.clone()))
            .unwrap();
        services
            .register(VerticalExtentServiceHandle::new(scene.clone()))
            .unwrap();
    })
}

fn run(scene: Scene, rule: &CompiledRule) -> CapabilityEvaluation {
    let model = model(&scene);
    run_with(model, Arc::new(scene), rule)
}

#[test]
fn a_door_hidden_by_a_wall_fails_and_one_visible_past_a_column_passes() {
    // t1 stands behind the wall; t2 behind the column, which leaves it in
    // view.
    let scene = Scene::default()
        .with("t1", 6.0, Seen::HiddenBy("w"))
        .with("t2", 8.0, Seen::Open);
    let scene = Arc::new(scene);
    let evaluation = run_with(model(&scene), scene.clone(), &visibility("at-least", None));
    assert!(evaluation.findings().is_empty(), "{evaluation:#?}");
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    // The eye stands 1.6 m above the desk's base, over its centre, and
    // neither the desk nor the target blocks.
    let asked = scene.asked.lock().unwrap();
    assert!(asked.iter().all(|request| {
        let [x, y, z] = request.eye();
        (x - 1.0).abs() < 1e-12 && (y - 2.0).abs() < 1e-12 && (z - 2.1).abs() < 1e-12
    }));
    assert!(
        asked
            .iter()
            .all(|request| request.blockers() == [id("c"), id("w")])
    );

    let hidden_only = Scene::default().with("t1", 6.0, Seen::HiddenBy("w"));
    let evaluation = run(hidden_only, &visibility("at-least", None));
    assert_eq!(
        common::findings(&evaluation),
        [(
            "d".to_owned(),
            "0 target(s) within 10 m of the eye 1.6 m above the base of test:model/d are in \
             view; required at least 1; 1 hidden"
                .to_owned()
        )]
    );
    assert_eq!(evaluation.findings()[0].related, [id("t1")]);
}

#[test]
fn at_least_counts_targets_and_ignores_those_beyond_the_radius() {
    let scene = Scene::default()
        .with("t1", 6.0, Seen::Open)
        .with("t2", 8.0, Seen::Open)
        .with("t3", 12.0, Seen::Open);
    let evaluation = run(scene, &visibility("at-least", Some(3)));
    assert_eq!(common::flagged(&evaluation), ["d"]);
    assert!(
        evaluation.findings()[0]
            .message
            .starts_with("2 target(s) within 10 m"),
        "{evaluation:#?}"
    );
}

#[test]
fn none_finds_every_target_in_view() {
    let scene = Scene::default()
        .with("t1", 6.0, Seen::HiddenBy("w"))
        .with("t2", 8.0, Seen::Open);
    let evaluation = run(scene, &visibility("none", None));
    assert_eq!(
        common::findings(&evaluation),
        [(
            "d".to_owned(),
            "1 target(s) within 10 m of the eye 1.6 m above the base of test:model/d are in \
             view, none allowed: test:model/t2"
                .to_owned()
        )]
    );
    let hidden = Scene::default().with("t1", 6.0, Seen::HiddenBy("w"));
    let evaluation = run(hidden, &visibility("none", None));
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn an_undecided_view_decides_only_what_it_cannot_change() {
    let undecided = || Scene::default().with("t1", 6.0, Seen::Undecided);
    let evaluation = run(undecided(), &visibility("at-least", None));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("d".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "0 target(s) are in view, 1 required; 1 target(s) within 10 m of the eye 1.6 m above \
         the base of test:model/d are undecided: test:model/t1 can be proven neither in view \
         nor hidden (grazed, or covered only where blockers meet)"
    );
    // Another door in view settles it.
    let evaluation = run(
        undecided().with("t2", 8.0, Seen::Open),
        &visibility("at-least", None),
    );
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
    let evaluation = run(undecided(), &visibility("none", None));
    assert_eq!(
        unevaluated(&evaluation),
        [("d".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_blocker_whose_selection_is_undecided_cannot_prove_a_target_hidden() {
    // `p` might be load-bearing; only it hides t1.
    let scene = Scene::default().with("t1", 6.0, Seen::HiddenBy("p"));
    let model = model(&scene).object("p", "pillar").unreadable("p");
    let evaluation = run_with(model, Arc::new(scene), &visibility("at-least", None));
    assert!(evaluation.findings().is_empty());
    assert!(
        unevaluated(&evaluation)
            .contains(&("d".to_owned(), NotEvaluatedReason::IncompleteEvidence)),
        "{evaluation:#?}"
    );
}

#[test]
fn a_bad_declaration_or_a_missing_service_is_not_evaluated() {
    let refused = |mode: &str, minimum: Option<i64>| {
        let scene = Scene::default().with("t1", 6.0, Seen::Open);
        let evaluation = run(scene, &visibility(mode, minimum));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
        evaluation.not_evaluated_outcomes()[0].message().to_owned()
    };
    assert_eq!(
        refused("none", Some(2)),
        "component-visibility: minimum applies only to mode `at-least`"
    );
    assert_eq!(
        refused("at-least", Some(0)),
        "component-visibility: minimum must be a positive count"
    );
    assert_eq!(
        refused("all", None),
        "component-visibility: mode `all` is unsupported; use `at-least` or `none`"
    );
    let scene = Scene::default().with("t1", 6.0, Seen::Open);
    let evaluation = model(&scene).evaluate_measured(&HELD, &visibility("at-least", None), |_| {});
    assert_eq!(
        unevaluated(&evaluation),
        [("d".to_owned(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "line-of-sight service is not registered"
    );
}

/// Generated scenes: doors at random distances, in view, hidden by a wall
/// or by an undecided pillar, or undecided, judged under either mode and
/// many minimums; each held to the implementation the template replaced.
#[test]
fn generated_views_hold_parity() {
    let seen = [
        Seen::Open,
        Seen::HiddenBy("w"),
        Seen::HiddenBy("p"),
        Seen::Undecided,
    ];
    let mut judged = 0;
    for doors in 0..4_usize {
        for pattern in 0..16_usize {
            let mut scene = Scene::default();
            for door in 0..doors {
                let distance = 4.0 + 3.0 * f64::from(u32::try_from((pattern + door) % 4).unwrap());
                scene = scene.with(
                    &format!("t{door}"),
                    distance,
                    seen[(pattern / (door + 1)) % seen.len()],
                );
            }
            for (mode, minimum) in [
                ("at-least", None),
                ("at-least", Some(2)),
                ("at-least", Some(3)),
                ("none", None),
            ] {
                let scene = Arc::new(Scene {
                    targets: scene.targets.clone(),
                    asked: Mutex::new(Vec::new()),
                    approximate: doors % 2 == 1,
                });
                let model = model(&scene).object("p", "pillar").unreadable("p");
                // `run_with` holds the template to the reference.
                let evaluation = run_with(model, scene, &visibility(mode, minimum));
                judged += evaluation.findings().len() + evaluation.not_evaluated_outcomes().len();
            }
        }
    }
    assert!(judged > 0);
}

/// A view the line-of-sight service answers only approximately is cited as
/// inexact, as the capability cited it.
#[test]
fn a_view_measured_approximately_is_inexact() {
    let scene = Scene {
        approximate: true,
        ..Scene::default().with("t1", 6.0, Seen::Open)
    };
    let evaluation = run(scene, &visibility("none", None));
    assert_eq!(common::flagged(&evaluation), ["d"]);
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| !evidence.exact),
        "{evaluation:#?}"
    );
}
