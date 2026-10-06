//! Template features no rebuilt capability uses yet, held to what their
//! documentation says on small templates of their own: two member
//! populations, and a ratio whose denominator may be zero.
#![allow(missing_docs)]

mod common;

use axioval_engine::template::{
    Decision, Derived, Form, Members, Operand, Template, TemplateValue, Term, UndecidedMembers,
};
use axioval_engine::{ParameterDescriptor, ParameterType};
use axioval_ir::NotEvaluatedReason;
use axioval_ir::contract::{AggregateFunction, Expression, ParameterValue, Selector};
use axioval_rules::templates::{ForkError, Templated, fork};
use common::{Model, findings, kind, number, rule, selector, string, unevaluated};

const ID: &str = "test:window-share";

fn count(name: &'static str, selector: &str) -> TemplateValue {
    TemplateValue {
        name,
        expression: Expression::Aggregate {
            function: AggregateFunction::Count,
            over: Members::source(selector),
            filter: None,
            value: None,
            label: None,
        },
        expect: None,
        absent: None,
        mismatch: None,
    }
}

/// Windows per wall of each room: two populations along one relationship,
/// their counts' ratio within `minimum` and `maximum`.
fn template() -> Template {
    Template {
        id: ID,
        parameters: vec![
            ParameterDescriptor::required("numerator_selector", ParameterType::Selector),
            ParameterDescriptor::optional("denominator_selector", ParameterType::Selector),
            ParameterDescriptor::optional("minimum", ParameterType::Number),
            ParameterDescriptor::optional("maximum", ParameterType::Number),
        ]
        .into_iter()
        .chain([ParameterDescriptor::optional(
            "relationship",
            ParameterType::String,
        )])
        .collect(),
        grades: false,
        name: "window-share",
        defaults: Vec::new(),
        declaration: Vec::new(),
        services: None,
        texts: Vec::new(),
        forms: vec![Form {
            when: &[],
            values: vec![
                count("windows", "numerator_selector"),
                count("walls", "denominator_selector"),
            ],
            decision: Decision::Within {
                value: "share",
                minimum: Some(vec![Term::plus(Operand::Parameter("minimum"))]),
                maximum: Some(vec![Term::plus(Operand::Parameter("maximum"))]),
                rounding: Vec::new(),
            },
            fail: "{windows:least} window(s) to {walls:least} wall(s); required {required}",
            undecided: "the share straddles {bound:plain}",
            members: Some(Members {
                selector: "numerator_selector",
                undecided: UndecidedMembers::Widen,
                every_when_unstated: false,
                same_ends: None,
                more: &["denominator_selector"],
            }),
            table: None,
            scope: None,
            derived: vec![Derived::Ratio {
                name: "share",
                numerator: "windows",
                denominator: "walls",
                zero: "the room has no wall {relation}",
            }],
        }],
    }
}

fn rooms() -> Model {
    Model::default()
        .object("r1", "room")
        .object("r2", "room")
        .object("r3", "room")
        .object("w1", "window")
        .object("w2", "window")
        .object("x1", "wall")
        .object("x2", "wall")
        .object("x3", "wall")
        .edge("bounds", "r1", "w1")
        .edge("bounds", "r1", "w2")
        .edge("bounds", "r1", "x1")
        .edge("bounds", "r2", "x2")
        .edge("bounds", "r2", "x3")
}

fn parameters(extra: Vec<(&'static str, ParameterValue)>) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("numerator_selector", selector(kind("window"))),
        ("denominator_selector", selector(kind("wall"))),
        ("relationship", string("bounds")),
    ];
    parameters.extend(extra);
    parameters
}

/// Each population is counted on its own along the same relationship, and
/// their ratio decides; a room without a wall cannot be divided by.
#[test]
fn two_populations_stand_in_a_ratio() {
    let templated = Templated::new(template());
    let evaluation = rooms().evaluate(
        &templated,
        &rule(ID, kind("room"), parameters(vec![("maximum", number(1.0))])),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "r1".to_owned(),
            "2 window(s) to 1 wall(s); required at most 1".to_owned()
        )]
    );
    assert_eq!(
        evaluation.findings()[0].related.len(),
        3,
        "the windows and the wall"
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("r3".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].message(),
        "the room has no wall via bounds"
    );
}

/// A denominator that may be zero leaves the ratio without an upper bound
/// (where the evaluator's division would refuse): a minimum it surely
/// exceeds still passes, a maximum it may exceed straddles.
#[test]
fn a_denominator_that_may_be_zero_has_no_upper_bound() {
    let templated = Templated::new(template());
    // Whether r1's one wall is external cannot be read: its two windows
    // stand to zero or one external wall.
    let external = Selector::property(
        Some("Pset".into()),
        "External",
        axioval_ir::contract::ComparisonOperator::Equals,
        Some(common::boolean(true)),
    );
    let judge = |extra: Vec<(&'static str, ParameterValue)>| {
        let mut declared = parameters(extra);
        declared[1] = ("denominator_selector", selector(external.clone()));
        rooms()
            .unreadable("x1")
            .evaluate(&templated, &rule(ID, kind("room"), declared))
    };
    let at_least = judge(vec![("minimum", number(1.0))]);
    assert!(
        unevaluated(&at_least).iter().all(|(room, _)| room != "r1"),
        "{:?}",
        unevaluated(&at_least)
    );
    assert!(findings(&at_least).iter().all(|(room, _)| room != "r1"));
    let at_most = judge(vec![("maximum", number(5.0))]);
    assert!(
        at_most
            .not_evaluated_outcomes()
            .iter()
            .any(|outcome| outcome.message() == "the share straddles at most 5"),
        "{:?}",
        at_most.not_evaluated_outcomes()
    );
}

/// A derived value has no expression form the evaluator decides alike.
#[test]
fn a_derived_value_is_not_forked() {
    let templated = Templated::new(template());
    assert!(matches!(
        fork(
            &templated,
            &rule(ID, kind("room"), parameters(vec![("maximum", number(1.0))]))
        ),
        Err(ForkError::Inexpressible(_))
    ));
}
