//! Dates and date-times in property-predicate, property selectors,
//! property-value and property-comparison.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityEvaluation;
use axioval_ir::contract::{ComparisonOperator, ParameterValue, Quantifier, Selector};
use axioval_ir::{NotEvaluatedReason, PropertyValue, TemporalPrecision};
use axioval_rules::{ManualIssue, PropertyComparison, PropertyValueConstraint};
use common::{Model, findings, flagged, number, property, rule, string, strings, unevaluated};

/// `property-value` runs as a template, held on every fixture to the
/// implementation it replaced.
const PROPERTY_VALUE: common::Held = common::Held(
    &PropertyValueConstraint,
    &axioval_rules::reference::PropertyValueConstraint,
);

const SET: &str = "Pset";

fn date(text: &str) -> PropertyValue {
    PropertyValue::Date(text.parse().unwrap())
}

fn date_time(text: &str) -> PropertyValue {
    PropertyValue::DateTime(text.parse().unwrap())
}

fn date_literal(text: &str) -> ParameterValue {
    ParameterValue::Date {
        value: text.parse().unwrap(),
    }
}

fn date_time_literal(text: &str) -> ParameterValue {
    ParameterValue::DateTime {
        value: text.parse().unwrap(),
    }
}

/// `a` to `c` state a date, `d` to `f` a date-time, `g` text, `h` nothing.
fn model() -> Model {
    Model::default()
        .object("a", "door")
        .object("b", "door")
        .object("c", "door")
        .object("d", "door")
        .object("e", "door")
        .object("f", "door")
        .object("g", "door")
        .object("h", "door")
        .value("a", SET, "Inspected", date("2026-09-26"))
        .value("b", SET, "Inspected", date("2026-09-27"))
        .value("c", SET, "Inspected", date("2026-09-28"))
        // 08:00 UTC on the 27th, stated in Berlin.
        .value(
            "d",
            SET,
            "Inspected",
            date_time("2026-09-27T10:00:00+02:00"),
        )
        // 03:30 UTC on the 28th, but the 27th where it was stated.
        .value(
            "e",
            SET,
            "Inspected",
            date_time("2026-09-27T22:30:00-05:00"),
        )
        .value("f", SET, "Inspected", date_time("2026-09-28T00:30:00Z"))
        .text("g", SET, "Inspected", "2026-09-27")
}

fn predicate(operator: &str, target: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("property_set", string(SET)),
        ("property", string("Inspected")),
        ("operator", string(operator)),
    ];
    parameters.extend(target);
    common::predicate(
        model(),
        &rule(
            "axioval:capability.property-predicate",
            Selector::All,
            parameters,
        ),
    )
}

fn not_evaluated(evaluation: &CapabilityEvaluation) -> Vec<String> {
    unevaluated(evaluation)
        .into_iter()
        .map(|(object, _)| object)
        .collect()
}

#[test]
fn property_predicate_compares_dates_by_day() {
    let evaluation = predicate(
        "greater_or_equal",
        vec![("date", date_literal("2026-09-27"))],
    );
    // A date-time needs day precision to meet a date; text is not a date and
    // fails like any other type mismatch; absence fails.
    assert_eq!(flagged(&evaluation), ["a", "g", "h"]);
    assert_eq!(not_evaluated(&evaluation), ["d", "e", "f"]);
    assert!(
        evaluation
            .not_evaluated_outcomes()
            .iter()
            .all(|outcome| outcome.message().contains("declare precision `day`")),
        "{:?}",
        evaluation.not_evaluated_outcomes()
    );
    let messages = findings(&evaluation);
    assert!(
        messages[0]
            .1
            .contains("does not satisfy greater_or_equal 2026-09-27; actual value is 2026-09-26"),
        "{messages:?}"
    );
}

