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
            .map_err(PlanSpanError::Unavailable)?;
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

fn evaluate(
    recesses: Vec<(&str, Found)>,
    requirements: ParameterValue,
) -> axioval_engine::CapabilityEvaluation {
    let mut model = Model::default();
    for (local, _) in &recesses {
        model = model.object(local, "space");
    }
    let service = Recesses(
        recesses
            .into_iter()
            .map(|(local, found)| (id(local), found))
            .collect(),
    );
    let rule = rule(ID, kind("space"), vec![("requirements", requirements)]);
    model.evaluate_with(&RecessWidth, &rule, move |services| {
        services
            .register(PlanSpanServiceHandle::new(Arc::new(service)))
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
    let found = findings(&outcome);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].0, "a");
    assert!(
        found[0]
            .1
            .ends_with("is 0.9 m wide and 0.8 m deep; row 0 requires at least 1 m"),
        "{found:?}"
    );
    assert_eq!(found[1].0, "b");
    assert!(
        found[1].1.ends_with("row 1 requires at least 2 m"),
        "{found:?}"
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
}

#[test]
fn an_unmeasured_space_or_a_missing_service_is_not_evaluated() {
    let outcome = evaluate(vec![("a", Err("tessellated".into()))], table());
    assert_eq!(
        unevaluated(&outcome),
        vec![("a".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let rule = rule(ID, kind("space"), vec![("requirements", table())]);
    let outcome = Model::default()
        .object("a", "space")
        .evaluate(&RecessWidth, &rule);
    assert_eq!(
        unevaluated(&outcome),
        vec![("a".to_owned(), NotEvaluatedReason::MissingService)]
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
                    object: id("a"),
                    capability: Some(axioval_rules::parity::Outcome::NotEvaluated {
                        reason: NotEvaluatedReason::IncompleteEvidence,
                    }),
                    expression: None,
                }],
                "case {index}"
            );
        } else {
            assert!(parity.holds(), "case {index}:\n{}", parity.diff());
        }
    }
}
