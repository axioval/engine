//! The truth judge (`Decision::Holds`) held to what its documentation says
//! on a small template of its own: a room stating its area and an area
//! allowance must not exceed it, where it states either. A truth decides
//! what a stated absence means, the values after it only word a failure,
//! and a value stated `null` where a kind is expected is of the wrong kind.
#![allow(missing_docs)]

mod common;

use axioval_engine::template::{
    Check, Decision, Expect, Form, ParameterDefault, Template, TemplateValue,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::contract::{Expression, ParameterValue, ScalarValue};
use axioval_ir::{NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::templates::{Fork, Templated, fork};
use common::{Model, findings, kind, rule, unevaluated};
use serde_json::json;

const ID: &str = "test:room-allowance";

fn value(name: &'static str, expression: &serde_json::Value) -> TemplateValue {
    TemplateValue {
        name,
        expression: serde_json::from_value(expression.clone()).unwrap(),
        expect: None,
        absent: None,
        mismatch: None,
        refused: None,
    }
}

fn stated(name: &str) -> serde_json::Value {
    json!({"kind": "property", "propertySet": "Room", "property": name})
}

fn template() -> Template {
    let area = TemplateValue {
        expect: Some(Expect::Area),
        mismatch: Some("`Room.Area` is {area:stated}, not an area"),
        ..value("area", &stated("Area"))
    };
    let allowance = TemplateValue {
        expect: Some(Expect::Area),
        mismatch: Some("`Room.Allowance` is {allowance:stated}, not an area"),
        ..value("allowance", &stated("Allowance"))
    };
    let within = json!({"kind": "implies",
        "antecedent": {"kind": "or", "operands": [
            {"kind": "isDefined", "operand": stated("Area")},
            {"kind": "isDefined", "operand": stated("Allowance")}]},
        "consequent": {"kind": "compare", "operator": "lessThanOrEquals",
            "left": stated("Area"),
            "right": {"kind": "add", "left": stated("Allowance"),
                "right": {"kind": "parameter", "name": "slack"}}}});
    Template {
        id: ID,
        parameters: vec![ParameterDescriptor::optional(
            "slack",
            ParameterType::Quantity,
        )],
        grades: false,
        name: "room-allowance",
        refusals: axioval_engine::template::Refusals::Rule,
        defaults: vec![ParameterDefault {
            parameter: "slack",
            value: ScalarValue::Quantity {
                value: 0.0,
                unit: "m2".into(),
            },
            from: &[],
        }],
        declaration: vec![Check::NonNegativeQuantity {
            parameter: "slack",
            dimension: QuantityDimension::Area,
            message: "`slack` is not a non-negative area",
        }],
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![
                area,
                allowance,
                value("within", &within),
                value("label", &stated("Label")),
            ],
            decision: Decision::Holds { value: "within" },
            fail: "{label:stated} is {area:m2}, more than {allowance:m2} and {slack:m2}",
            undecided: "only one of the area and the allowance is stated",
            members: None,
            table: None,
            scope: None,
            derived: Vec::new(),
            related: None,
            checks: Vec::new(),
            unless: Vec::new(),
            grading: None,
            once: Vec::new(),
            project: Vec::new(),
            joined: None,
        }],
    }
}

fn area(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Area,
    }
}

fn rooms() -> Model {
    Model::default()
        // Within, and beyond: worded by the label read after the truth.
        .object("small", "room")
        .value("small", "Room", "Area", area(10.0))
        .value("small", "Room", "Allowance", area(12.0))
        .object("large", "room")
        .value("large", "Room", "Area", area(15.0))
        .value("large", "Room", "Allowance", area(12.0))
        .text("large", "Room", "Label", "Hall")
        // Neither stated passes; one stated is open.
        .object("bare", "room")
        .object("half", "room")
        .value("half", "Room", "Area", area(15.0))
        // Stated null where an area is expected; a length.
        .object("null", "room")
        .value("null", "Room", "Area", PropertyValue::Null)
        .object("long", "room")
        .value(
            "long",
            "Room",
            "Area",
            PropertyValue::Quantity {
                value: 3.0,
                dimension: QuantityDimension::Length,
            },
        )
        // Within, its unreadable label never read; beyond, the label
        // leaves the room open.
        .object("quiet", "room")
        .value("quiet", "Room", "Area", area(10.0))
        .value("quiet", "Room", "Allowance", area(12.0))
        .unreadable_value("quiet", "Room", "Label", "IFCLABEL")
        .object("loud", "room")
        .value("loud", "Room", "Area", area(15.0))
        .value("loud", "Room", "Allowance", area(12.0))
        .unreadable_value("loud", "Room", "Label", "IFCLABEL")
}

