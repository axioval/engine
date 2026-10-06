//! `recess-width`: each recess wide enough for its depth, by table row.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    PlanLength, PlanRecess, PlanRecesses, PlanSpan, PlanSpanError, PlanSpanService,
    PlanSpanServiceHandle,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};
use axioval_rules::RecessWidth;
use common::{Model, findings, id, kind, number, rule, source, unevaluated};

const ID: &str = "axioval:capability.recess-width";

/// Recesses per object as `(width, depth)` intervals, or a refusal.
struct Recesses(BTreeMap<ObjectId, Found>);

/// Each recess as `(width, depth)` intervals, or why none were measured.
type Found = Result<Vec<((f64, f64), (f64, f64))>, String>;

fn length((lower, upper): (f64, f64), locator: &str) -> PlanLength {
    #[allow(clippy::float_cmp)]
    let exact = lower == upper;
    PlanLength::try_new(
        lower,
        upper,
        Evidence {
            source: source(),
            locator: locator.into(),
            exact,
        },
    )
    .unwrap()
}

impl PlanSpanService for Recesses {
    fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("unused".into()))
    }
    fn measure_span(
        &self,
        _: &ObjectId,
        _: &ObjectId,
        _: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        Err(PlanSpanError::Unavailable("unused".into()))
    }
    fn measure_recesses(&self, object: &ObjectId) -> Result<PlanRecesses, PlanSpanError> {
        let found = self
            .0
            .get(object)
            .ok_or_else(|| PlanSpanError::UnknownObject(object.clone()))?
            .clone()
            .map_err(|why| {
                // A footprint the service measures only approximately.
                if why == "inexact" {
                    PlanSpanError::InexactEvidence
                } else {
                    PlanSpanError::Unavailable(why)
                }
            })?;
        let recesses = found
            .into_iter()
            .enumerate()
            .map(|(index, (width, depth))| {
                let x = f64::from(u32::try_from(index).unwrap());
                PlanRecess::try_new(
                    [[x, 0.0], [x + width.0, 0.0]],
                    length(width, &format!("width:{index}")),
                    length(depth, &format!("depth:{index}")),
                )
                .unwrap()
            })
            .collect();
        PlanRecesses::try_new(
            object.clone(),
            recesses,
            Evidence::exact(source(), format!("recesses:{object}")),
        )
    }
}

fn row(cells: &[(&str, f64)]) -> TableRow {
    cells
        .iter()
        .map(|(column, value)| ((*column).to_owned(), number(*value)))
        .collect()
}

/// Up to 1 m deep, at least 1 m wide; deeper, at least as wide as deep.
fn table() -> ParameterValue {
    ParameterValue::Table {
        value: vec![
            row(&[("maximum_depth_metres", 1.0), ("minimum_width_metres", 1.0)]),
            row(&[
                ("minimum_depth_metres", 1.0),
                ("minimum_width_per_depth", 1.0),
            ]),
        ],
    }
}

/// `recess-width` as it runs, held to the implementation it replaced on
/// every evaluation.
static HELD: common::Held = common::Held(&RecessWidth, &axioval_rules::reference::RecessWidth);

fn evaluate(
    recesses: Vec<(&str, Found)>,
    requirements: ParameterValue,
) -> axioval_engine::CapabilityEvaluation {
    let mut model = Model::default();
    for (local, _) in &recesses {
        model = model.object(local, "space");
    }
    let service = Arc::new(Recesses(
        recesses
            .into_iter()
            .map(|(local, found)| (id(local), found))
            .collect(),
    ));
    let rule = rule(ID, kind("space"), vec![("requirements", requirements)]);
    model.evaluate_measured(&HELD, &rule, move |services| {
        services
            .register(PlanSpanServiceHandle::new(service.clone()))
            .unwrap();
    })
}

fn point(value: f64) -> (f64, f64) {
    (value, value)
}

