//! Rules gated on other rules' outcomes, compiled from packages and run end
//! to end: the parent runs first, and what it left undecided stays
//! undecided for the rules that read it.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityRegistry, EngineError, compile};
use axioval_ir::{NotEvaluatedReason, PropertyValue, Report, RuleStatus, RuleSummary, Scope};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, ruleset, run, session};
use common::{Model, id};
use serde_json::{Value, json};

const EXISTS: &str = "axioval:capability.property-exists";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Three doors: `d1` has a fire rating, `d2` has none, and whether `d3` is
/// a door the parent selects cannot be decided (its kind is a list the
/// parent's selector compares without a quantifier). No door has hardware.
fn doors() -> Model {
    Model::default()
        .object("d1", "door")
        .object("d2", "door")
        .object("d3", "door")
        .text("d1", "Pset", "Kind", "door")
        .text("d2", "Pset", "Kind", "door")
        .value(
            "d3",
            "Pset",
            "Kind",
            PropertyValue::List(vec![PropertyValue::String("door".into())]),
        )
        .text("d1", "Pset", "FireRating", "F30")
}

fn property(name: &str) -> Value {
    json!({ "property": { "type": "propertyReference", "property": format!("t.{name}") } })
}

/// The parent: every door the parent's selector selects has a fire rating.
fn parent(id: &str) -> Value {
    rule(
        id,
        EXISTS,
        "error",
        json!({ "kind": "allOf", "operands": [
            entity("door"),
            { "kind": "property", "property": "t.Kind", "operator": "equals",
              "value": { "type": "string", "value": "door" } },
        ] }),
        property("FireRating"),
        json!({}),
    )
}

/// A child: every door has hardware, gated as `gate` says.
fn child(id: &str, gate: Value) -> Value {
    rule(
        id,
        EXISTS,
        "error",
        entity("door"),
        property("Hardware"),
        gate,
    )
}

fn gate(parent: &str, condition: &str) -> Value {
    json!({ "gate": { "rule": parent, "condition": condition } })
}

fn check(rules: Vec<Value>, model: Model) -> Report {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[EXISTS],
        &["door"],
        &["Kind", "FireRating", "Hardware"],
        &[],
    );
    let plan = plan(&registry, &definitions, rules).unwrap();
    run(registry, plan, &session(model), |runtime| {
        runtime.with_rule_summaries()
    })
    .unwrap()
}

fn found(report: &Report, rule: &str) -> Vec<String> {
    report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(common::subject)
        .collect()
}

fn open(report: &Report, rule: &str) -> Vec<(Scope, NotEvaluatedReason)> {
    report
        .not_evaluated
        .iter()
        .filter(|outcome| outcome.rule_id.to_string() == rule)
        .map(|outcome| (outcome.scope.clone(), outcome.reason.clone()))
        .collect()
}

fn status(report: &Report, rule: &str) -> RuleStatus {
    report
        .rules()
        .iter()
        .find(|summary| summary.rule_id.to_string() == rule)
        .map(|summary: &RuleSummary| summary.status)
        .unwrap()
}

#[test]
fn the_parent_fails_one_door_and_leaves_one_undecided() {
    let report = check(vec![parent("type")], doors());
    assert_eq!(found(&report, "type"), ["d2"]);
    assert_eq!(
        open(&report, "type"),
        [(Scope::Object(id("d3")), NotEvaluatedReason::InvalidEvidence)]
    );
}

