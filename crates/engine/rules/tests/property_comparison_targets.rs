//! Constant targets and the `count` and `sum` quantifiers of property-comparison.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::contract::ParameterValue;
use axioval_ir::{NotEvaluatedReason, PropertyValue};
use axioval_rules::PropertyComparison;
use common::{
    Model, boolean, findings, flagged, kind, number, property, selector, string, strings,
    unevaluated,
};

/// `property-comparison` runs as a template, held on every fixture to the
/// implementation it replaced.
const PROPERTY_COMPARISON: common::Held = common::Held(
    &PropertyComparison,
    &axioval_rules::reference::PropertyComparison,
);

const CAPABILITY: &str = "axioval:capability.property-comparison";

/// Two rooms: `r1` holds three chairs, `r2` holds one.
fn rooms() -> Model {
    Model::default()
        .object("r1", "room")
        .object("r2", "room")
        .object("c1", "chair")
        .object("c2", "chair")
        .object("c3", "chair")
        .object("c4", "chair")
        .edge("contains", "r1", "c1")
        .edge("contains", "r1", "c2")
        .edge("contains", "r1", "c3")
        .edge("contains", "r2", "c4")
        .value("r1", "Pset", "Seats", PropertyValue::Integer(3))
        .value("r2", "Pset", "Seats", PropertyValue::Integer(2))
        .value("c1", "Pset", "Width", PropertyValue::Decimal(0.5))
        .value("c2", "Pset", "Width", PropertyValue::Decimal(0.5))
        .value("c3", "Pset", "Width", PropertyValue::Integer(1))
        .value("c4", "Pset", "Width", PropertyValue::Decimal(0.75))
        .text("c1", "Pset", "Colour", "red")
        .text("c2", "Pset", "Colour", "blue")
        .text("c3", "Pset", "Colour", "red")
        .text("c4", "Pset", "Colour", "green")
}

fn compare(
    model: Model,
    quantifier: &str,
    operator: &str,
    extra: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("compared_selector", selector(kind("chair"))),
        ("operator", string(operator)),
        ("factor", number(1.0)),
        ("component_mode", string("related")),
        ("relationship", string("contains")),
        ("quantifier", string(quantifier)),
    ];
    parameters.extend(extra);
    model.evaluate(
        &PROPERTY_COMPARISON,
        &common::rule(CAPABILITY, kind("room"), parameters),
    )
}

