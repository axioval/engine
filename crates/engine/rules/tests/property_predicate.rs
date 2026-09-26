//! Text, number, boolean and presence predicates over exact property values.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::PropertyValue;
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_rules::PropertyPredicate;
use common::{Model, boolean, findings, flagged, integer, number, string, strings, unevaluated};

const CAPABILITY: &str = "axioval:capability.property-predicate";

fn model() -> Model {
    Model::default()
        .object("a", "wall")
        .object("b", "wall")
        .object("c", "wall")
        .text("a", "Pset", "FireRating", "F90")
        .text("b", "Pset", "FireRating", "f30")
        .text("c", "Pset", "FireRating", "   ")
        .value("a", "Pset", "Width", PropertyValue::Decimal(0.24))
        .value("b", "Pset", "Width", PropertyValue::Integer(1))
        .value("a", "Pset", "LoadBearing", PropertyValue::Boolean(true))
}

fn check(
    name: &str,
    operator: &str,
    target: Vec<(&str, ParameterValue)>,
) -> axioval_engine::CapabilityEvaluation {
    let mut parameters = vec![
        ("property_set", string("Pset")),
        ("property", string(name)),
        ("operator", string(operator)),
    ];
    parameters.extend(target);
    model().evaluate(
        &PropertyPredicate,
        &common::rule(CAPABILITY, Selector::All, parameters),
    )
}

#[test]
fn text_equality_is_case_sensitive_unless_declared_otherwise() {
    let strict = check("FireRating", "equal", vec![("text", string("F30"))]);
    assert_eq!(flagged(&strict), ["a", "b", "c"]);
    let folded = check(
        "FireRating",
        "equal",
        vec![("text", string("F30")), ("case_sensitive", boolean(false))],
    );
    assert_eq!(flagged(&folded), ["a", "c"]);
}

#[test]
fn matches_is_a_whole_value_regular_expression() {
    let evaluation = check("FireRating", "matches", vec![("text", string("F\\d+"))]);
    // `f30` fails on case, the blank value fails, and `F90` passes whole.
    assert_eq!(flagged(&evaluation), ["b", "c"]);
    let partial = check("FireRating", "matches", vec![("text", string("F9"))]);
    assert_eq!(flagged(&partial), ["a", "b", "c"]);
}

#[test]
fn an_invalid_regular_expression_is_a_declaration_error() {
    let evaluation = check("FireRating", "matches", vec![("text", string("F("))]);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn one_of_and_none_of_take_a_list() {
    let one = check(
        "FireRating",
        "one_of",
        vec![
            ("texts", strings(&["F30", "F90"])),
            ("case_sensitive", boolean(false)),
        ],
    );
    assert_eq!(flagged(&one), ["c"]);
    let none = check("FireRating", "none_of", vec![("texts", strings(&["F90"]))]);
    assert_eq!(flagged(&none), ["a"]);
}

#[test]
fn contains_looks_inside_the_value() {
    let evaluation = check("FireRating", "contains", vec![("text", string("9"))]);
    assert_eq!(flagged(&evaluation), ["b", "c"]);
}

#[test]
fn presence_treats_blank_text_as_undefined() {
    let defined = check("FireRating", "is_defined", vec![]);
    assert_eq!(flagged(&defined), ["c"]);
    let undefined = check("Width", "is_undefined", vec![]);
    // `c` has no width at all, which is what is_undefined asks for.
    assert_eq!(flagged(&undefined), ["a", "b"]);
}

#[test]
fn numbers_compare_integers_and_decimals_but_absence_fails() {
    let evaluation = check("Width", "less_or_equal", vec![("number", number(0.3))]);
    assert_eq!(flagged(&evaluation), ["b", "c"]);
    let messages = findings(&evaluation);
    assert!(messages[0].1.contains("actual value is 1"), "{messages:?}");
    assert!(
        messages[1].1.contains("actual value is absent"),
        "{messages:?}"
    );
}

#[test]
fn booleans_compare_only_with_booleans() {
    let evaluation = check("LoadBearing", "equal", vec![("boolean", boolean(true))]);
    assert_eq!(flagged(&evaluation), ["b", "c"]);
    let ordered = check(
        "LoadBearing",
        "greater_than",
        vec![("boolean", boolean(true))],
    );
    assert_eq!(
        unevaluated(&ordered),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_target_is_required_exactly_once() {
    for target in [vec![], vec![("text", string("F30")), ("value", integer(3))]] {
        let evaluation = check("FireRating", "equal", target);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    let presence = check("FireRating", "is_defined", vec![("text", string("x"))]);
    assert_eq!(
        unevaluated(&presence),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

#[test]
fn a_quantity_is_never_compared_with_a_bare_number() {
    let model = Model::default().object("q", "slab").value(
        "q",
        "Pset",
        "Depth",
        PropertyValue::Quantity {
            value: 0.2,
            dimension: axioval_ir::QuantityDimension::Length,
        },
    );
    let evaluation = model.evaluate(
        &PropertyPredicate,
        &common::rule(
            CAPABILITY,
            Selector::All,
            vec![
                ("property_set", string("Pset")),
                ("property", string("Depth")),
                ("operator", string("less_than")),
                ("number", number(1.0)),
            ],
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("q".to_owned(), NotEvaluatedReason::InvalidEvidence)]
    );
}
