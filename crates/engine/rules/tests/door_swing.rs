//! `door-swing`: which of its spaces a door swings into, from its leaves
//! and probes on both sides.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityEvaluation, DoorLeavesError, NotEvaluatedReason};
use axioval_ir::contract::ParameterValue;
use axioval_rules::DoorSwing;
use common::doors::{Doors, Rooms, hinged, sliding};
use common::{Model, findings, kind, rule, selector, strings, unevaluated};

const ID: &str = "axioval:capability.door-swing";

/// Office north of y = 0, corridor south of it; every door opens onto
/// both. `in` swings north into the office, `out` south into the corridor,
/// `both` both ways; `slide` slides; `far` stands where neither room is;
/// `unknown` states no leaves.
fn model() -> Model {
    let mut model = Model::default()
        .object("office", "office")
        .object("corridor", "corridor");
    for door in ["in", "out", "both", "slide", "far", "unknown"] {
        model = model
            .object(door, "door")
            .edge("opens", door, "office")
            .edge("opens", door, "corridor");
    }
    model
}

fn doors() -> Doors {
    let east = [1.0, 0.0, 0.0];
    Doors::default()
        .door(
            "in",
            vec![hinged([0.0; 3], east, [0.0, 1.0, 0.0], 0.9, false)],
            1.0,
            None,
        )
        .door(
            "out",
            vec![hinged([0.0; 3], east, [0.0, -1.0, 0.0], 0.9, false)],
            1.0,
            None,
        )
        .door(
            "both",
            vec![hinged([0.0; 3], east, [0.0, 1.0, 0.0], 0.9, true)],
            1.0,
            None,
        )
        .door("slide", vec![sliding([0.0; 3], east, 0.9)], 1.0, None)
        .door(
            "far",
            vec![hinged([20.0, 0.0, 0.0], east, [0.0, 1.0, 0.0], 0.9, false)],
            1.0,
            None,
        )
        .unknown("unknown", DoorLeavesError::NotStated("no panels".into()))
}

fn run(parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut all = vec![("space_path", strings(&["opens:forward"]))];
    all.extend(parameters);
    held(model(), &rule(ID, kind("door"), all), doors)
}

/// The template's evaluation of `rule` over `model` with the leaves
/// `doors` states and the two rooms, held to the implementation it
/// replaced under `Parity::contract()`.
fn held(
    model: Model,
    rule: &axioval_engine::CompiledRule,
    doors: impl Fn() -> Doors,
) -> CapabilityEvaluation {
    model.holding_contract(
        &DoorSwing,
        &axioval_rules::reference::DoorSwing,
        rule,
        |services| {
            services.register(doors().handle()).unwrap();
            services
                .register(
                    Rooms::default()
                        .room("office", [-5.0, 0.0], [5.0, 4.0])
                        .room("corridor", [-5.0, -2.0], [5.0, 0.0])
                        .handle(),
                )
                .unwrap();
        },
        &[],
        0.0,
    )
}

fn flagged(evaluation: &CapabilityEvaluation) -> Vec<String> {
    findings(evaluation)
        .into_iter()
        .map(|(door, _)| door)
        .collect()
}

fn open(evaluation: &CapabilityEvaluation) -> Vec<String> {
    unevaluated(evaluation)
        .into_iter()
        .map(|(door, _)| door)
        .collect()
}

#[test]
fn a_door_swinging_into_a_forbidden_space_is_found() {
    let evaluation = run(vec![("swing_not_into", selector(kind("corridor")))]);
    assert_eq!(flagged(&evaluation), ["both", "out"]);
    assert!(
        findings(&evaluation)
            .iter()
            .all(|(_, message)| message.ends_with("/corridor, which `swing_not_into` forbids")),
        "{:?}",
        findings(&evaluation)
    );
    // Neither probe of `far` lies in the corridor: it does not swing into
    // it. `slide` swings nowhere and `unknown` is unknown.
    assert_eq!(open(&evaluation), ["slide", "unknown"]);
    assert!(
        unevaluated(&evaluation)
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::IncompleteEvidence)
    );
}