#[test]
fn each_compares_every_candidate_with_a_constant() {
    let evaluation = compare(
        rooms(),
        "each",
        "less_or_equal",
        vec![
            ("compared_property", property(Some("Pset"), "Width")),
            ("target_number", number(0.6)),
        ],
    );
    // c3 (1) in r1 and c4 (0.75) in r2 are too wide.
    assert_eq!(flagged(&evaluation), ["r1", "r2"]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

/// Numbers compare through the one comparison every rule uses: a signed
/// zero is zero, as for selectors and expressions (divergence D20 of the
/// parity chapter: the capability once ordered `-0` below `0`).
#[test]
fn a_signed_zero_equals_zero() {
    let model = rooms()
        .value("c1", "Pset", "Offset", PropertyValue::Decimal(-0.0))
        .value("c2", "Pset", "Offset", PropertyValue::Decimal(0.0))
        .value("c3", "Pset", "Offset", PropertyValue::Decimal(0.0))
        .value("c4", "Pset", "Offset", PropertyValue::Decimal(-0.0));
    let evaluation = compare(
        model,
        "each",
        "equals",
        vec![
            ("compared_property", property(Some("Pset"), "Offset")),
            ("target_number", number(0.0)),
        ],
    );
    assert!(
        flagged(&evaluation).is_empty(),
        "{:?}",
        findings(&evaluation)
    );
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_text_list_target_is_one_of_or_none_of() {
    let allowed = compare(
        rooms(),
        "each",
        "one_of",
        vec![
            ("compared_property", property(Some("Pset"), "Colour")),
            ("target_texts", strings(&["red", "green"])),
        ],
    );
    assert_eq!(
        findings(&allowed),
        [(
            "r1".into(),
            "candidate test:model/c2 does not satisfy comparison".into()
        )]
    );
    let forbidden = compare(
        rooms(),
        "at_least_one",
        "none_of",
        vec![
            ("compared_property", property(Some("Pset"), "Colour")),
            ("target_texts", strings(&["red", "blue"])),
        ],
    );
    // r1 has no chair outside red and blue; r2's green chair satisfies it.
    assert_eq!(flagged(&forbidden), ["r1"]);
}

#[test]
fn count_compares_the_number_of_candidates_with_a_property_of_the_checked_object() {
    let evaluation = compare(
        rooms(),
        "count",
        "equals",
        vec![("target_property", property(Some("Pset"), "Seats"))],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "r2".into(),
            "count of compared components is 1 and is not equals 2".into()
        )]
    );
}

#[test]
fn count_of_nothing_is_zero_not_a_skip() {
    let model = rooms().object("r3", "room");
    let evaluation = compare(
        model,
        "count",
        "greater_or_equal",
        vec![("target_number", number(1.0))],
    );
    assert_eq!(flagged(&evaluation), ["r3"]);
}

#[test]
fn sum_adds_integers_and_decimals() {
    let evaluation = compare(
        rooms(),
        "sum",
        "less_or_equal",
        vec![
            ("compared_property", property(Some("Pset"), "Width")),
            ("target_number", number(1.5)),
        ],
    );
    // r1: 0.5 + 0.5 + 1 = 2 exceeds 1.5; r2: 0.75 does not.
    assert_eq!(
        findings(&evaluation),
        [(
            "r1".into(),
            "sum of compared values is 2 and is not less_or_equal 1.5".into()
        )]
    );
}

#[test]
fn a_sum_with_an_absent_value_draws_no_verdict() {
    let model = rooms().object("c5", "chair").edge("contains", "r2", "c5");
    let evaluation = compare(
        model,
        "sum",
        "greater",
        vec![
            ("compared_property", property(Some("Pset"), "Width")),
            ("target_number", number(100.0)),
        ],
    );
    // r1 fails for real; r2's partial total is not judged, and the
    // chair without a width is reported instead.
    assert_eq!(
        flagged(&evaluation),
        ["c5", "r1"],
        "{:?}",
        unevaluated(&evaluation)
    );
}

#[test]
fn a_sum_of_mixed_kinds_is_not_evaluated() {
    let model = rooms().text("c4", "Pset", "Width", "wide");
    let evaluation = compare(
        model,
        "sum",
        "less_or_equal",
        vec![
            ("compared_property", property(Some("Pset"), "Width")),
            ("target_number", number(10.0)),
        ],
    );
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("r2".to_owned(), NotEvaluatedReason::InvalidEvidence)]
    );
}

#[test]
fn a_boolean_constant_is_compared_for_equality() {
    let model = rooms().value("c1", "Pset", "Stackable", PropertyValue::Boolean(false));
    let evaluation = compare(
        model,
        "at_least_one",
        "equals",
        vec![
            ("compared_property", property(Some("Pset"), "Stackable")),
            ("target_boolean", boolean(true)),
        ],
    );
    // r1: c1 is not stackable, c2 and c3 do not say -- missing information,
    // not a violation of "at least one". r2: c4 does not say either.
    assert_eq!(flagged(&evaluation), ["c2", "c3", "c4"]);
}