fn run(model: Model, slack: Option<f64>) -> axioval_engine::CapabilityEvaluation {
    let parameters = slack
        .map(|value| {
            (
                "slack",
                ParameterValue::Quantity {
                    value,
                    unit: "m2".into(),
                },
            )
        })
        .into_iter()
        .collect();
    model.evaluate(
        &Templated::new(template()),
        &rule(ID, kind("room"), parameters),
    )
}

#[test]
fn a_truth_passes_finds_or_leaves_open() {
    let evaluation = run(rooms(), None);
    assert_eq!(
        findings(&evaluation),
        [(
            "large".into(),
            "`Hall` is 15 m², more than 12 m² and 0 m²".into()
        )]
    );
    let open: Vec<(String, NotEvaluatedReason, String)> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| {
            (
                outcome.object_id().unwrap().local_id.clone(),
                outcome.reason().clone(),
                outcome.message().to_owned(),
            )
        })
        .collect();
    let open_on = |local: &str| {
        open.iter()
            .find(|(id, ..)| id == local)
            .map(|(_, reason, message)| (reason.clone(), message.as_str()))
    };
    assert_eq!(open.len(), 4, "{open:?}");
    assert_eq!(
        open_on("half"),
        Some((
            NotEvaluatedReason::IncompleteEvidence,
            "only one of the area and the allowance is stated"
        ))
    );
    assert_eq!(
        open_on("null"),
        Some((
            NotEvaluatedReason::InvalidEvidence,
            "`Room.Area` is null, not an area"
        ))
    );
    assert_eq!(
        open_on("long"),
        Some((
            NotEvaluatedReason::InvalidEvidence,
            "`Room.Area` is 3 m, not an area"
        ))
    );
    // The label is read after the truth only where it fails.
    assert!(open_on("loud").is_some());
    assert!(open_on("quiet").is_none());
    // A slack lets the large room pass.
    let evaluation = run(rooms(), Some(3.0));
    assert!(findings(&evaluation).is_empty());
    assert_eq!(unevaluated(&evaluation).len(), 3);
    // A slack of another kind is refused as the check words it.
    let evaluation = rooms().evaluate(
        &Templated::new(template()),
        &rule(
            ID,
            kind("room"),
            vec![(
                "slack",
                ParameterValue::Quantity {
                    value: 1.0,
                    unit: "m".into(),
                },
            )],
        ),
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "room-allowance: `slack` is not a non-negative area"
    );
}

/// A truth's expression form is the truth itself, so a rule forks into
/// it and reaches the template's verdicts.
#[test]
fn a_truth_forks_into_itself() {
    let template = Templated::new(template());
    let bound = rule(ID, kind("room"), Vec::new());
    let forked = fork(&template, &bound).unwrap();
    let mut expression_rule = bound.clone();
    expression_rule.capability = Fork::CAPABILITY.into();
    expression_rule.parameters = forked.parameters();
    let model = || {
        Model::default()
            .object("small", "room")
            .value("small", "Room", "Area", area(10.0))
            .value("small", "Room", "Allowance", area(12.0))
            .object("large", "room")
            .value("large", "Room", "Area", area(15.0))
            .value("large", "Room", "Allowance", area(12.0))
            .object("bare", "room")
    };
    let templated = model().evaluate(&template, &bound);
    let rewritten = model().evaluate(&axioval_rules::ExpressionRequirement, &expression_rule);
    let parity =
        axioval_rules::parity::compare_evaluations(("template", &templated), ("fork", &rewritten));
    assert!(parity.holds(), "{}", parity.diff());
    assert!(matches!(
        forked.parameters().get("requirement"),
        Some(ParameterValue::Expression { .. })
    ));
    let _: Option<&Expression> = None;
}
