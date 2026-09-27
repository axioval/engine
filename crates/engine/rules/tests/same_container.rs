//! `same-container`: a door or window stands on its host wall's storey.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_rules::SameContainer;
use common::{Model, findings, id, kind, rule, selector, string, strings, unevaluated};

const ID: &str = "axioval:capability.same-container";

/// Storeys `eg` and `og`. Wall `w1` stands on `eg`, `w2` on `og`. Door `ok`
/// stands on `eg` in `w1`; door `up` on `og` but fills an opening in `w1`;
/// window `loose` stands on no storey in `w2`; door `free` fills nothing.
fn storeys() -> Model {
    Model::default()
        .object("eg", "storey")
        .object("og", "storey")
        .object("w1", "wall")
        .object("w2", "wall")
        .object("o1", "opening")
        .object("o2", "opening")
        .object("o3", "opening")
        .object("ok", "door")
        .object("up", "door")
        .object("loose", "door")
        .object("free", "door")
        .edge("contains", "eg", "w1")
        .edge("contains", "og", "w2")
        .edge("contains", "eg", "ok")
        .edge("contains", "og", "up")
        .edge("contains", "eg", "free")
        .edge("voids", "w1", "o1")
        .edge("voids", "w1", "o2")
        .edge("voids", "w2", "o3")
        .edge("fills", "o1", "ok")
        .edge("fills", "o2", "up")
        .edge("fills", "o3", "loose")
}

fn parameters(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        (
            "counterpart_path",
            strings(&["fills:backward", "voids:backward"]),
        ),
        ("container_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
        ("direction", string("backward")),
    ];
    parameters.extend(extra);
    parameters
}

fn run(model: Model, extra: Vec<(&'static str, ParameterValue)>) -> CapabilityEvaluation {
    model.evaluate(&SameContainer, &rule(ID, kind("door"), parameters(extra)))
}

#[test]
fn a_door_on_another_storey_than_its_host_wall_is_found() {
    let evaluation = run(storeys(), vec![]);
    assert_eq!(
        findings(&evaluation),
        [
            (
                "loose".into(),
                "in no container, but its counterpart via fills then voids is not: w2 is in og"
                    .into()
            ),
            (
                "up".into(),
                "in og, but its counterpart via fills then voids is not: w1 is in eg".into()
            ),
        ]
    );
    // The finding relates the host and both storeys.
    let related: Vec<&str> = evaluation.findings()[1]
        .related
        .iter()
        .map(|object| object.local_id.as_str())
        .collect();
    assert_eq!(related, ["eg", "og", "w1"]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_counterpart_the_selector_leaves_out_is_not_compared() {
    let evaluation = run(
        storeys(),
        vec![("counterpart_selector", selector(kind("column")))],
    );
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

fn load_bearing() -> Selector {
    Selector::Property {
        property_set: Some("Pset".into()),
        property: "LoadBearing".into(),
        operator: ComparisonOperator::Exists,
        value: None,
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    }
}

#[test]
fn an_undecided_counterpart_leaves_only_an_agreeing_door_open() {
    let model = storeys().unreadable("w1").unreadable("w2");
    let evaluation = run(
        model,
        vec![("counterpart_selector", selector(load_bearing()))],
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("loose".into(), NotEvaluatedReason::IncompleteEvidence),
            ("ok".into(), NotEvaluatedReason::IncompleteEvidence),
            ("up".into(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn an_undecided_container_leaves_every_door_open() {
    let model = storeys().unreadable("eg");
    let evaluation = model.evaluate(
        &SameContainer,
        &rule(
            ID,
            kind("door"),
            vec![
                (
                    "counterpart_path",
                    strings(&["fills:backward", "voids:backward"]),
                ),
                ("container_selector", selector(load_bearing())),
                ("relationship", string("contains")),
                ("direction", string("backward")),
            ],
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(unevaluated(&evaluation).len(), 4);
    assert!(
        evaluation
            .not_evaluated_outcomes()
            .iter()
            .all(|outcome| outcome.object_id() != Some(&id("eg")))
    );
}

#[test]
fn declarations_without_a_climb_or_path_are_refused() {
    for parameters in [
        vec![
            ("counterpart_path", strings(&["fills:backward"])),
            ("container_selector", selector(kind("storey"))),
        ],
        vec![
            ("container_selector", selector(kind("storey"))),
            ("relationship", string("contains")),
        ],
        vec![
            ("counterpart_path", strings(&[])),
            ("container_selector", selector(kind("storey"))),
            ("relationship", string("contains")),
        ],
        parameters(vec![(
            "follow_chain",
            ParameterValue::Boolean { value: true },
        )]),
    ] {
        let evaluation = storeys().evaluate(&SameContainer, &rule(ID, kind("door"), parameters));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
