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

fn in_doc(document: &str, local: &str) -> axioval_ir::ObjectId {
    axioval_ir::ObjectId::new(axioval_ir::SourceId::new("test", document).unwrap(), local).unwrap()
}

fn metres(value: f64) -> axioval_ir::PropertyValue {
    axioval_ir::PropertyValue::Quantity {
        value,
        dimension: axioval_ir::QuantityDimension::Length,
    }
}

/// A door `d` of the MEP model on storey `l1`, hosted by wall `w` of the
/// architecture model on storey `og` at 3 m; `l1` stands at `elevation`.
fn federated(elevation: Option<f64>) -> Model {
    let model = Model::default()
        .object_in("mep", "l1", "storey")
        .object_in("mep", "d", "door")
        .object_in("arch", "og", "storey")
        .object_in("arch", "w", "wall")
        .edge_of("contains", in_doc("mep", "l1"), in_doc("mep", "d"))
        .edge_of("contains", in_doc("arch", "og"), in_doc("arch", "w"))
        .edge_of("hosts", in_doc("arch", "w"), in_doc("mep", "d"))
        .value_of(
            in_doc("arch", "og"),
            "axioval:attributes",
            "Elevation",
            metres(3.0),
        );
    match elevation {
        Some(elevation) => model.value_of(
            in_doc("mep", "l1"),
            "axioval:attributes",
            "Elevation",
            metres(elevation),
        ),
        None => model,
    }
}

fn run_levels(model: Model, levels: bool) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("counterpart_path", strings(&["hosts:backward"])),
        ("container_selector", selector(kind("storey"))),
        ("relationship", string("contains")),
        ("direction", string("backward")),
    ];
    if levels {
        parameters.push((
            "container_relationship",
            string("axioval:derived.same-level;tolerance=0.01"),
        ));
        parameters.push((
            "level_property",
            common::property(Some("axioval:attributes"), "Elevation"),
        ));
    }
    model.evaluate(&SameContainer, &rule(ID, kind("door"), parameters))
}

#[test]
fn storeys_of_several_models_at_one_elevation_are_one_level() {
    // Without the level match the MEP storey is not the architecture one.
    let evaluation = run_levels(federated(Some(3.0)), false);
    assert_eq!(evaluation.findings().len(), 1);
    // With it they are one level.
    let evaluation = run_levels(federated(Some(3.005)), true);
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(
        evaluation.not_evaluated_outcomes().is_empty(),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    // A storey half a metre off is another level.
    let evaluation = run_levels(federated(Some(3.5)), true);
    assert_eq!(evaluation.findings().len(), 1);
    // An unstated elevation leaves the door not evaluated.
    let evaluation = run_levels(federated(None), true);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("d".into(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_level_match_needs_its_property_and_a_valid_identity() {
    for extra in [
        vec![(
            "container_relationship",
            string("axioval:derived.same-level"),
        )],
        vec![(
            "level_property",
            common::property(Some("axioval:attributes"), "Elevation"),
        )],
        vec![
            (
                "container_relationship",
                string("axioval:derived.contained-in-space"),
            ),
            (
                "level_property",
                common::property(Some("axioval:attributes"), "Elevation"),
            ),
        ],
    ] {
        let evaluation = run(storeys(), extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
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
