//! A present property of an exactly declared type whose value the source
//! cannot read: presence and type are decided, the value is not.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityEvaluation, RuleCapability};
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_rules::{
    ManualIssue, PropertyDataType, PropertyExists, PropertyRequired, PropertyValueConstraint,
};
use common::{Model, boolean, flagged, property, rule, string, strings, unevaluated};

const SET: &str = "Pset";

/// `a` states a mass it cannot read, `b` has no such property.
fn model() -> Model {
    Model::default()
        .object("a", "wall")
        .object("b", "wall")
        .unreadable_value("a", SET, "Weight", "IFCMASSMEASURE")
}

fn check(
    capability: &dyn RuleCapability,
    id: &str,
    extra: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut parameters = vec![("property", property(Some(SET), "Weight"))];
    parameters.extend(extra);
    model().evaluate(capability, &rule(id, Selector::All, parameters))
}

fn open(evaluation: &CapabilityEvaluation) -> Vec<String> {
    unevaluated(evaluation)
        .into_iter()
        .map(|(object, _)| object)
        .collect()
}

#[test]
fn an_unreadable_value_is_present_and_not_empty() {
    for (capability, id) in [
        (
            &PropertyExists as &dyn RuleCapability,
            "axioval:capability.property-exists",
        ),
        (&PropertyRequired, "axioval:capability.property-required"),
    ] {
        let evaluation = check(capability, id, Vec::new());
        assert_eq!(flagged(&evaluation), ["b"], "{id}");
        assert!(open(&evaluation).is_empty(), "{id}");
    }
}

#[test]
fn its_declared_type_decides_a_data_type_requirement() {
    let data_type = |expected: &str| {
        check(
            &PropertyDataType,
            "axioval:capability.property-data-type",
            vec![("data_type", string(expected))],
        )
    };
    let other = data_type("IFCTIMEMEASURE");
    assert_eq!(flagged(&other), ["a", "b"]);
    assert!(open(&other).is_empty());
    let messages = common::findings(&other);
    assert!(
        messages[0]
            .1
            .contains("is IFCMASSMEASURE, not IFCTIMEMEASURE"),
        "{messages:?}"
    );
    let same = data_type("IfcMassMeasure");
    assert_eq!(flagged(&same), ["b"]);
    assert!(open(&same).is_empty());
}

#[test]
fn its_value_is_never_compared() {
    let value = |extra: Vec<(&str, ParameterValue)>| {
        check(
            &PropertyValueConstraint,
            "axioval:capability.property-value",
            extra,
        )
    };
    // Another declared type fails whatever the value.
    let other = value(vec![
        ("data_type", string("IFCTIMEMEASURE")),
        ("values", strings(&["2"])),
    ]);
    assert_eq!(flagged(&other), ["a", "b"]);
    assert!(open(&other).is_empty());
    // The same type, or none, leaves the comparison open.
    for extra in [
        vec![
            ("data_type", string("IFCMASSMEASURE")),
            ("values", strings(&["2"])),
        ],
        vec![("values", strings(&["2"]))],
        vec![("min_inclusive", string("0"))],
    ] {
        let evaluation = value(extra);
        assert_eq!(flagged(&evaluation), ["b"]);
        assert_eq!(open(&evaluation), ["a"]);
        assert!(
            evaluation
                .not_evaluated_outcomes()
                .iter()
                .all(|outcome| outcome.message().contains("the unit of Weight is unknown")),
            "{:?}",
            evaluation.not_evaluated_outcomes()
        );
    }
    // A type alone, as an optional IDS facet with a data type states it.
    let typed = value(vec![
        ("data_type", string("IFCMASSMEASURE")),
        ("optional", boolean(true)),
    ]);
    assert!(flagged(&typed).is_empty());
    assert!(open(&typed).is_empty());
}

#[test]
fn a_predicate_or_selector_on_it_is_not_evaluated() {
    let predicate = common::predicate(
        model(),
        &rule(
            "axioval:capability.property-predicate",
            Selector::All,
            vec![
                ("property_set", string(SET)),
                ("property", string("Weight")),
                ("operator", string("is_defined")),
            ],
        ),
    );
    assert_eq!(open(&predicate), ["a"]);

    let selected = model().evaluate(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            Selector::Property {
                property_set: Some(SET.into()),
                property: "Weight".into(),
                operator: ComparisonOperator::Exists,
                value: None,
                case_sensitive: true,
                trim: false,
                quantifier: None,
                precision: None,
            },
            vec![("title", string("selected"))],
        ),
    );
    assert!(selected.findings().is_empty());
    assert_eq!(open(&selected), ["a"]);
}
