//! The table mode of relative-count: stepwise minimums with extrapolation.
#![allow(missing_docs)]

mod common;

use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::ParameterValue;
use axioval_rules::RelativeCount;
use common::{Model, findings, integer, kind, rule, selector, string, strings, unevaluated};

const ID: &str = "axioval:capability.relative-count";

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
    model.evaluate(&RelativeCount, &rule(ID, kind("storey"), parameters))
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
    ] {
        let evaluation = check(storeys(&[("a", 1, 1)]), extra);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}
