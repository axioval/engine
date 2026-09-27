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
    model().evaluate_with(&DoorSwing, &rule(ID, kind("door"), all), |services| {
        services.register(doors().handle()).unwrap();
        services
            .register(
                Rooms::default()
                    .room("office", [-5.0, 0.0], [5.0, 4.0])
                    .room("corridor", [-5.0, -2.0], [5.0, 0.0])
                    .handle(),
            )
            .unwrap();
    })
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
