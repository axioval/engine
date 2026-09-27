//! Property selector operators: what each selects, and what it cannot decide.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ComparisonOperator, ParameterValue, Selector};
use axioval_ir::{NotEvaluatedReason, PropertyValue, QuantityDimension};
use axioval_rules::ManualIssue;
use common::{Model, boolean, flagged, integer, number, rule, string, strings, unevaluated};

const SET: &str = "Pset";
const NAME: &str = "P";

/// `(selected, not evaluated)` for a selector over the model.
fn select(model: Model, selector: Selector) -> (Vec<String>, Vec<(String, NotEvaluatedReason)>) {
    let evaluation = model.evaluate(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            selector,
            vec![("title", string("selected"))],
        ),
    );
    (flagged(&evaluation), unevaluated(&evaluation))
}

fn property(operator: ComparisonOperator, value: ParameterValue) -> Selector {
    Selector::property(Some(SET.into()), NAME, operator, Some(value))
}

fn options(
    operator: ComparisonOperator,
    value: ParameterValue,
    case_sensitive: bool,
    trim: bool,
) -> Selector {
    Selector::Property {
        property_set: Some(SET.into()),
        property: NAME.into(),
        operator,
        value: Some(value),
        case_sensitive,
        trim,
    }
}

fn texts(values: &[(&str, &str)]) -> Model {
    values
        .iter()
        .fold(Model::default(), |model, (local, text)| {
            model.object(local, "thing").text(local, SET, NAME, text)
        })
}

fn values(values: Vec<(&str, PropertyValue)>) -> Model {
    values
        .into_iter()
        .fold(Model::default(), |model, (local, value)| {
            model.object(local, "thing").value(local, SET, NAME, value)
        })
}

fn length(metres: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: metres,
        dimension: QuantityDimension::Length,
    }
}

fn quantity(value: f64, unit: &str) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: unit.into(),
    }
}

fn selected(names: &[&str]) -> (Vec<String>, Vec<(String, NotEvaluatedReason)>) {
    (
        names.iter().map(|name| (*name).to_owned()).collect(),
        vec![],
    )
}

fn invalid_everywhere(model: Model, selector: Selector, objects: &[&str]) {
    let (chosen, undecided) = select(model, selector);
    assert!(chosen.is_empty(), "{chosen:?}");
    assert_eq!(
        undecided,
        objects
            .iter()
            .map(|object| ((*object).to_owned(), NotEvaluatedReason::InvalidDeclaration))
            .collect::<Vec<_>>()
    );
}

#[test]
fn equals_and_not_equals_compare_text_exactly() {
    let model = || texts(&[("a", "Office"), ("b", "office"), ("c", "Kitchen")]);
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Equals, string("Office"))
        ),
        selected(&["a"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::NotEquals, string("Office"))
        ),
        selected(&["b", "c"])
    );
}

#[test]
fn ordered_operators_compare_integers_and_numbers_across_kinds() {
    let model = || {
        values(vec![
            ("a", PropertyValue::Integer(2)),
            ("b", PropertyValue::Decimal(2.5)),
            ("c", PropertyValue::Integer(3)),
        ])
    };
    assert_eq!(
        select(model(), property(ComparisonOperator::LessThan, integer(3))),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::LessThanOrEquals, number(2.5))
        ),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::GreaterThan, number(2.0))
        ),
        selected(&["b", "c"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::GreaterThanOrEquals, integer(3))
        ),
        selected(&["c"])
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::Equals, number(2.0))),
        selected(&["a"])
    );
}

#[test]
fn a_quantity_in_another_unit_is_compared_in_si() {
    let model = || {
        values(vec![
            ("a", length(2.4)),
            ("b", length(0.35)),
            ("c", length(3.0)),
        ])
    };
    assert_eq!(
        select(
            model(),
            property(
                ComparisonOperator::GreaterThanOrEquals,
                quantity(2400.0, "mm")
            )
        ),
        selected(&["a", "c"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::LessThan, quantity(240.0, "cm"))
        ),
        selected(&["b"])
    );
    // 35 cm scales to 0.35000000000000003 m; one conversion is not a difference.
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Equals, quantity(35.0, "cm"))
        ),
        selected(&["b"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::NotEquals, quantity(35.0, "cm"))
        ),
        selected(&["a", "c"])
    );
}

#[test]
fn a_mismatched_type_is_not_evaluated_rather_than_left_out() {
    let model = || {
        values(vec![
            ("text", PropertyValue::String("30".into())),
            ("number", PropertyValue::Integer(30)),
            (
                "area",
                PropertyValue::Quantity {
                    value: 30.0,
                    dimension: QuantityDimension::Area,
                },
            ),
            ("flag", PropertyValue::Boolean(true)),
        ])
    };
    // Text against an integer, a quantity against a unit-less number, a
    // boolean against a number: none of them is a "no".
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::NotEquals, integer(30))
        ),
        (
            vec![],
            vec![
                ("area".into(), NotEvaluatedReason::InvalidEvidence),
                ("flag".into(), NotEvaluatedReason::InvalidEvidence),
                ("text".into(), NotEvaluatedReason::InvalidEvidence),
            ]
        )
    );
    // A length against an area, and plain numbers against a quantity.
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Equals, quantity(30.0, "m"))
        ),
        (
            vec![],
            vec![
                ("area".into(), NotEvaluatedReason::InvalidEvidence),
                ("flag".into(), NotEvaluatedReason::InvalidEvidence),
                ("number".into(), NotEvaluatedReason::InvalidEvidence),
                ("text".into(), NotEvaluatedReason::InvalidEvidence),
            ]
        )
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::Equals, boolean(true))).0,
        ["flag"]
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::Contains, string("3"))).1,
        [
            ("area".into(), NotEvaluatedReason::InvalidEvidence),
            ("flag".into(), NotEvaluatedReason::InvalidEvidence),
            ("number".into(), NotEvaluatedReason::InvalidEvidence),
        ]
    );
}