#[test]
fn day_precision_reads_a_date_time_as_the_day_it_states() {
    let evaluation = predicate(
        "equal",
        vec![
            ("date", date_literal("2026-09-27")),
            ("precision", string("day")),
        ],
    );
    // `e` is on the 28th in UTC but states the 27th.
    assert_eq!(flagged(&evaluation), ["a", "c", "f", "g", "h"]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn date_times_compare_as_instants_whatever_their_offsets() {
    let evaluation = predicate(
        "equal",
        vec![("date_time", date_time_literal("2026-09-27T08:00:00Z"))],
    );
    assert_eq!(flagged(&evaluation), ["e", "f", "g", "h"]);
    assert_eq!(not_evaluated(&evaluation), ["a", "b", "c"]);
    let later = predicate(
        "greater_than",
        vec![("date_time", date_time_literal("2026-09-28T00:00:00Z"))],
    );
    // `e` (03:30 UTC) and `f` (00:30 UTC) are after midnight UTC.
    assert_eq!(flagged(&later), ["d", "g", "h"]);
}

#[test]
fn precision_and_operators_that_do_not_fit_a_date_are_declaration_errors() {
    for (operator, target) in [
        ("contains", vec![("date", date_literal("2026-09-27"))]),
        (
            "equal",
            vec![("text", string("x")), ("precision", string("day"))],
        ),
        (
            "equal",
            vec![
                ("date", date_literal("2026-09-27")),
                ("precision", string("month")),
            ],
        ),
        (
            "equal",
            vec![
                ("date", date_literal("2026-09-27")),
                ("tolerance", number(1.0)),
            ],
        ),
    ] {
        let evaluation = predicate(operator, target);
        assert!(evaluation.findings().is_empty());
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{operator}"
        );
    }
}

fn select(selector: Selector) -> (Vec<String>, Vec<String>) {
    let evaluation = model().evaluate(
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
    chosen.dedup();
    (chosen, not_evaluated(&evaluation))
}

fn inspected(
    operator: ComparisonOperator,
    value: ParameterValue,
    precision: Option<TemporalPrecision>,
) -> Selector {
    Selector::Property {
        property_set: Some(SET.into()),
        property: "Inspected".into(),
        operator,
        value: Some(value),
        case_sensitive: true,
        trim: false,
        quantifier: None,
        precision,
    }
}

#[test]
fn a_property_selector_orders_dates_and_leaves_other_kinds_open() {
    let (chosen, open) = select(inspected(
        ComparisonOperator::LessThan,
        date_literal("2026-09-28"),
        None,
    ));
    // Text is not a date and a date-time needs day precision: neither is
    // silently left out.
    assert_eq!(chosen, ["a", "b"]);
    assert_eq!(open, ["d", "e", "f", "g"]);

    let (chosen, open) = select(inspected(
        ComparisonOperator::LessThan,
        date_literal("2026-09-28"),
        Some(TemporalPrecision::Day),
    ));
    assert_eq!(chosen, ["a", "b", "d", "e"]);
    assert_eq!(open, ["g"]);

    let (chosen, open) = select(inspected(
        ComparisonOperator::GreaterThanOrEquals,
        date_time_literal("2026-09-27T09:00:00+01:00"),
        None,
    ));
    assert_eq!(chosen, ["d", "e", "f"]);
    assert_eq!(open, ["a", "b", "c", "g"]);
}

#[test]
fn a_selector_precision_without_a_date_is_a_declaration_error() {
    let (chosen, open) = select(inspected(
        ComparisonOperator::Equals,
        string("2026-09-27"),
        Some(TemporalPrecision::Day),
    ));
    assert!(chosen.is_empty());
    assert_eq!(open, ["a", "b", "c", "d", "e", "f", "g", "h"]);
}

#[test]
fn a_quantified_selector_compares_date_elements() {
    let model = Model::default()
        .object("x", "door")
        .object("y", "door")
        .value(
            "x",
            SET,
            "Inspected",
            PropertyValue::List(vec![date("2026-01-01"), date("2026-09-27")]),
        )
        .value(
            "y",
            SET,
            "Inspected",
            PropertyValue::List(vec![date("2025-01-01")]),
        );
    let mut selector = inspected(
        ComparisonOperator::GreaterThanOrEquals,
        date_literal("2026-06-01"),
        None,
    );
    if let Selector::Property { quantifier, .. } = &mut selector {
        *quantifier = Some(Quantifier::Any);
    }
    let evaluation = model.evaluate(
        &ManualIssue,
        &rule(
            "axioval:capability.manual-issue",
            selector,
            vec![("title", string("selected"))],
        ),
    );
    assert_eq!(flagged(&evaluation), ["x"]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

fn constraint(parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut all = vec![("property", property(Some(SET), "Inspected"))];
    all.extend(parameters);
    model().evaluate(
        &PROPERTY_VALUE,
        &rule("axioval:capability.property-value", Selector::All, all),
    )
}

#[test]
fn property_value_casts_bounds_to_dates() {
    let evaluation = constraint(vec![
        ("min_inclusive", string("2026-09-27")),
        ("max_exclusive", string("2026-09-28")),
    ]);
    // The text value `g` takes no date bounds; it is text, not a date.
    assert_eq!(flagged(&evaluation), ["a", "c", "h"]);
    assert_eq!(not_evaluated(&evaluation), ["d", "e", "f", "g"]);

    let by_day = constraint(vec![
        ("values", strings(&["2026-09-27"])),
        ("precision", string("day")),
    ]);
    assert_eq!(flagged(&by_day), ["a", "c", "f", "h"]);
    assert_eq!(not_evaluated(&by_day), ["g"]);

    let instants = constraint(vec![("values", strings(&["2026-09-27T08:00:00Z"]))]);
    // Text compares as text, exactly.
    assert_eq!(flagged(&instants), ["e", "f", "g", "h"]);
    assert_eq!(not_evaluated(&instants), ["a", "b", "c"]);
}

#[test]
fn property_value_refuses_what_a_date_cannot_take() {
    for parameters in [
        vec![("values", strings(&["27.09.2026"]))],
        vec![("min_length", common::integer(3))],
        vec![
            ("values", strings(&["2026-09-27"])),
            ("precision", string("week")),
        ],
    ] {
        let evaluation = Model::default()
            .object("only", "door")
            .value("only", SET, "Inspected", date("2026-09-27"))
            .evaluate(
                &PROPERTY_VALUE,
                &rule(
                    "axioval:capability.property-value",
                    Selector::All,
                    [
                        vec![("property", property(Some(SET), "Inspected"))],
                        parameters,
                    ]
                    .concat(),
                ),
            );
        assert!(
            evaluation.findings().is_empty(),
            "{:?}",
            evaluation.findings()
        );
        let reasons: Vec<_> = unevaluated(&evaluation)
            .into_iter()
            .map(|(_, reason)| reason)
            .collect();
        assert_eq!(reasons, [NotEvaluatedReason::InvalidDeclaration]);
    }
}

fn comparison(extra: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("compared_selector", common::selector(Selector::All)),
        ("compared_property", property(Some(SET), "Inspected")),
        ("factor", number(1.0)),
        ("component_mode", string("checked")),
        ("quantifier", string("each")),
    ];
    parameters.extend(extra);
    model().evaluate(
        &PropertyComparison,
        &rule(
            "axioval:capability.property-comparison",
            Selector::All,
            parameters,
        ),
    )
}

#[test]
fn property_comparison_orders_dates_against_a_constant_or_a_property() {
    let evaluation = comparison(vec![
        ("operator", string("less_or_equal")),
        ("target_date", date_literal("2026-09-27")),
        ("precision", string("day")),
    ]);
    // `c` and `f` are on the 28th; `g` is text; `h` has no value.
    assert_eq!(flagged(&evaluation), ["c", "f", "h"]);
    assert_eq!(not_evaluated(&evaluation), ["g"]);

    let permits = || {
        Model::default()
            .object("p", "permit")
            .object("q", "permit")
            .value("p", SET, "Issued", date("2026-03-01"))
            .value("p", SET, "Expires", date_time("2026-02-28T23:00:00-02:00"))
            .value("q", SET, "Issued", date("2026-03-01"))
            .value("q", SET, "Expires", date_time("2027-03-01T00:00:00Z"))
    };
    let expiry = |precision: Option<&str>| {
        let mut parameters = vec![
            ("compared_selector", common::selector(Selector::All)),
            ("compared_property", property(Some(SET), "Expires")),
            ("target_property", property(Some(SET), "Issued")),
            ("operator", string("greater")),
            ("factor", number(1.0)),
            ("component_mode", string("checked")),
            ("quantifier", string("each")),
        ];
        if let Some(precision) = precision {
            parameters.push(("precision", string(precision)));
        }
        permits().evaluate(
            &PropertyComparison,
            &rule(
                "axioval:capability.property-comparison",
                Selector::All,
                parameters,
            ),
        )
    };
    // Exactly, a date-time against a date is undecided.
    assert_eq!(not_evaluated(&expiry(None)), ["p", "q"]);
    // By day, `p` expires on the 28th of February it states, before issue.
    let by_day = expiry(Some("day"));
    assert_eq!(flagged(&by_day), ["p"]);
    assert!(by_day.not_evaluated_outcomes().is_empty());
}

/// Inspections `in` on 15 March and `out` on 1 July; `edge`, a zoned first
/// of January, lies within 14 hours of an unzoned one; `late` is zoned but
/// surely after the window; the phase `Start` and `End` of each are stated
/// alongside, except on `open`, which states no end.
fn phases() -> Model {
    let mut model = Model::default();
    for (local, inspected) in [
        ("in", "2026-03-15"),
        ("out", "2026-07-01"),
        ("edge", "2026-01-01Z"),
        ("late", "2026-07-02+01:00"),
        ("open", "2026-03-15"),
    ] {
        model = model
            .object(local, "door")
            .value(local, SET, "Inspected", date(inspected))
            .value(local, SET, "Start", date("2026-01-01"));
        if local != "open" {
            model = model.value(local, SET, "End", date("2026-06-30"));
        }
    }
    model
}

fn between(bounds: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![
        ("compared_selector", common::selector(Selector::All)),
        ("compared_property", property(Some(SET), "Inspected")),
        ("operator", string("between")),
        ("factor", number(1.0)),
        ("component_mode", string("checked")),
        ("quantifier", string("each")),
    ];
    parameters.extend(bounds);
    phases().evaluate(
        &PropertyComparison,
        &rule(
            "axioval:capability.property-comparison",
            Selector::All,
            parameters,
        ),
    )
}

#[test]
fn property_comparison_takes_a_date_window_as_constants_or_properties() {
    let literal = between(vec![
        ("minimum_date", date_literal("2026-01-01")),
        ("maximum_date", date_literal("2026-06-30")),
    ]);
    assert_eq!(flagged(&literal), ["late", "out"]);
    // A zoned date within 14 hours of the unzoned start is in no order.
    assert_eq!(not_evaluated(&literal), ["edge"]);

    let stated = between(vec![
        ("minimum_property", property(Some(SET), "Start")),
        ("maximum_property", property(Some(SET), "End")),
    ]);
    // `open` states no end: its window is incomplete, a finding.
    assert_eq!(flagged(&stated), ["late", "open", "out"]);
    assert_eq!(not_evaluated(&stated), ["edge"]);
    // A stated bound is cited.
    let out = stated
        .findings()
        .iter()
        .find(|finding| common::subject(finding) == "out")
        .unwrap();
    assert!(
        out.evidence
            .iter()
            .any(|evidence| evidence.locator.ends_with("Pset.End")),
        "{:?}",
        out.evidence
    );

    // One constant bound and one stated bound mix.
    let mixed = between(vec![
        ("minimum_date", date_literal("2026-03-16")),
        ("maximum_property", property(Some(SET), "End")),
    ]);
    assert_eq!(flagged(&mixed), ["edge", "in", "late", "open", "out"]);
}

#[test]
fn property_comparison_orders_date_time_bounds_as_instants() {
    let window = |precision: Option<&str>| {
        let mut bounds = vec![
            (
                "minimum_date_time",
                date_time_literal("2026-03-15T00:00:00Z"),
            ),
            (
                "maximum_date_time",
                date_time_literal("2026-03-15T23:59:59Z"),
            ),
        ];
        if let Some(precision) = precision {
            bounds.push(("precision", string(precision)));
        }
        let mut parameters = vec![
            ("compared_selector", common::selector(Selector::All)),
            ("compared_property", property(Some(SET), "Done")),
            ("operator", string("between")),
            ("factor", number(1.0)),
            ("component_mode", string("checked")),
            ("quantifier", string("each")),
        ];
        parameters.extend(bounds);
        Model::default()
            .object("utc", "door")
            .object("east", "door")
            .object("day", "door")
            .value("utc", SET, "Done", date_time("2026-03-15T12:00:00Z"))
            // 22:30 UTC on the 14th.
            .value("east", SET, "Done", date_time("2026-03-15T00:30:00+02:00"))
            .value("day", SET, "Done", date("2026-03-15"))
            .evaluate(
                &PropertyComparison,
                &rule(
                    "axioval:capability.property-comparison",
                    Selector::All,
                    parameters,
                ),
            )
    };
    let exact = window(None);
    assert_eq!(flagged(&exact), ["east"]);
    // A date against date-time bounds compares only by day.
    assert_eq!(not_evaluated(&exact), ["day"]);
    let by_day = window(Some("day"));
    assert!(by_day.findings().is_empty(), "{:?}", findings(&by_day));
    assert!(by_day.not_evaluated_outcomes().is_empty());
}

#[test]
fn property_comparison_refuses_date_bounds_of_mixed_kinds_or_in_reverse() {
    for bounds in [
        vec![
            ("minimum_date", date_literal("2026-01-01")),
            (
                "maximum_date_time",
                date_time_literal("2026-06-30T00:00:00Z"),
            ),
        ],
        vec![
            ("minimum_date", date_literal("2026-01-01")),
            ("maximum_number", number(3.0)),
        ],
        vec![
            ("minimum_date", date_literal("2026-06-30")),
            ("maximum_date", date_literal("2026-01-01")),
        ],
        vec![
            ("minimum_date", date_literal("2026-01-01")),
            ("minimum_property", property(Some(SET), "Start")),
            ("maximum_date", date_literal("2026-06-30")),
        ],
        vec![
            ("minimum_date", date_literal("2026-01-01")),
            ("maximum_date", date_literal("2026-06-30")),
            ("tolerance", number(1.0)),
        ],
    ] {
        assert_eq!(
            unevaluated(&between(bounds.clone())),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)],
            "{bounds:?}"
        );
    }
}

#[test]
fn property_comparison_refuses_precision_off_dates_and_a_factor_on_them() {
    let precision_on_text = comparison(vec![
        ("operator", string("equals")),
        ("target_text", string("x")),
        ("precision", string("day")),
    ]);
    assert_eq!(
        unevaluated(&precision_on_text),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    let mut parameters = vec![
        ("compared_selector", common::selector(Selector::All)),
        ("compared_property", property(Some(SET), "Inspected")),
        ("factor", number(2.0)),
        ("component_mode", string("checked")),
        ("quantifier", string("each")),
        ("operator", string("equals")),
    ];
    parameters.push(("target_date", date_literal("2026-09-27")));
    let scaled = Model::default()
        .object("a", "door")
        .value("a", SET, "Inspected", date("2026-09-27"))
        .evaluate(
            &PropertyComparison,
            &rule(
                "axioval:capability.property-comparison",
                Selector::All,
                parameters,
            ),
        );
    assert!(scaled.findings().is_empty());
    assert_eq!(not_evaluated(&scaled), ["a"]);
}

#[test]
fn one_instant_in_two_offsets_is_one_value() {
    let evaluation = Model::default()
        .object("x", "door")
        .object("y", "door")
        .object("z", "door")
        .value(
            "x",
            SET,
            "Inspected",
            date_time("2026-09-27T10:00:00+02:00"),
        )
        .value("y", SET, "Inspected", date_time("2026-09-27T08:00:00Z"))
        .value("z", SET, "Inspected", date("2026-09-27"))
        .evaluate(
            &common::Held(
                &axioval_rules::UniqueValue,
                &axioval_rules::reference::UniqueValue,
            ),
            &rule(
                "axioval:capability.unique-value",
                Selector::All,
                vec![("property", property(Some(SET), "Inspected"))],
            ),
        );
    assert_eq!(flagged(&evaluation), ["x", "y"]);
}

/// `u` states 2022-01-01 in UTC, `v` the unzoned day, `w` and `x` one day
/// beginning at 12:00 UTC in two zones, `y` 2022-01-03 in UTC.
fn zoned_model() -> Model {
    Model::default()
        .object("u", "door")
        .object("v", "door")
        .object("w", "door")
        .object("x", "door")
        .object("y", "door")
        .value("u", SET, "Inspected", date("2022-01-01+00:00"))
        .value("v", SET, "Inspected", date("2022-01-01"))
        .value("w", SET, "Inspected", date("2022-01-02+12:00"))
        .value("x", SET, "Inspected", date("2022-01-01-12:00"))
        .value("y", SET, "Inspected", date("2022-01-03Z"))
}

fn zoned_value(extra: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    let mut parameters = vec![("property", property(Some(SET), "Inspected"))];
    parameters.extend(extra);
    zoned_model().evaluate(
        &PROPERTY_VALUE,
        &rule(
            "axioval:capability.property-value",
            Selector::All,
            parameters,
        ),
    )
}

#[test]
fn a_zoned_date_equals_no_unzoned_date() {
    let equal = zoned_value(vec![("values", strings(&["2022-01-01"]))]);
    assert_eq!(flagged(&equal), ["u", "w", "x", "y"]);
    assert!(equal.not_evaluated_outcomes().is_empty());
    // Zoned literals compare the instants the days begin.
    let zoned = zoned_value(vec![("values", strings(&["2022-01-01-12:00"]))]);
    assert_eq!(flagged(&zoned), ["u", "v", "y"]);
    // An order XML Schema leaves indeterminate is not evaluated.
    let bound = zoned_value(vec![("min_inclusive", string("2022-01-01"))]);
    assert!(flagged(&bound).is_empty());
    assert_eq!(not_evaluated(&bound), ["u", "w", "x"]);
    assert!(
        bound
            .not_evaluated_outcomes()
            .iter()
            .all(|outcome| outcome.message().contains("within 14 hours")),
        "{:?}",
        bound.not_evaluated_outcomes()
    );
    // At day precision only the stated day counts.
    let by_day = zoned_value(vec![
        ("values", strings(&["2022-01-01"])),
        ("precision", string("day")),
    ]);
    assert_eq!(flagged(&by_day), ["w", "y"]);
}

#[test]
fn zoned_dates_compare_as_xml_schema_orders_them_in_every_capability() {
    let predicate = |operator: &str| {
        common::predicate(
            zoned_model(),
            &rule(
                "axioval:capability.property-predicate",
                Selector::All,
                vec![
                    ("property_set", string(SET)),
                    ("property", string("Inspected")),
                    ("operator", string(operator)),
                    ("date", date_literal("2022-01-01")),
                ],
            ),
        )
    };
    assert_eq!(flagged(&predicate("equal")), ["u", "w", "x", "y"]);
    assert_eq!(flagged(&predicate("not_equal")), ["v"]);
    let less = predicate("less_than");
    assert_eq!(flagged(&less), ["v", "y"]);
    assert_eq!(not_evaluated(&less), ["u", "w", "x"]);

    let compared = zoned_model().evaluate(
        &PropertyComparison,
        &rule(
            "axioval:capability.property-comparison",
            Selector::All,
            vec![
                ("compared_selector", common::selector(Selector::All)),
                ("compared_property", property(Some(SET), "Inspected")),
                ("factor", number(1.0)),
                ("component_mode", string("checked")),
                ("quantifier", string("each")),
                ("operator", string("equals")),
                ("target_date", date_literal("2022-01-01")),
            ],
        ),
    );
    assert_eq!(flagged(&compared), ["u", "w", "x", "y"]);
    assert!(compared.not_evaluated_outcomes().is_empty());

    // Two zones stating one day are one value; the unzoned day is another.
    let unique = zoned_model().evaluate(
        &common::Held(
            &axioval_rules::UniqueValue,
            &axioval_rules::reference::UniqueValue,
        ),
        &rule(
            "axioval:capability.unique-value",
            Selector::All,
            vec![("property", property(Some(SET), "Inspected"))],
        ),
    );
    assert_eq!(flagged(&unique), ["w", "x"]);

    let chosen = |operator| {
        let evaluation = zoned_model().evaluate(
            &ManualIssue,
            &rule(
                "axioval:capability.manual-issue",
                inspected(operator, date_literal("2022-01-01"), None),
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
        chosen.dedup();
        (chosen, not_evaluated(&evaluation))
    };
    let (equal, open) = chosen(ComparisonOperator::Equals);
    assert_eq!(equal, ["v"]);
    assert!(open.is_empty());
    let (other, _) = chosen(ComparisonOperator::NotEquals);
    assert_eq!(other, ["u", "w", "x", "y"]);
    let (later, open) = chosen(ComparisonOperator::GreaterThan);
    assert_eq!(later, ["y"]);
    assert_eq!(open, ["u", "w", "x"]);
}
