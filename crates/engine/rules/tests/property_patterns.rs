//! Properties named by XML Schema patterns: `property-value` over every
//! matching property, and the `propertyPattern` selector.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Quantifier, Selector};
use axioval_rules::{ManualIssue, PropertyValueConstraint};
use common::{Model, findings, kind, property, rule, string, strings, unevaluated};

const VALUE: &str = "axioval:capability.property-value";

/// `w1` has `Foobar` and `Foobaz` both `x`, `w2` `Foobar` `x` and `Foobaz`
/// `z`, `w3` only `Other`; all in `Foo_Bar`. `w4` has `Foo` in `Foo_Baz`.
fn model() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("w3", "wall")
        .object("w4", "wall")
        .text("w1", "Foo_Bar", "Foobar", "x")
        .text("w1", "Foo_Bar", "Foobaz", "x")
        .text("w2", "Foo_Bar", "Foobar", "x")
        .text("w2", "Foo_Bar", "Foobaz", "z")
        .text("w3", "Foo_Bar", "Other", "x")
        .text("w4", "Foo_Baz", "Foo", "x")
}

fn values_x(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![("values", strings(&["x"]))];
    parameters.extend(extra);
    parameters
}

#[test]
fn every_matching_property_must_meet_the_constraints() {
    let evaluation = model().evaluate(
        &PropertyValueConstraint,
        &rule(
            VALUE,
            kind("wall"),
            values_x(vec![
                ("property_set_pattern", string("Foo_Bar")),
                ("property_pattern", string("Foo.*")),
            ]),
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "w2".into(),
                "property Foo_Bar.Foobaz is \"z\", not one of the required values".into()
            ),
            (
                "w3".into(),
                "missing required property /Foo_Bar/./Foo.*/: no property matches".into()
            ),
            (
                "w4".into(),
                "missing required property /Foo_Bar/./Foo.*/: no property matches".into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn an_optional_pattern_may_match_nothing_and_any_set_is_searched() {
    let evaluation = model().evaluate(
        &PropertyValueConstraint,
        &rule(
            VALUE,
            kind("wall"),
            values_x(vec![
                ("property_pattern", string("Foo")),
                ("optional", ParameterValue::Boolean { value: true }),
            ]),
        ),
    );
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn a_pattern_rule_is_declared_exactly_once() {
    for parameters in [
        values_x(vec![
            ("property", property(Some("Foo_Bar"), "Foobar")),
            ("property_pattern", string("Foo.*")),
        ]),
        values_x(vec![("property_set_pattern", string("Foo_Bar"))]),
        values_x(vec![]),
        values_x(vec![("property_pattern", string("[a-z-[aeiou]]"))]),
    ] {
        let evaluation = model().evaluate(
            &PropertyValueConstraint,
            &rule(VALUE, kind("wall"), parameters),
        );
        assert_eq!(
            unevaluated(&evaluation),
            [("-".into(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    // A source that cannot list its properties decides nothing.
    let evaluation = model().names_only().evaluate(
        &PropertyValueConstraint,
        &rule(
            VALUE,
            kind("wall"),
            values_x(vec![("property_pattern", string("Foo.*"))]),
        ),
    );
    assert!(findings(&evaluation).is_empty());
    assert_eq!(unevaluated(&evaluation).len(), 4);
}

fn pattern(set: Option<&str>, name: &str, matched: Quantifier) -> Selector {
    Selector::PropertyPattern {
        property_set_pattern: set.map(ToOwned::to_owned),
        property_pattern: name.into(),
        matched,
        operator: ComparisonOperator::Equals,
        value: Some(string("x")),
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision: None,
    }
}

/// The objects a selector picks, and those it leaves undecided.
fn select(model: Model, selector: Selector) -> (Vec<String>, Vec<(String, NotEvaluatedReason)>) {
    let evaluation = model.evaluate(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            selector,
            vec![("title", string("selected"))],
        ),
    );
    let mut chosen: Vec<String> = evaluation
        .findings()
        .iter()
        .flat_map(|finding| finding.object_id().into_iter().chain(&finding.related))
        .map(|id| id.local_id.clone())
        .collect();
    chosen.sort();
    (chosen, unevaluated(&evaluation))
}

#[test]
fn a_pattern_selector_quantifies_over_the_matched_properties() {
    assert_eq!(
        select(model(), pattern(Some("Foo_.*"), "Foo.*", Quantifier::All)).0,
        ["w1", "w4"]
    );
    assert_eq!(
        select(model(), pattern(Some("Foo_.*"), "Foo.*", Quantifier::Any)).0,
        ["w1", "w2", "w4"]
    );
    // Matching nothing is no match, even under `all`.
    assert_eq!(
        select(model(), pattern(None, "Missing", Quantifier::All)).0,
        Vec::<String>::new()
    );
    // Negated, a pattern selects the objects without a matching property.
    assert_eq!(
        select(
            model(),
            Selector::Not {
                operand: Box::new(pattern(None, "Foo.*", Quantifier::Any)),
            }
        )
        .0,
        ["w3"]
    );
    let (chosen, undecided) = select(
        model().names_only(),
        pattern(None, "Foo.*", Quantifier::Any),
    );
    assert!(chosen.is_empty());
    assert_eq!(undecided.len(), 4);
    let (_, invalid) = select(model(), pattern(None, "[a-z-[aeiou]]", Quantifier::Any));
    assert!(
        invalid
            .iter()
            .all(|(_, reason)| *reason == NotEvaluatedReason::InvalidDeclaration)
    );
}