#[test]
fn an_absent_or_null_value_matches_no_comparison() {
    let model = || {
        Model::default()
            .object("absent", "thing")
            .object("null", "thing")
            .value("null", SET, NAME, PropertyValue::Null)
    };
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::NoneOf, strings(&["A"]))
        ),
        selected(&[])
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::NotEquals, integer(1))),
        selected(&[])
    );
    assert_eq!(
        select(
            model(),
            Selector::property(Some(SET.into()), NAME, ComparisonOperator::Exists, None)
        ),
        selected(&["null"])
    );
}

#[test]
fn matches_is_anchored_to_the_whole_value() {
    let model = || texts(&[("a", "EI30"), ("b", "EI30-T1"), ("c", "XEI30")]);
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Matches, string(r"EI\d+"))
        ),
        selected(&["a"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Matches, string(r"EI\d+.*"))
        ),
        selected(&["a", "b"])
    );
    // Anchors an author already wrote keep their meaning.
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Matches, string(r"^EI\d+$"))
        ),
        selected(&["a"])
    );
    // An alternation is anchored as a whole, not only its last branch.
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Matches, string("EI30|XEI"))
        ),
        selected(&["a"])
    );
}

#[test]
fn like_matches_wildcards_against_the_whole_value() {
    let model = || texts(&[("a", "W1.01"), ("b", "W12.01"), ("c", "W1x01"), ("d", "W*")]);
    assert_eq!(
        select(model(), property(ComparisonOperator::Like, string("W?.01"))),
        selected(&["a"])
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::Like, string("W*.01"))),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::Like, string("W*"))),
        selected(&["a", "b", "c", "d"])
    );
    assert_eq!(
        select(model(), property(ComparisonOperator::Like, string(r"W\*"))),
        selected(&["d"])
    );
}

#[test]
fn contains_finds_a_substring() {
    let model = || texts(&[("a", "Fire door"), ("b", "Door"), ("c", "fireproof")]);
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Contains, string("ire"))
        ),
        selected(&["a", "c"])
    );
}

#[test]
fn one_of_and_none_of_test_membership() {
    let model = || texts(&[("a", "A"), ("b", "B"), ("c", "C")]);
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::OneOf, strings(&["A", "C"]))
        ),
        selected(&["a", "c"])
    );
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::NoneOf, strings(&["A", "C"]))
        ),
        selected(&["b"])
    );
}

#[test]
fn case_insensitive_mode_folds_every_text_operator() {
    let model = || texts(&[("a", "Office"), ("b", "OFFICE 2"), ("c", "Kitchen")]);
    let folded = |operator, value| options(operator, value, false, false);
    assert_eq!(
        select(
            model(),
            folded(ComparisonOperator::Equals, string("office"))
        ),
        selected(&["a"])
    );
    assert_eq!(
        select(
            model(),
            folded(ComparisonOperator::Matches, string("office.*"))
        ),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(model(), folded(ComparisonOperator::Like, string("office*"))),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(
            model(),
            folded(ComparisonOperator::Contains, string("FFIC"))
        ),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(
            model(),
            folded(ComparisonOperator::OneOf, strings(&["KITCHEN"]))
        ),
        selected(&["c"])
    );
    // Case-sensitive by default.
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Equals, string("office"))
        ),
        selected(&[])
    );
}

#[test]
fn trim_drops_surrounding_whitespace_of_the_value() {
    let model = || texts(&[("a", "  Office "), ("b", "Office")]);
    assert_eq!(
        select(
            model(),
            property(ComparisonOperator::Equals, string("Office"))
        ),
        selected(&["b"])
    );
    assert_eq!(
        select(
            model(),
            options(ComparisonOperator::Equals, string("Office"), true, true)
        ),
        selected(&["a", "b"])
    );
    assert_eq!(
        select(
            model(),
            options(ComparisonOperator::Matches, string("Office"), true, true)
        ),
        selected(&["a", "b"])
    );
}

#[test]
fn a_selector_that_does_not_fit_its_operator_is_an_invalid_declaration() {
    let model = || texts(&[("a", "A")]);
    for selector in [
        property(ComparisonOperator::OneOf, string("A")),
        property(ComparisonOperator::Contains, strings(&["A"])),
        property(ComparisonOperator::Like, string("A\\")),
        property(ComparisonOperator::Matches, string("(")),
        property(ComparisonOperator::LessThan, boolean(true)),
        property(ComparisonOperator::Equals, quantity(1.0, "furlong")),
        options(ComparisonOperator::Equals, integer(1), false, false),
        Selector::Property {
            property_set: Some(SET.into()),
            property: NAME.into(),
            operator: ComparisonOperator::Exists,
            value: None,
            case_sensitive: true,
            trim: true,
        },
    ] {
        invalid_everywhere(model(), selector, &["a"]);
    }
}
