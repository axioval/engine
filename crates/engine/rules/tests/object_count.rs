//! `object-count`: existence and cardinality reported against a source or
//! the project, never against an object.
#![allow(missing_docs)]

mod common;

use axioval_ir::contract::{ComparisonOperator, Selector};
use axioval_ir::{NotEvaluatedReason, Scope, SourceId};
use axioval_rules::{ObjectCount, register_builtins};
use common::{Model, boolean, findings, integer, kind, rule, source};

const ID: &str = "axioval:capability.object-count";

fn other() -> SourceId {
    SourceId::new("test", "other").unwrap()
}

/// Two walls and a slab in the default source, one wall in `other`.
fn model() -> Model {
    Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .object("s1", "slab")
        .object_in("other", "w9", "wall")
}

#[test]
fn an_empty_selection_is_reported_none_found_instead_of_passing() {
    let evaluation = model().evaluate(&ObjectCount, &rule(ID, kind("building"), vec![]));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "source".into(),
                "no object matches the selection in source `test:model`; required at least 1"
                    .into()
            ),
            (
                "source".into(),
                "no object matches the selection in source `test:other`; required at least 1"
                    .into()
            ),
        ]
    );
    let scopes: Vec<&Scope> = evaluation
        .findings()
        .iter()
        .map(|finding| &finding.scope)
        .collect();
    assert_eq!(scopes, [&Scope::Source(source()), &Scope::Source(other())]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn across_sources_the_project_is_one_scope() {
    let evaluation = model().evaluate(
        &ObjectCount,
        &rule(
            ID,
            kind("building"),
            vec![("across_sources", boolean(true))],
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "project".into(),
            "no object matches the selection in the project; required at least 1".into()
        )]
    );
    assert_eq!(evaluation.findings()[0].scope, Scope::Project);

    // Three walls in the project satisfy "at least 3" even though no
    // single source has three.
    let evaluation = model().evaluate(
        &ObjectCount,
        &rule(
            ID,
            kind("wall"),
            vec![("across_sources", boolean(true)), ("minimum", integer(3))],
        ),
    );
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn bounds_are_inclusive_and_name_what_was_found() {
    let evaluation = model().evaluate(
        &ObjectCount,
        &rule(
            ID,
            kind("wall"),
            vec![("minimum", integer(1)), ("maximum", integer(1))],
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "source".into(),
            "2 object(s) match the selection in source `test:model`; required exactly 1".into()
        )]
    );
    let related: Vec<&str> = evaluation.findings()[0]
        .related
        .iter()
        .map(|id| id.local_id.as_str())
        .collect();
    assert_eq!(related, ["w1", "w2"]);

    let evaluation = model().evaluate(
        &ObjectCount,
        &rule(ID, kind("wall"), vec![("maximum", integer(0))]),
    );
    assert_eq!(evaluation.findings().len(), 2);

    // A maximum alone allows an empty selection.
    let evaluation = model().evaluate(
        &ObjectCount,
        &rule(ID, kind("building"), vec![("maximum", integer(1))]),
    );
    assert!(evaluation.findings().is_empty());
}

fn fire_rated() -> Selector {
    Selector::property(
        Some("Pset".into()),
        "FireRated",
        ComparisonOperator::Equals,
        Some(boolean(true)),
    )
}

#[test]
fn undecided_objects_leave_the_scope_not_evaluated_only_when_they_matter() {
    let fire_rated = fire_rated();
    let walls = || {
        Model::default()
            .object("w1", "wall")
            .object("w2", "wall")
            .value(
                "w1",
                "Pset",
                "FireRated",
                axioval_ir::PropertyValue::Boolean(true),
            )
            .unreadable("w2")
    };

    // One fire-rated wall is found; the unreadable one cannot matter.
    let evaluation = walls().evaluate(&ObjectCount, &rule(ID, fire_rated.clone(), vec![]));
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());

    // For "at least 2" it can: the source and the wall are not evaluated.
    let evaluation = walls().evaluate(
        &ObjectCount,
        &rule(ID, fire_rated, vec![("minimum", integer(2))]),
    );
    assert!(evaluation.findings().is_empty());
    let outcomes: Vec<(&Scope, &NotEvaluatedReason)> = evaluation
        .not_evaluated_outcomes()
        .iter()
        .map(|outcome| (outcome.scope(), outcome.reason()))
        .collect();
    assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    assert!(matches!(outcomes[0].0, Scope::Object(id) if id.local_id == "w2"));
    assert_eq!(
        outcomes[1],
        (
            &Scope::Source(source()),
            &NotEvaluatedReason::IncompleteEvidence
        )
    );
}

#[test]
fn a_count_finding_cites_the_facts_that_selected_its_objects() {
    let fire_rated = fire_rated();
    let evaluation = Model::default()
        .object("w1", "wall")
        .object("w2", "wall")
        .value(
            "w1",
            "Pset",
            "FireRated",
            axioval_ir::PropertyValue::Boolean(true),
        )
        .value(
            "w2",
            "Pset",
            "FireRated",
            axioval_ir::PropertyValue::Boolean(false),
        )
        .evaluate(
            &ObjectCount,
            &rule(ID, fire_rated, vec![("minimum", integer(2))]),
        );
    let finding = &evaluation.findings()[0];
    assert_eq!(finding.scope, Scope::Source(source()));
    let locators: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert_eq!(locators, ["test:model/w1:Pset.FireRated"]);
    assert!(finding.evidence.iter().all(|evidence| evidence.exact));
}

#[test]
fn per_source_over_an_empty_project_is_not_evaluated() {
    let evaluation = Model::default().evaluate(&ObjectCount, &rule(ID, kind("building"), vec![]));
    assert!(evaluation.findings().is_empty());
    let outcome = &evaluation.not_evaluated_outcomes()[0];
    assert_eq!(outcome.scope(), &Scope::Project);
    assert_eq!(outcome.reason(), &NotEvaluatedReason::IncompleteEvidence);
}

#[test]
fn contradictory_bounds_are_a_declaration_error() {
    for parameters in [
        vec![("minimum", integer(2)), ("maximum", integer(1))],
        vec![("minimum", integer(-1))],
        vec![("maximum", integer(-1))],
    ] {
        let evaluation = model().evaluate(&ObjectCount, &rule(ID, kind("wall"), parameters));
        assert_eq!(
            evaluation.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

#[test]
fn object_count_is_a_builtin() {
    let registry = register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    assert!(registry.get(ID).is_some());
}
