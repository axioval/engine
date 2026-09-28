//! Classifications a ruleset derives: ordered rows assigning class names,
//! read as the reserved `axioval:classification` property by every selector
//! and capability.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use axioval_engine::{CapabilityRegistry, EngineError, compile, compile_rulesets};
use axioval_ir::{NotEvaluatedReason, PropertyValue, Report, RuleSetPackage, Scope};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, rule, ruleset, run, session};
use common::{Model, id};
use serde_json::{Value, json};

const KEYED: &str = "axioval:capability.keyed-limit";
const UNCLASSIFIED: &str = "axioval:capability.unclassified-object";
const EXISTS: &str = "axioval:capability.property-exists";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn like(pattern: &str) -> Value {
    json!({ "kind": "property", "property": "t.Name", "operator": "like",
            "value": { "type": "string", "value": pattern } })
}

/// Space use: offices first, then labs.
fn space_use(mode: &str) -> Value {
    json!({
        "id": "space-use",
        "name": { "default": "Space use", "translations": {} },
        "mode": mode,
        "rows": [
            { "selector": like("Office*"), "class": "office" },
            { "selector": like("*Lab*"), "class": "lab" },
        ],
    })
}

/// `office` (9 m²), `store` matching no row, `unsure` whose name is a list
/// the first row compares without a quantifier, and `both` (an office lab,
/// 30 m²).
fn spaces() -> Model {
    Model::default()
        .object("office", "space")
        .object("store", "space")
        .object("unsure", "space")
        .object("both", "space")
        .text("office", "Pset", "Name", "Office 1")
        .text("store", "Pset", "Name", "Store")
        .value(
            "unsure",
            "Pset",
            "Name",
            PropertyValue::List(vec![PropertyValue::String("Office 2".into())]),
        )
        .text("both", "Pset", "Name", "Office Lab")
        .value("office", "Pset", "Area", PropertyValue::Decimal(9.0))
        .value("store", "Pset", "Area", PropertyValue::Decimal(9.0))
        .value("unsure", "Pset", "Area", PropertyValue::Decimal(9.0))
        .value("both", "Pset", "Area", PropertyValue::Decimal(30.0))
}

fn classified(rules: Vec<Value>, classification: Value) -> RuleSetPackage {
    let mut set = ruleset(rules);
    set.classifications = serde_json::from_value(json!({ "space-use": classification })).unwrap();
    set
}

/// Offices need 10 m², labs 20 m², keyed on the derived space use.
fn limits() -> Value {
    rule(
        "area",
        KEYED,
        "error",
        entity("space"),
        json!({
            "key_1": { "type": "propertyReference", "propertySet": "axioval:classification",
                       "property": "space-use" },
            "quantity": { "type": "string", "value": "property" },
            "quantity_property": { "type": "propertyReference", "property": "t.Area" },
            "limits": { "type": "table", "value": [
                { "key_1": { "type": "string", "value": "office" },
                  "minimum": { "type": "number", "value": 10.0 } },
                { "key_1": { "type": "string", "value": "lab" },
                  "minimum": { "type": "number", "value": 20.0 } },
            ] },
        }),
        json!({}),
    )
}

fn unclassified() -> Value {
    rule(
        "classified",
        UNCLASSIFIED,
        "warning",
        entity("space"),
        json!({ "classification": { "type": "string", "value": "space-use" } }),
        json!({}),
    )
}

fn check(set: &RuleSetPackage) -> Result<Report, EngineError> {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[KEYED, UNCLASSIFIED, EXISTS],
        &["space"],
        &["Name", "Area"],
        &[],
    );
    let plan = compile(&registry, &[definitions], set)?;
    run(registry, plan, &session(spaces()), |runtime| runtime)
}

