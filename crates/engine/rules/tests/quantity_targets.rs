//! Declared quantities with units, compared with SI property values.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::{PropertyComparison, PropertyPredicate};
use common::{Model, findings, flagged, kind, number, property, selector, string, unevaluated};

fn length(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

fn quantity(value: f64, unit: &str) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: unit.into(),
    }
}

fn slab_depth(target: ParameterValue) -> axioval_engine::CapabilityEvaluation {
    let model = Model::default()
        .object("thin", "slab")
        .object("thick", "slab")
        .object("plain", "slab")
        .value("thin", "Pset", "Depth", length(0.18))
        .value("thick", "Pset", "Depth", length(0.3))
        .value("plain", "Pset", "Depth", PropertyValue::Decimal(0.3));
    model.evaluate(
        &PropertyPredicate,
        &common::rule(
            "axioval:capability.property-predicate",
            Selector::All,
            vec![
                ("property_set", string("Pset")),
                ("property", string("Depth")),
                ("operator", string("greater_or_equal")),
                ("quantity", target),
            ],
        ),
    )
}

#[test]
fn a_predicate_converts_its_quantity_to_si_before_comparing() {
    let evaluation = slab_depth(quantity(200.0, "mm"));
    assert_eq!(flagged(&evaluation), ["thin"]);
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("actual value is 0.18 m"),
        "{:?}",
        findings(&evaluation)
    );
    // A bare number cannot be read against a length.
    assert_eq!(
        unevaluated(&evaluation),
        [("plain".to_owned(), NotEvaluatedReason::InvalidEvidence)]
    );
}

#[test]
fn another_dimension_or_an_unknown_unit_is_refused() {
    let area = slab_depth(quantity(1.0, "m²"));
    assert!(area.findings().is_empty());
    assert_eq!(area.not_evaluated_outcomes().len(), 3);
    let unknown = slab_depth(quantity(1.0, "furlong"));
    assert_eq!(
        unevaluated(&unknown),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_comparison_sums_quantities_against_a_quantity_target() {
    let model = Model::default()
        .object("r1", "room")
        .object("r2", "room")
        .object("c1", "chair")
        .object("c2", "chair")
        .object("c3", "chair")
        .edge("contains", "r1", "c1")
        .edge("contains", "r1", "c2")
        .edge("contains", "r2", "c3")
        .value("c1", "Pset", "Depth", length(0.9))
        .value("c2", "Pset", "Depth", length(0.65))
        .value("c3", "Pset", "Depth", length(0.5));
    let evaluation = model.evaluate(
        &PropertyComparison,
        &common::rule(
            "axioval:capability.property-comparison",
            kind("room"),
            vec![
                ("compared_selector", selector(kind("chair"))),
                ("compared_property", property(Some("Pset"), "Depth")),
                ("target_quantity", quantity(1500.0, "mm")),
                ("operator", string("less_or_equal")),
                ("factor", number(1.0)),
                ("component_mode", string("related")),
                ("relationship", string("contains")),
                ("quantifier", string("sum")),
            ],
        ),
    );
    // r1 sums to 1.55 m, over 1.5 m; r2's 0.5 m is within it.
    assert_eq!(flagged(&evaluation), ["r1"]);
    assert!(
        findings(&evaluation)[0]
            .1
            .ends_with("is not less_or_equal 1.5 m"),
        "{:?}",
        findings(&evaluation)
    );
}