#[test]
fn a_door_swinging_away_from_a_required_space_is_found() {
    let evaluation = run(vec![("swing_into", selector(kind("office")))]);
    assert_eq!(flagged(&evaluation), ["out"]);
    assert_eq!(
        findings(&evaluation)[0].1,
        "swings away from test:model/office, which `swing_into` requires it to swing into"
    );
    // `far` lies beside neither room at its probes, so it stays open.
    assert_eq!(open(&evaluation), ["far", "slide", "unknown"]);
}

#[test]
fn a_rule_states_a_direction() {
    let evaluation = run(vec![]);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

/// The measured leaves and swing read what the capability judges: how many
/// rooms each door swings into, its leaves and the area they sweep.
#[test]
fn the_measured_leaves_and_swing_match_the_judged_doors() {
    let (project, mut services) = model().services();
    services.register(doors().handle()).unwrap();
    services
        .register(
            Rooms::default()
                .room("office", [-5.0, 0.0], [5.0, 4.0])
                .room("corridor", [-5.0, -2.0], [5.0, 0.0])
                .handle(),
        )
        .unwrap();
    let read =
        |door: &str, name: &str| common::measured(&services, &project, &common::id(door), name);
    // `in` and `out` swing into one room each, `both` into both, `far` none.
    for (door, rooms) in [("in", 1.0), ("out", 1.0), ("both", 2.0), ("far", 0.0)] {
        assert_eq!(
            read(door, "swings_into;path=opens:forward").unwrap(),
            Some((rooms, rooms)),
            "{door}"
        );
    }
    assert!(read("slide", "swings_into;path=opens:forward").is_err());
    assert_eq!(read("in", "leaf_count").unwrap(), Some((1.0, 1.0)));
    assert_eq!(read("in", "leaf_width").unwrap(), Some((0.9, 0.9)));
    // A quarter circle of 0.9 m, bracketed by its polygons.
    let (lower, upper) = read("in", "swing_area").unwrap().unwrap();
    let quarter = std::f64::consts::FRAC_PI_4 * 0.81;
    assert!(
        lower <= quarter && quarter <= upper && upper - lower < 0.05,
        "{lower} {upper}"
    );
    assert!(read("unknown", "leaf_count").is_err());
}

/// Probes answered on approximate geometry prove nothing: the containment
/// contract refuses them, so the swung spaces are undecided and never
/// exact; probed exactly, they are exact.
#[test]
fn spaces_probed_approximately_are_never_exact() {
    use axioval_engine::{
        CapabilityRegistry, ClearanceOutcome, ClearanceRequest, ContainmentEvidence,
        ContainmentOutcome, ContainmentRequest, FreeAreaEvidence, FreeAreaRequest, FreeSpaceError,
        FreeSpaceService, FreeSpaceServiceHandle, MemberValue, PlacementOutcome, PlacementRequest,
        ServiceRegistry, measured_members,
    };
    /// Containment measured on a tessellation.
    struct Approximate;
    impl FreeSpaceService for Approximate {
        fn assess_clearance(
            &self,
            _: &ClearanceRequest,
        ) -> Result<ClearanceOutcome, FreeSpaceError> {
            Err(FreeSpaceError::Unavailable("containment only".into()))
        }
        fn find_placement(&self, _: &PlacementRequest) -> Result<PlacementOutcome, FreeSpaceError> {
            Err(FreeSpaceError::Unavailable("containment only".into()))
        }
        fn measure_free_area(
            &self,
            _: &FreeAreaRequest,
        ) -> Result<FreeAreaEvidence, FreeSpaceError> {
            Err(FreeSpaceError::Unavailable("containment only".into()))
        }
        fn assess_containment(
            &self,
            request: &ContainmentRequest,
        ) -> Result<ContainmentOutcome, FreeSpaceError> {
            let mut evidence = axioval_ir::Evidence::exact(common::source(), "mesh");
            evidence.exact = false;
            ContainmentEvidence::try_new(request.clone(), evidence).map(ContainmentOutcome::Inside)
        }
    }
    let (project, stated) = model().services();
    let registry = axioval_rules::register_builtins(CapabilityRegistry::new()).unwrap();
    let rooms = || {
        Rooms::default()
            .room("office", [-5.0, 0.0], [5.0, 4.0])
            .room("corridor", [-5.0, -2.0], [5.0, 0.0])
            .handle()
    };
    for (free, exact) in [
        (
            FreeSpaceServiceHandle::new(std::sync::Arc::new(Approximate)),
            false,
        ),
        (rooms(), true),
    ] {
        let mut services: ServiceRegistry = stated.clone();
        services.register(doors().handle()).unwrap();
        services.register(free).unwrap();
        registry.install_measured(&mut services, &project);
        let spaces = measured_members(
            &services,
            &common::id("in"),
            "swing_spaces;path=opens:forward",
        )
        .unwrap();
        assert_eq!(spaces.len(), 2);
        for space in &spaces {
            assert_eq!(space.exact, exact);
            assert_eq!(
                matches!(space.fields["into"], MemberValue::Undecided { .. }),
                !exact
            );
        }
    }
}

/// Each direction as an expression over the spaces a door opens onto
/// (`swing_spaces`): `swing_not_into` as no picked space swung into,
/// `swing_into` as not every picked space swung away from. Both judge every
/// door as `door-swing` does.
#[test]
fn the_directions_as_expressions_over_the_swung_spaces_reach_the_verdicts() {
    use axioval_rules::ExpressionRequirement;
    use serde_json::{Value, json};
    let spaces = |kinds: &str, function: &str, field: &str| {
        json!({"kind": "aggregate", "function": function,
            "over": {"kind": "measured",
                "name": format!("swing_spaces;path=opens:forward;kinds={kinds}")},
            "value": {"kind": "property", "propertySet": "axioval:member", "property": field}})
    };
    let services = |services: &mut axioval_engine::ServiceRegistry| {
        services.register(doors().handle()).unwrap();
        services
            .register(
                Rooms::default()
                    .room("office", [-5.0, 0.0], [5.0, 4.0])
                    .room("corridor", [-5.0, -2.0], [5.0, 0.0])
                    .handle(),
            )
            .unwrap();
    };
    let not_every_away =
        |kinds: &str| json!({"kind": "not", "operand": spaces(kinds, "all", "away")});
    let checks: [(&str, &str, Value); 4] = [
        (
            "swing_not_into",
            "corridor",
            spaces("corridor", "none", "into"),
        ),
        ("swing_not_into", "office", spaces("office", "none", "into")),
        ("swing_into", "office", not_every_away("office")),
        ("swing_into", "corridor", not_every_away("corridor")),
    ];
    for (direction, picked, requirement) in checks {
        let evaluated = run(vec![(direction, selector(kind(picked)))]);
        let rewritten = model().evaluate_measured(
            &ExpressionRequirement,
            &rule(
                "axioval:capability.expression",
                kind("door"),
                vec![("requirement", common::expression(requirement))],
            ),
            services,
        );
        let parity = axioval_rules::parity::compare_evaluations(
            (ID, &evaluated),
            ("expression", &rewritten),
        );
        assert!(parity.holds(), "{direction} {picked}:\n{}", parity.diff());
        assert!(parity.found > 0 && parity.open > 0, "{direction} {picked}");
    }
}

/// Spaces whose `Pset.Use` is `use`: a selection that cannot decide a
/// space whose use cannot be read.
fn used_as(use_: &str) -> ParameterValue {
    selector(
        serde_json::from_value(serde_json::json!({
            "kind": "property", "propertySet": "Pset", "property": "Use",
            "operator": "equals", "value": {"type": "string", "value": use_}}))
        .unwrap(),
    )
}

/// Every outcome worded as the capability worded it: probes that place
/// nothing, selections that cannot decide a space, both directions
/// together (the door left open once), and the refused declarations and
/// services.
#[test]
fn every_outcome_is_worded_as_before() {
    let model = || {
        model()
            .text("office", "Pset", "Use", "office")
            .unreadable_value("corridor", "Pset", "Use", "IFCLABEL")
    };
    let both = vec![
        ("space_path", strings(&["opens:forward"])),
        ("swing_into", used_as("office")),
        ("swing_not_into", used_as("corridor")),
    ];
    let evaluation = held(model(), &rule(ID, kind("door"), both), doors);
    let open: Vec<String> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .collect();
    // `out` and `both` swing into the corridor, which the selection may
    // pick; `slide` and `unknown` are open once.
    assert!(
        open.iter()
            .any(|message| message.ends_with("which `swing_not_into` may pick")),
        "{open:?}"
    );
    assert_eq!(
        evaluation
            .not_evaluated_outcomes()
            .iter()
            .filter(|outcome| outcome.object_id().unwrap().local_id == "slide")
            .count(),
        1
    );
    let refused = |parameters: Vec<(&'static str, ParameterValue)>| {
        let evaluation = held(model(), &rule(ID, kind("door"), parameters), doors);
        evaluation.not_evaluated_outcomes()[0].message().to_owned()
    };
    assert_eq!(
        refused(vec![("swing_into", used_as("office"))]),
        "door-swing: parameter `space_path` is required"
    );
    assert_eq!(
        refused(vec![
            ("space_path", strings(&["opens:forward"])),
            ("swing_into", strings(&["office"])),
        ]),
        "door-swing: parameter `swing_into` has the wrong type"
    );
    assert_eq!(
        refused(vec![("space_path", strings(&["opens:sideways"]))]),
        "door-swing: declare `swing_into`, `swing_not_into` or both"
    );
    let unserved = model().holding_contract(
        &DoorSwing,
        &axioval_rules::reference::DoorSwing,
        &rule(
            ID,
            kind("door"),
            vec![
                ("space_path", strings(&["opens:forward"])),
                ("swing_into", used_as("office")),
            ],
        ),
        |services| {
            services.register(doors().handle()).unwrap();
        },
        &[],
        0.0,
    );
    assert_eq!(
        unserved.not_evaluated_outcomes()[0].message(),
        "door-swing needs the object-frame and free-space services"
    );
}

/// Generated doors: each of six leaves (into the office, the corridor,
/// both ways, sliding, beside neither room, unknown) opening onto a random
/// set of the rooms, the rooms' uses stated, unreadable or absent, judged
/// against `swing_into` and `swing_not_into` by kind or by use, alike by
/// the template and the implementation it replaced.
mod generated {
    use super::*;
    use proptest::prelude::*;

    fn direction() -> impl Strategy<Value = Option<(bool, &'static str)>> {
        proptest::option::of((any::<bool>(), prop_oneof![Just("office"), Just("corridor")]))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn generated_doors_hold_parity(
            opens in proptest::collection::vec((any::<bool>(), any::<bool>()), 6),
            uses in [0u8..3, 0u8..3],
            into in direction(),
            not_into in direction(),
        ) {
            let mut model = Model::default()
                .object("office", "office")
                .object("corridor", "corridor");
            for (room, stated) in ["office", "corridor"].into_iter().zip(uses) {
                model = match stated {
                    0 => model.text(room, "Pset", "Use", room),
                    1 => model.unreadable_value(room, "Pset", "Use", "IFCLABEL"),
                    _ => model,
                };
            }
            for (door, (office, corridor)) in
                ["in", "out", "both", "slide", "far", "unknown"].into_iter().zip(opens)
            {
                model = model.object(door, "door");
                if office {
                    model = model.edge("opens", door, "office");
                }
                if corridor {
                    model = model.edge("opens", door, "corridor");
                }
            }
            let pick = |(by_use, room): (bool, &str)| {
                if by_use { used_as(room) } else { selector(kind(room)) }
            };
            let mut parameters = vec![("space_path", strings(&["opens:forward"]))];
            if let Some(into) = into {
                parameters.push(("swing_into", pick(into)));
            }
            if let Some(not_into) = not_into {
                parameters.push(("swing_not_into", pick(not_into)));
            }
            held(model, &rule(ID, kind("door"), parameters), doors);
        }
    }
}