#[test]
fn a_child_gated_on_failed_objects_reports_only_on_doors_the_parent_failed() {
    // The child sorts before its parent: the plan runs the parent first.
    let report = check(
        vec![
            parent("type"),
            child("a-hardware", gate("type", "failedObjects")),
        ],
        doors(),
    );
    assert_eq!(found(&report, "a-hardware"), ["d2"]);
    // The parent did not evaluate d3, so neither does the child.
    assert_eq!(
        open(&report, "a-hardware"),
        [(
            Scope::Object(id("d3")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

#[test]
fn a_child_gated_on_passed_objects_reports_only_on_doors_the_parent_passed() {
    let report = check(
        vec![
            parent("type"),
            child("hardware", gate("type", "passedObjects")),
        ],
        doors(),
    );
    assert_eq!(found(&report, "hardware"), ["d1"]);
    assert_eq!(
        open(&report, "hardware"),
        [(
            Scope::Object(id("d3")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

#[test]
fn a_rule_outcome_selector_selects_as_an_object_gate_does() {
    let selector = rule(
        "hardware",
        EXISTS,
        "error",
        json!({ "kind": "ruleOutcome", "rule": "type", "outcome": "failed" }),
        property("Hardware"),
        json!({}),
    );
    let report = check(vec![parent("type"), selector], doors());
    assert_eq!(found(&report, "hardware"), ["d2"]);
    assert_eq!(open(&report, "hardware").len(), 1);
}

#[test]
fn a_child_gated_on_all_if_passed_is_skipped_when_the_parent_fails_anywhere() {
    let report = check(
        vec![
            parent("type"),
            child("hardware", gate("type", "allIfPassed")),
        ],
        doors(),
    );
    assert!(found(&report, "hardware").is_empty());
    assert!(open(&report, "hardware").is_empty());
    assert_eq!(status(&report, "hardware"), RuleStatus::Skipped);
}

#[test]
fn a_child_gated_on_all_if_failed_checks_every_door_when_the_parent_fails() {
    let report = check(
        vec![
            parent("type"),
            child("hardware", gate("type", "allIfFailed")),
        ],
        doors(),
    );
    assert_eq!(found(&report, "hardware"), ["d1", "d2", "d3"]);
}

#[test]
fn a_parent_passing_everywhere_opens_all_if_passed_and_closes_all_if_failed() {
    let rated = Model::default()
        .object("d1", "door")
        .text("d1", "Pset", "Kind", "door")
        .text("d1", "Pset", "FireRating", "F30");
    let report = check(
        vec![
            parent("type"),
            child("if-passed", gate("type", "allIfPassed")),
            child("if-failed", gate("type", "allIfFailed")),
        ],
        rated,
    );
    assert_eq!(found(&report, "if-passed"), ["d1"]);
    assert_eq!(status(&report, "if-failed"), RuleStatus::Skipped);
}

#[test]
fn a_parent_leaving_doors_open_without_a_finding_leaves_a_whole_gate_undecided() {
    // Only d3 remains: the parent finds nothing and decides nothing.
    let undecided = Model::default().object("d3", "door").value(
        "d3",
        "Pset",
        "Kind",
        PropertyValue::List(vec![PropertyValue::String("door".into())]),
    );
    let report = check(
        vec![
            parent("type"),
            child("hardware", gate("type", "allIfPassed")),
        ],
        undecided,
    );
    assert!(found(&report, "hardware").is_empty());
    assert_eq!(
        open(&report, "hardware"),
        [(Scope::Project, NotEvaluatedReason::IncompleteEvidence)]
    );
    assert_eq!(status(&report, "hardware"), RuleStatus::NotEvaluated);
}

#[test]
fn a_rule_gated_on_a_skipped_rule_is_skipped_too() {
    let report = check(
        vec![
            parent("type"),
            child("hardware", gate("type", "allIfPassed")),
            child("labels", gate("hardware", "allIfFailed")),
            child("marks", gate("hardware", "failedObjects")),
        ],
        doors(),
    );
    assert_eq!(status(&report, "labels"), RuleStatus::Skipped);
    // A skipped rule failed no object.
    assert!(found(&report, "marks").is_empty());
    assert_eq!(status(&report, "marks"), RuleStatus::NothingSelected);
}

#[test]
fn a_folder_gate_applies_to_every_rule_in_it() {
    let text = |value: &str| json!({ "default": value, "translations": {} });
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[EXISTS],
        &["door"],
        &["Kind", "FireRating", "Hardware"],
        &[],
    );
    let ruleset = serde_json::from_value(json!({
        "schemaVersion": "0.1.0",
        "package": { "id": "t.ruleset", "name": text("test"), "version": "0.1.0", "authors": [] },
        "definitionPackages": ["t.definitions"],
        "root": {
            "id": "root", "name": text("root"),
            "rules": [parent("type")],
            "folders": [{
                "id": "hardware", "name": text("hardware"),
                "gate": { "rule": "type", "condition": "failedObjects" },
                "rules": [child("hardware", json!({}))],
            }],
        },
    }))
    .unwrap();
    let plan = compile(&registry, &[definitions], &ruleset).unwrap();
    let report = run(registry, plan, &session(doors()), |runtime| runtime).unwrap();
    assert_eq!(found(&report, "hardware"), ["d2"]);
}

#[test]
fn rulesets_compiled_together_read_their_own_rules() {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[EXISTS],
        &["door"],
        &["Kind", "FireRating", "Hardware"],
        &[],
    );
    let package = |id: &str, condition: &str| {
        let mut set = ruleset(vec![
            parent("type"),
            child("hardware", gate("type", condition)),
        ]);
        set.package.id = id.into();
        set
    };
    let plan = axioval_engine::compile_rulesets(
        &registry,
        &[definitions],
        &[
            package("p1", "failedObjects"),
            package("p2", "passedObjects"),
        ],
    )
    .unwrap();
    let report = run(registry, plan, &session(doors()), |runtime| runtime).unwrap();
    assert_eq!(found(&report, "p1/hardware"), ["d2"]);
    assert_eq!(found(&report, "p2/hardware"), ["d1"]);
}

mod compilation {
    use super::*;

    fn compiled(rules: Vec<Value>) -> Result<Vec<String>, EngineError> {
        let registry = registry();
        let definitions = definitions(
            &registry,
            &[EXISTS],
            &["door"],
            &["Kind", "FireRating", "Hardware"],
            &[],
        );
        plan(&registry, &definitions, rules).map(|plan| {
            plan.rules()
                .iter()
                .map(|rule| rule.id.to_string())
                .collect()
        })
    }

    #[test]
    fn a_rule_runs_after_the_rules_it_reads() {
        let order = compiled(vec![
            child("a", gate("b", "allIfFailed")),
            child("b", gate("c", "passedObjects")),
            parent("c"),
            parent("d"),
        ])
        .unwrap();
        assert_eq!(order, ["c", "b", "a", "d"]);
    }

    #[test]
    fn a_cycle_is_refused() {
        let error = compiled(vec![
            child("a", gate("b", "allIfPassed")),
            child("b", gate("a", "failedObjects")),
        ])
        .unwrap_err();
        assert!(
            matches!(&error, EngineError::InvalidDependency { detail, .. } if detail.contains("cycle")),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_or_own_rule_is_refused() {
        for rules in [
            vec![child("a", gate("missing", "allIfPassed"))],
            vec![child("a", gate("a", "passedObjects"))],
        ] {
            assert!(matches!(
                compiled(rules),
                Err(EngineError::InvalidDependency { .. })
            ));
        }
    }

    #[test]
    fn a_rule_reading_a_disabled_rule_is_not_evaluated() {
        let mut disabled = parent("type");
        disabled["enabled"] = json!(false);
        let registry = registry();
        let definitions = definitions(
            &registry,
            &[EXISTS],
            &["door"],
            &["Kind", "FireRating", "Hardware"],
            &[],
        );
        let plan = compile(
            &registry,
            &[definitions],
            &ruleset(vec![
                disabled,
                child("hardware", gate("type", "failedObjects")),
            ]),
        )
        .unwrap();
        assert!(plan.rules().is_empty());
        let report = run(registry, plan, &session(doors()), |runtime| runtime).unwrap();
        assert_eq!(
            open(&report, "hardware"),
            [(Scope::Project, NotEvaluatedReason::InvalidDeclaration)]
        );
    }

    #[test]
    fn reading_outcomes_per_object_needs_a_refiner() {
        let mut bare = CapabilityRegistry::new();
        bare = bare.register(axioval_rules::PropertyExists).unwrap();
        let definitions = definitions(
            &registry(),
            &[EXISTS],
            &["door"],
            &["Kind", "FireRating", "Hardware"],
            &[],
        );
        let result = plan(
            &bare,
            &definitions,
            vec![
                parent("type"),
                child("hardware", gate("type", "failedObjects")),
            ],
        );
        assert!(matches!(result, Err(EngineError::InvalidDependency { .. })));
        // A whole-rule gate reads only the parent's status.
        assert!(
            plan(
                &bare,
                &definitions,
                vec![
                    parent("type"),
                    child("hardware", gate("type", "allIfFailed"))
                ],
            )
            .is_ok()
        );
    }
}