#[test]
fn targets_are_declared_exactly_once() {
    for extra in [
        vec![("compared_property", property(Some("Pset"), "Width"))],
        vec![
            ("compared_property", property(Some("Pset"), "Width")),
            ("target_number", number(1.0)),
            ("target_text", string("x")),
        ],
        // A text list is only a target of one_of/none_of, and vice versa.
        vec![
            ("compared_property", property(Some("Pset"), "Colour")),
            ("target_texts", strings(&["red"])),
        ],
    ] {
        let evaluation = compare(rooms(), "each", "equals", extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
    let sum_without_property = compare(
        rooms(),
        "sum",
        "equals",
        vec![("target_number", number(1.0))],
    );
    assert_eq!(
        unevaluated(&sum_without_property),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// Every refusal keeps its words after `property-comparison parameters are
/// invalid: `.
#[test]
#[allow(clippy::type_complexity)]
fn every_refusal_keeps_its_wording() {
    let refusal = |quantifier: &str, operator: &str, extra: Vec<(&str, ParameterValue)>| {
        compare(rooms(), quantifier, operator, extra)
            .not_evaluated_outcomes()
            .iter()
            .map(|outcome| outcome.message().to_owned())
            .collect::<Vec<_>>()
    };
    let cases: Vec<(&str, &str, Vec<(&str, ParameterValue)>, &str)> = vec![
        ("each", "beside", vec![], "operator `beside` is unsupported"),
        (
            "each",
            "equals",
            vec![("target_number", number(1.0)), ("target_text", string("a"))],
            "declare exactly one target",
        ),
        (
            "every",
            "equals",
            vec![("target_number", number(1.0))],
            "quantifier `every` is unsupported",
        ),
        (
            "each",
            "equals",
            vec![("target_number", number(1.0))],
            "`compared_property` is required except for `count`",
        ),
        (
            "count",
            "contains",
            vec![("target_text", string("a"))],
            "operator `contains` does not apply to a count or sum",
        ),
    ];
    for (quantifier, operator, extra, message) in cases {
        assert_eq!(
            refusal(quantifier, operator, extra),
            [format!(
                "property-comparison parameters are invalid: {message}"
            )]
        );
    }
}

/// Generated rooms of chairs stating widths, seats, colours and ratings of
/// every kind (numbers, integers, lengths, text, lists, `null` or nothing,
/// some unreadable), compared each, at least one, by count or by sum,
/// against constants, properties, text lists and ranges, scaled, under a
/// tolerance or not. The template is held to the implementation it
/// replaced on each.
mod generated {
    use super::*;
    use axioval_ir::QuantityDimension;
    use proptest::prelude::*;

    fn value(kind: u8) -> Option<PropertyValue> {
        Some(match kind {
            0 => PropertyValue::Decimal(0.5),
            1 => PropertyValue::Integer(1),
            2 => PropertyValue::Quantity {
                value: 0.5,
                dimension: QuantityDimension::Length,
            },
            3 => PropertyValue::String("red".into()),
            4 => PropertyValue::String("Red".into()),
            5 => PropertyValue::Null,
            6 => PropertyValue::List(vec![PropertyValue::Integer(1)]),
            7 => PropertyValue::Decimal(-0.0),
            _ => return None,
        })
    }

    fn target(kind: u8) -> (&'static str, Vec<(&'static str, ParameterValue)>) {
        match kind {
            0 => ("greater_or_equal", vec![("target_number", number(0.5))]),
            1 => (
                "less",
                vec![("target_property", property(Some("Pset"), "Seats"))],
            ),
            2 => ("one_of", vec![("target_texts", strings(&["red", "blue"]))]),
            3 => ("like", vec![("target_text", string("r*"))]),
            4 => (
                "between",
                vec![
                    ("minimum_number", number(0.0)),
                    ("maximum_number", number(1.0)),
                ],
            ),
            5 => (
                "equals",
                vec![(
                    "target_quantity",
                    ParameterValue::Quantity {
                        value: 500.0,
                        unit: "mm".into(),
                    },
                )],
            ),
            6 => ("is_defined", vec![]),
            _ => ("not_equals", vec![("target_boolean", boolean(true))]),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn generated_rooms_hold_parity(
            chairs in proptest::collection::vec((0u8..2, 0u8..9, any::<bool>()), 0..6),
            seats in proptest::option::of(0u8..9),
            quantifier in 0u8..4,
            which in 0u8..8,
            factor in proptest::option::of(0.5f64..2.0),
            tolerance in any::<bool>(),
            mode in 0u8..2,
        ) {
            let mut model = Model::default().object("r1", "room").object("r2", "room");
            for (index, (room, width, unreadable)) in chairs.iter().enumerate() {
                let local = format!("c{index}");
                model = model
                    .object(&local, "chair")
                    .edge("contains", &format!("r{}", room + 1), &local);
                if let Some(value) = value(*width) {
                    model = model.value(&local, "Pset", "Width", value);
                }
                if *unreadable && index % 2 == 1 {
                    model = model.unreadable(&local);
                }
            }
            if let Some(value) = seats.and_then(value) {
                model = model.value("r1", "Pset", "Seats", value);
            }
            let (operator, mut extra) = target(which);
            let quantifier = ["each", "at_least_one", "count", "sum"][usize::from(quantifier)];
            if quantifier != "count" {
                extra.push(("compared_property", property(Some("Pset"), "Width")));
            }
            if tolerance {
                extra.push(("tolerance", number(0.01)));
            }
            let mut model_extra = extra;
            let mut parameters = vec![
                ("compared_selector", selector(kind("chair"))),
                ("operator", string(operator)),
                ("factor", number(factor.unwrap_or(1.0))),
                ("quantifier", string(quantifier)),
            ];
            if mode == 0 {
                parameters.push(("component_mode", string("related")));
                parameters.push(("relationship", string("contains")));
            } else {
                parameters.push(("component_mode", string("checked")));
            }
            parameters.append(&mut model_extra);
            model.evaluate(
                &PROPERTY_COMPARISON,
                &common::rule(CAPABILITY, kind("room"), parameters),
            );
        }
    }
}
