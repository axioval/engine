//! A complex property is present and holds no value of any type: presence
//! is met, any type or value it must have fails, and no comparison passes.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityEvaluation, RuleCapability};
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_rules::{
    BooleanPropertyEquals, ManualIssue, PropertyComparison, PropertyDataType, PropertyExists,
    PropertyRequired, PropertyValueConstraint,
};
use common::{Model, boolean, findings, flagged, kind, number, property, rule, selector, string};

/// `property-value` runs as a template, held on every fixture to the
/// implementation it replaced.
const PROPERTY_VALUE: common::Held = common::Held(
    &PropertyValueConstraint,
    &axioval_rules::reference::PropertyValueConstraint,
);

const SET: &str = "Foo_Bar";

/// `a` holds the complex `Foo`, `b` no `Foo`.
fn model() -> Model {
    Model::default()
        .object("a", "wall")
        .object("b", "wall")
        .value("a", SET, "Foo", PropertyValue::Complex)
}

fn check(
    capability: &dyn RuleCapability,
    id: &str,
    extra: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut parameters = vec![("property", property(Some(SET), "Foo"))];
    parameters.extend(extra);
    model().evaluate(capability, &rule(id, Selector::All, parameters))
}

fn open(evaluation: &CapabilityEvaluation) -> usize {
    evaluation.not_evaluated_outcomes().len()
}

#[test]
fn a_complex_property_is_present_and_not_empty() {
    for (capability, id) in [
        (
            &PropertyExists as &dyn RuleCapability,
            "axioval:capability.property-exists",
        ),
        (&PropertyRequired, "axioval:capability.property-required"),
    ] {
        let evaluation = check(capability, id, Vec::new());
        assert_eq!(flagged(&evaluation), ["b"], "{id}");
        assert_eq!(open(&evaluation), 0, "{id}");
    }
}

#[test]
fn it_has_no_data_type() {
    let data_type = check(
        &PropertyDataType,
        "axioval:capability.property-data-type",
        vec![("data_type", string("IFCLENGTHMEASURE"))],
    );
    assert_eq!(flagged(&data_type), ["a", "b"]);
    assert_eq!(open(&data_type), 0);
    assert!(
        findings(&data_type)[0]
            .1
            .contains("is a complex property, not IFCLENGTHMEASURE"),
        "{:?}",
        findings(&data_type)
    );
    // An optional facet with a data type is failed by a complex as well.
    let optional = check(
        &PROPERTY_VALUE,
        "axioval:capability.property-value",
        vec![
            ("data_type", string("IFCLENGTHMEASURE")),
            ("optional", boolean(true)),
        ],
    );
    assert_eq!(flagged(&optional), ["a"]);
    assert_eq!(open(&optional), 0);
}

#[test]
fn it_meets_no_value_constraint() {
    for extra in [
        vec![("values", common::strings(&["42"]))],
        vec![("min_inclusive", string("0"))],
        vec![("patterns", common::strings(&[".*"]))],
    ] {
        let evaluation = check(&PROPERTY_VALUE, "axioval:capability.property-value", extra);
        assert_eq!(flagged(&evaluation), ["a", "b"]);
        assert_eq!(open(&evaluation), 0);
    }
    let equals = check(
        &BooleanPropertyEquals,
        "axioval:capability.property-value-equals",
        vec![("expected", boolean(true))],
    );
    assert_eq!(flagged(&equals), ["a", "b"]);
    assert_eq!(open(&equals), 0);
}

#[test]
fn no_predicate_passes() {
    for (operator, target) in [
        ("greater_than", ("number", number(0.0))),
        ("equal", ("number", number(42.0))),
        ("not_equal", ("number", number(42.0))),
        ("equal", ("text", string("Foo"))),
    ] {
        let predicate = common::predicate(
            model(),
            &rule(
                "axioval:capability.property-predicate",
                kind("wall"),
                vec![
                    ("property_set", string(SET)),
                    ("property", string("Foo")),
                    ("operator", string(operator)),
                    target,
                ],
            ),
        );
        assert!(
            flagged(&predicate).contains(&"a".to_owned()),
            "{operator}: {predicate:?}"
        );
        assert_eq!(open(&predicate), 0, "{operator}");
    }
}

#[test]
fn no_comparison_passes() {
    for (operator, target) in [
        ("greater", ("target_number", number(0.0))),
        ("equals", ("target_number", number(42.0))),
        ("not_equals", ("target_number", number(42.0))),
        ("equals", ("target_text", string("Foo"))),
    ] {
        let compared = Model::default()
            .object("r", "room")
            .object("a", "wall")
            .edge("contains", "r", "a")
            .value("a", SET, "Foo", PropertyValue::Complex)
            .evaluate(
                &PropertyComparison,
                &rule(
                    "axioval:capability.property-comparison",
                    kind("room"),
                    vec![
                        ("compared_selector", selector(kind("wall"))),
                        ("compared_property", property(Some(SET), "Foo")),
                        ("operator", string(operator)),
                        ("factor", number(1.0)),
                        ("component_mode", string("related")),
                        ("relationship", string("contains")),
                        ("quantifier", string("each")),
                        target,
                    ],
                ),
            );
        assert_eq!(flagged(&compared), ["r"], "{operator}: {compared:?}");
        assert_eq!(open(&compared), 0, "{operator}");
    }
}

#[test]
fn a_selector_comparing_its_value_leaves_it_undecided() {
    let selected = |operator, value: Option<ParameterValue>| {
        model().evaluate(
            &ManualIssue,
            &rule(
                "axioval:capability.manual-issue",
                Selector::Property {
                    property_set: Some(SET.into()),
                    property: "Foo".into(),
                    operator,
                    value,
                    case_sensitive: true,
                    trim: false,
                    quantifier: None,
                    precision: None,
                },
                vec![("title", string("selected"))],
            ),
        )
    };
    // Its presence selects it.
    let exists = selected(ComparisonOperator::Exists, None);
    assert_eq!(flagged(&exists), ["a"]);
    // Its value is never taken to match or to differ.
    let equals = selected(
        ComparisonOperator::Equals,
        Some(ParameterValue::Number { value: 42.0 }),
    );
    assert!(equals.findings().is_empty());
    assert_eq!(open(&equals), 1);
}