#[test]
fn each_recess_is_judged_by_the_row_its_depth_selects() {
    let outcome = evaluate(
        vec![
            // 0.8 m deep needs 1 m: 1.2 m passes, 0.9 m fails.
            (
                "a",
                Ok(vec![(point(1.2), point(0.8)), (point(0.9), point(0.8))]),
            ),
            // 2 m deep needs 2 m: 2.5 m passes, 1.5 m fails.
            (
                "b",
                Ok(vec![(point(2.5), point(2.0)), (point(1.5), point(2.0))]),
            ),
            // A convex space has no recess.
            ("c", Ok(Vec::new())),
        ],
        table(),
    );
    assert_eq!(
        findings(&outcome),
        vec![
            (
                "a".to_owned(),
                "recess at (1.000, 0.000)-(1.900, 0.000) is 0.9 m wide and 0.8 m deep; row 0 \
                 requires at least 1 m"
                    .to_owned()
            ),
            (
                "b".to_owned(),
                "recess at (1.000, 0.000)-(2.500, 0.000) is 1.5 m wide and 2 m deep; row 1 \
                 requires at least 2 m"
                    .to_owned()
            ),
        ]
    );
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_recess_no_row_holds_has_no_requirement() {
    let only_deep = ParameterValue::Table {
        value: vec![row(&[
            ("minimum_depth_metres", 1.0),
            ("minimum_width_metres", 3.0),
        ])],
    };
    let outcome = evaluate(vec![("a", Ok(vec![(point(0.5), point(1.0))]))], only_deep);
    assert!(findings(&outcome).is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn straddling_intervals_decide_nothing() {
    let outcome = evaluate(
        vec![
            // A depth either side of 1 m: which row applies is undecided.
            ("a", Ok(vec![(point(5.0), (0.9, 1.1))])),
            // A width either side of the 1 m required.
            ("b", Ok(vec![((0.9, 1.1), point(0.5))])),
        ],
        table(),
    );
    assert!(findings(&outcome).is_empty());
    assert_eq!(
        unevaluated(&outcome),
        vec![
            ("a".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("b".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
    assert_eq!(
        messages(&outcome),
        vec![
            "recess at (0.000, 0.000)-(5.000, 0.000) is 5 m wide and between 0.9 m and 1.1 m \
             deep; which row applies is undecided",
            "recess at (0.000, 0.000)-(0.900, 0.000) is between 0.9 m and 1.1 m wide and \
             0.5 m deep; row 0 requires at least 1 m, undecided",
        ]
    );
}

fn messages(outcome: &axioval_engine::CapabilityEvaluation) -> Vec<String> {
    outcome
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| outcome.message().to_owned())
        .collect()
}

#[test]
fn an_unmeasured_space_or_a_missing_service_is_not_evaluated() {
    let outcome = evaluate(
        vec![
            ("a", Err("tessellated".into())),
            ("b", Err("inexact".into())),
        ],
        table(),
    );
    assert_eq!(
        unevaluated(&outcome),
        vec![
            ("a".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("b".to_owned(), NotEvaluatedReason::InvalidEvidence),
        ]
    );
    assert_eq!(
        messages(&outcome),
        vec![
            "the recesses of test:model/a cannot be measured: plan span unavailable: tessellated"
                .to_owned(),
            format!(
                "the recesses of test:model/b cannot be measured: {}",
                PlanSpanError::InexactEvidence
            ),
        ]
    );
    let rule = rule(ID, kind("space"), vec![("requirements", table())]);
    let outcome = Model::default()
        .object("a", "space")
        .evaluate_measured(&HELD, &rule, |_| {});
    assert_eq!(
        unevaluated(&outcome),
        vec![("a".to_owned(), NotEvaluatedReason::MissingService)]
    );
    assert_eq!(
        messages(&outcome),
        vec!["plan-span service is not registered".to_owned()]
    );
}

#[test]
fn a_row_without_a_width_or_with_an_empty_range_is_invalid() {
    for bad in [
        row(&[("maximum_depth_metres", 1.0)]),
        row(&[
            ("minimum_depth_metres", 2.0),
            ("maximum_depth_metres", 1.0),
            ("minimum_width_metres", 1.0),
        ]),
        row(&[("minimum_width_metres", -1.0)]),
    ] {
        let outcome = evaluate(
            vec![("a", Ok(Vec::new()))],
            ParameterValue::Table { value: vec![bad] },
        );
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
    let refused = |bad: TableRow| {
        let outcome = evaluate(
            vec![("a", Ok(Vec::new()))],
            ParameterValue::Table { value: vec![bad] },
        );
        messages(&outcome)
    };
    assert_eq!(
        refused(row(&[("maximum_depth_metres", 1.0)])),
        vec![
            "recess-width: row 0 needs `minimum_width_metres`, `minimum_width_per_depth` or \
             both"
                .to_owned()
        ]
    );
    assert_eq!(
        refused(row(&[
            ("minimum_depth_metres", 2.0),
            ("maximum_depth_metres", 1.0),
            ("minimum_width_metres", 1.0),
        ])),
        vec![
            "recess-width: row 0: `minimum_depth_metres` must be below `maximum_depth_metres`"
                .to_owned()
        ]
    );
    assert_eq!(
        refused(row(&[("minimum_width_metres", -1.0)])),
        vec!["recess-width: row 0: `minimum_width_metres` must not be negative".to_owned()]
    );
}

/// Each fixture's recesses judged by an expression over the measured
/// recesses: no recess narrower than its row requires. The parity harness
/// holds on every fixture but one, where the expression decides a recess
/// whose row the capability leaves open.
#[test]
#[allow(clippy::too_many_lines, clippy::type_complexity)]
fn the_rows_as_an_expression_over_recesses_reach_the_verdicts() {
    use serde_json::{Value, json};
    let field =
        |name: &str| json!({"kind": "property", "propertySet": "axioval:member", "property": name});
    let m = |value: f64| json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "m"}});
    let compare = |operator: &str, left: Value, right: Value| json!({"kind": "compare", "operator": operator, "left": left, "right": right});
    let none = |violation: Value| {
        json!({"kind": "aggregate", "function": "none", "over": {"kind": "measured", "name": "recesses"},
            "value": violation})
    };
    // Up to 1 m deep, at least 1 m wide; deeper, at least as wide as deep.
    let by_table = none(json!({"kind": "if", "branches": [{
        "when": compare("lessThanOrEquals", field("depth"), m(1.0)),
        "then": compare("lessThan", field("width"), m(1.0))}],
        "else": compare("lessThan", field("width"), field("depth"))}));
    let only_deep = none(json!({"kind": "and", "operands": [
        compare("greaterThan", field("depth"), m(1.0)),
        compare("lessThan", field("width"), m(3.0))]}));
    let deep_table = ParameterValue::Table {
        value: vec![row(&[
            ("minimum_depth_metres", 1.0),
            ("minimum_width_metres", 3.0),
        ])],
    };
    let cases: Vec<(Vec<(&str, Found)>, ParameterValue, Value)> = vec![
        (
            vec![
                (
                    "a",
                    Ok(vec![(point(1.2), point(0.8)), (point(0.9), point(0.8))]),
                ),
                (
                    "b",
                    Ok(vec![(point(2.5), point(2.0)), (point(1.5), point(2.0))]),
                ),
                ("c", Ok(Vec::new())),
            ],
            table(),
            by_table.clone(),
        ),
        (
            vec![("a", Ok(vec![(point(0.5), point(1.0))]))],
            deep_table,
            only_deep,
        ),
        (
            vec![
                ("a", Ok(vec![(point(5.0), (0.9, 1.1))])),
                ("b", Ok(vec![((0.9, 1.1), point(0.5))])),
            ],
            table(),
            by_table.clone(),
        ),
        (vec![("a", Err("tessellated".into()))], table(), by_table),
    ];
    for (index, (recesses, requirements, requirement)) in cases.into_iter().enumerate() {
        let expected = evaluate(recesses.clone(), requirements);
        let mut model = Model::default();
        for (local, _) in &recesses {
            model = model.object(local, "space");
        }
        let rule = rule(
            "axioval:capability.expression",
            kind("space"),
            vec![(
                "requirement",
                ParameterValue::Expression {
                    value: serde_json::from_value(requirement).unwrap(),
                },
            )],
        );
        let outcome =
            model.evaluate_measured(&axioval_rules::ExpressionRequirement, &rule, |services| {
                let service = Recesses(
                    recesses
                        .iter()
                        .map(|(local, found)| (id(local), found.clone()))
                        .collect(),
                );
                services
                    .register(PlanSpanServiceHandle::new(Arc::new(service)))
                    .unwrap();
            });
        let parity =
            axioval_rules::parity::compare_evaluations((ID, &expected), ("expression", &outcome));
        if index == 2 {
            // A depth either side of the row boundary leaves the row open,
            // but 5 m is wide enough under either: the expression decides.
            assert_eq!(
                parity.differences,
                vec![axioval_rules::parity::Difference {
                    scope: id("a").into(),
                    capability: Some(axioval_rules::parity::Outcome::NotEvaluated {
                        reason: NotEvaluatedReason::IncompleteEvidence,
                    }),
                    expression: None,
                    details: vec![],
                }],
                "case {index}"
            );
        } else {
            assert!(parity.holds(), "case {index}:\n{}", parity.diff());
        }
    }
}

/// Generated spaces of random recesses (exact and inexact widths and
/// depths, some spaces unmeasured) under random requirement rows, each
/// held to the implementation the template replaced.
mod generated {
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::{Found, ParameterValue, evaluate, row};

    /// A length in centimetres, exact or widened by a few.
    fn interval() -> impl Strategy<Value = (f64, f64)> {
        (0u32..400, 0u32..3).prop_map(|(low, slack)| {
            let low = f64::from(low) / 100.0;
            (low, low + f64::from(slack) / 100.0)
        })
    }

    fn found() -> impl Strategy<Value = Found> {
        prop_oneof![
            8 => vec((interval(), interval()), 0..4).prop_map(Ok),
            1 => Just(Err("tessellated".to_owned())),
            1 => Just(Err("inexact".to_owned())),
        ]
    }

    /// One row: an optional depth range and the width it requires.
    fn requirement() -> impl Strategy<Value = Vec<(&'static str, f64)>> {
        (
            proptest::option::of(0u32..300),
            proptest::option::of(1u32..300),
            proptest::option::of(0u32..300),
            proptest::option::of(0u32..30),
        )
            .prop_filter_map("a row requires a width", |(low, high, width, per)| {
                if width.is_none() && per.is_none() {
                    return None;
                }
                let mut cells = Vec::new();
                if let Some(low) = low {
                    cells.push(("minimum_depth_metres", f64::from(low) / 100.0));
                }
                if let Some(high) = high {
                    if low.is_some_and(|low| low >= high) {
                        return None;
                    }
                    cells.push(("maximum_depth_metres", f64::from(high) / 100.0));
                }
                if let Some(width) = width {
                    cells.push(("minimum_width_metres", f64::from(width) / 100.0));
                }
                if let Some(per) = per {
                    cells.push(("minimum_width_per_depth", f64::from(per) / 10.0));
                }
                Some(cells)
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn generated_recesses_hold_parity(
            spaces in vec(found(), 1..4),
            rows in vec(requirement(), 1..4),
        ) {
            let names = ["a", "b", "c"];
            let recesses = names.iter().copied().zip(spaces).collect();
            let table = ParameterValue::Table {
                value: rows.iter().map(|cells| row(cells)).collect(),
            };
            // `evaluate` holds the template to the reference.
            evaluate(recesses, table);
        }
    }
}