fn found(report: &Report, rule: &str) -> Vec<(String, String)> {
    report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(|finding| (common::subject(finding), finding.message.clone()))
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

#[test]
fn a_keyed_limit_keys_its_table_on_a_derived_class() {
    let report = check(&classified(
        vec![limits(), unclassified()],
        space_use("firstMatch"),
    ))
    .unwrap();
    // The office gets the office limit; the office lab is an office first.
    let area = found(&report, "area");
    assert_eq!(area.len(), 1, "{area:?}");
    assert_eq!(area[0].0, "office");
    // The store matches no row: it is reported unclassified, and has no key.
    assert_eq!(
        found(&report, "classified")
            .into_iter()
            .map(|(subject, _)| subject)
            .collect::<Vec<_>>(),
        ["store"]
    );
    assert!(found(&report, "classified")[0].1.contains("unclassified"));
    // The unsure space's first row is undecided: not evaluated by either.
    assert!(open(&report, "classified").contains(&(
        Scope::Object(id("unsure")),
        NotEvaluatedReason::InvalidEvidence
    )));
    assert!(
        open(&report, "area")
            .iter()
            .any(|(scope, _)| *scope == Scope::Object(id("unsure")))
    );
    assert!(
        open(&report, "area")
            .iter()
            .any(|(scope, _)| *scope == Scope::Object(id("store")))
    );
}

#[test]
fn every_selector_reads_a_derived_class() {
    let offices = rule(
        "offices-named",
        EXISTS,
        "error",
        json!({ "kind": "property", "propertySet": "axioval:classification",
                "property": "space-use", "operator": "equals",
                "value": { "type": "string", "value": "office" } }),
        json!({ "property": { "type": "propertyReference", "property": "t.Missing" } }),
        json!({}),
    );
    let registry = registry();
    let definitions = definitions(&registry, &[EXISTS], &["space"], &["Name", "Missing"], &[]);
    let set = classified(vec![offices], space_use("firstMatch"));
    let plan = compile(&registry, &[definitions], &set).unwrap();
    let report = run(registry, plan, &session(spaces()), |runtime| runtime).unwrap();
    let mut subjects: Vec<String> = found(&report, "offices-named")
        .into_iter()
        .map(|(subject, _)| subject)
        .collect();
    subjects.sort();
    assert_eq!(subjects, ["both", "office"]);
    assert_eq!(
        open(&report, "offices-named"),
        [(
            Scope::Object(id("unsure")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

#[test]
fn an_all_match_class_is_a_list_of_every_matching_class() {
    let labs = rule(
        "labs",
        EXISTS,
        "error",
        json!({ "kind": "property", "propertySet": "axioval:classification",
                "property": "space-use", "operator": "equals", "quantifier": "any",
                "value": { "type": "string", "value": "lab" } }),
        json!({ "property": { "type": "propertyReference", "property": "t.Missing" } }),
        json!({}),
    );
    let registry = registry();
    let definitions = definitions(&registry, &[EXISTS], &["space"], &["Name", "Missing"], &[]);
    let set = classified(vec![labs], space_use("allMatch"));
    let plan = compile(&registry, &[definitions], &set).unwrap();
    let report = run(registry, plan, &session(spaces()), |runtime| runtime).unwrap();
    assert_eq!(
        found(&report, "labs")
            .into_iter()
            .map(|(subject, _)| subject)
            .collect::<Vec<_>>(),
        ["both"]
    );
}

mod compilation {
    use super::*;

    fn compiled(set: &RuleSetPackage) -> Result<(), EngineError> {
        let registry = registry();
        let definitions = definitions(
            &registry,
            &[KEYED, UNCLASSIFIED, EXISTS],
            &["space"],
            &["Name", "Area"],
            &[],
        );
        compile(&registry, &[definitions], set).map(|_| ())
    }

    #[test]
    fn a_reference_to_an_undeclared_classification_is_refused() {
        assert!(matches!(
            compiled(&ruleset(vec![limits()])),
            Err(EngineError::UnknownConcept { .. })
        ));
    }

    #[test]
    fn malformed_classifications_are_refused() {
        let mut blank = space_use("firstMatch");
        blank["rows"][0]["class"] = json!(" ");
        let mut empty = space_use("firstMatch");
        empty["rows"] = json!([]);
        let mut outcome = space_use("firstMatch");
        outcome["rows"][0]["selector"] =
            json!({ "kind": "ruleOutcome", "rule": "area", "outcome": "failed" });
        let mut cyclic = space_use("firstMatch");
        cyclic["rows"][0]["selector"] = json!({ "kind": "property",
            "propertySet": "axioval:classification", "property": "space-use", "operator": "exists" });
        for classification in [blank, empty, outcome, cyclic] {
            assert!(matches!(
                compiled(&classified(vec![limits()], classification)),
                Err(EngineError::InvalidClassification { .. })
            ));
        }
    }

    #[test]
    fn rulesets_share_a_classification_only_declared_alike() {
        let registry = registry();
        let definitions = definitions(
            &registry,
            &[KEYED, UNCLASSIFIED, EXISTS],
            &["space"],
            &["Name", "Area"],
            &[],
        );
        let package = |id: &str, mode: &str| {
            let mut set = classified(vec![limits()], space_use(mode));
            set.package.id = id.into();
            set
        };
        assert!(
            compile_rulesets(
                &registry,
                std::slice::from_ref(&definitions),
                &[package("p1", "firstMatch"), package("p2", "firstMatch")],
            )
            .is_ok()
        );
        assert!(matches!(
            compile_rulesets(
                &registry,
                &[definitions],
                &[package("p1", "firstMatch"), package("p2", "allMatch")],
            ),
            Err(EngineError::InvalidClassification { .. })
        ));
    }
}
