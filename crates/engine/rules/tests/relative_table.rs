//! The table mode of relative-count: stepwise minimums, extrapolation beyond
//! the last row, and no requirement below the first.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::ParameterValue;
use axioval_rules::RelativeCount;
use common::{
    Model, findings, flagged, integer, kind, rule, selector, string, strings, unevaluated,
};

const ID: &str = "axioval:capability.relative-count";

/// `relative-count` runs as a template, held on every fixture to the
/// implementation it replaced.
const RELATIVE: common::Held =
    common::Held(&RelativeCount, &axioval_rules::reference::RelativeCount);

/// Storeys with (workplaces, washbasins).
fn storeys(counts: &[(&str, usize, usize)]) -> Model {
    let mut model = Model::default();
    for (storey, workplaces, basins) in counts {
        model = model.object(storey, "storey");
        for index in 0..*workplaces {
            let id = format!("{storey}-w{index}");
            model = model.object(&id, "workplace").edge("contains", storey, &id);
        }
        for index in 0..*basins {
            let id = format!("{storey}-b{index}");
            model = model.object(&id, "washbasin").edge("contains", storey, &id);
        }
    }
    model
}

fn check(model: Model, extra: Vec<(&str, ParameterValue)>) -> axioval_engine::CapabilityEvaluation {
    let mut parameters = vec![
        ("provided_selector", selector(kind("washbasin"))),
        ("required_selector", selector(kind("workplace"))),
        ("relationship", string("contains")),
    ];
    parameters.extend(extra);
    model.evaluate(&RELATIVE, &rule(ID, kind("storey"), parameters))
}

fn table() -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("table", strings(&["10:2", "1:1", "25:3"])),
        ("additional_required", integer(15)),
        ("additional_provided", integer(1)),
    ]
}

#[test]
fn the_applicable_row_and_the_extrapolation_set_the_minimum() {
    let evaluation = check(
        storeys(&[
            ("a", 9, 1),
            ("b", 10, 1),
            ("c", 55, 4),
            ("d", 0, 0),
            ("e", 40, 4),
        ]),
        table(),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "b".into(),
                "1 provided and 10 required object(s) via contains; required at least 2 provided for 10 required".into()
            ),
            (
                "c".into(),
                "4 provided and 55 required object(s) via contains; required at least 5 provided for 55 required".into()
            ),
        ]
    );
}

#[test]
fn without_increments_nothing_beyond_the_rows_is_extrapolated() {
    let evaluation = check(
        storeys(&[("c", 55, 3), ("z", 0, 0)]),
        vec![("table", strings(&["1:1", "25:3"]))],
    );
    assert!(
        evaluation.findings().is_empty(),
        "{:?}",
        findings(&evaluation)
    );
}

#[test]
fn a_malformed_table_is_a_declaration_error() {
    for extra in [
        vec![("table", strings(&["1:1", "ten:2"]))],
        vec![("table", strings(&["1:1", "1:2"]))],
        vec![
            ("table", strings(&["1:1"])),
            ("operator", string("at_least")),
        ],
        vec![
            ("table", strings(&["1:1"])),
            ("additional_required", integer(5)),
        ],
        vec![("table", strings(&[]))],
        vec![
            ("table", strings(&["1:1"])),
            ("small_required_below", integer(4)),
            ("small_provided", integer(1)),
        ],
    ] {
        let evaluation = check(storeys(&[("a", 1, 1)]), extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn below_the_first_row_the_anchor_is_skipped_not_extrapolated() {
    // From zero, 30 workplaces would need two washbasins; below the first
    // row the table sets no requirement at all.
    let evaluation = check(
        storeys(&[("a", 14, 0), ("b", 30, 0), ("c", 40, 0)]),
        vec![
            ("table", strings(&["40:3"])),
            ("additional_required", integer(15)),
            ("additional_provided", integer(1)),
        ],
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "c".into(),
            "0 provided and 40 required object(s) via contains; required at least 3 provided for 40 required".into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
}

#[test]
fn increments_alone_apply_from_zero() {
    let evaluation = check(
        storeys(&[("a", 14, 0), ("b", 15, 0)]),
        vec![
            ("table", strings(&[])),
            ("additional_required", integer(15)),
            ("additional_provided", integer(1)),
        ],
    );
    assert_eq!(flagged(&evaluation), ["b"]);
}
