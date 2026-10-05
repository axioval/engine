//! The differential parity harness: a capability rule and its rewrite as
//! an expression rule, run over one model, judged object by object.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityRegistry, RuleOutcomes, Runtime};
use axioval_ir::contract::PropertyValueKind;
use axioval_ir::{DefinitionPackage, PropertyValue, QuantityDimension, Report};
use axioval_rules::parity::{Observations, Outcome, Parity, compare, compare_recorded};
use axioval_rules::register_builtins;
use common::Model;
use common::runtime::{definition, definitions, entity, plan, rule, run, session};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";
const PREDICATE: &str = "axioval:capability.property-predicate";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION, PREDICATE],
        &["slab", "wall"],
        &["Cover"],
        &["Pset"],
    );
    let cover = package.properties.get_mut("t.Cover").unwrap();
    cover.value_kind = PropertyValueKind::Quantity;
    cover.unit_dimension = Some("length".into());
    assert!(package.definitions.contains_key(&definition(EXPRESSION)));
    package
}

/// Slabs with a concrete cover, one stating none, and a wall.
fn slabs() -> Model {
    let mut model = Model::default().object("w", "wall");
    for (local, cover) in [("s1", Some(0.045)), ("s2", Some(0.025)), ("s3", None)] {
        model = model.object(local, "slab");
        if let Some(cover) = cover {
            model = model.value(
                local,
                "Pset",
                "Cover",
                PropertyValue::Quantity {
                    value: cover,
                    dimension: QuantityDimension::Length,
                },
            );
        }
    }
    model
}

/// Cover at least 30 mm, as the property-predicate capability states it.
fn capability() -> Value {
    rule(
        "cover-capability",
        PREDICATE,
        "error",
        entity("slab"),
        json!({
            "property_set": {"type": "string", "value": "t.Pset"},
            "property": {"type": "string", "value": "t.Cover"},
            "operator": {"type": "string", "value": "greater_or_equal"},
            "quantity": {"type": "quantity", "value": 30.0, "unit": "mm"},
        }),
        json!({}),
    )
}

/// The same requirement rewritten as an expression; `minimum` in mm.
fn rewrite(minimum: f64) -> Value {
    rewrite_of("slab", minimum)
}

/// [`rewrite`] over the objects of `kind`.
fn rewrite_of(kind: &str, minimum: f64) -> Value {
    rule(
        "cover-expression",
        EXPRESSION,
        "error",
        entity(kind),
        json!({"requirement": {"type": "expression", "value": {
            "kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "property", "propertySet": "t.Pset", "property": "t.Cover"},
            "right": {"kind": "literal",
                "value": {"type": "quantity", "value": minimum, "unit": "mm"}}}}}),
        json!({}),
    )
}

fn check(rules: Vec<Value>) -> Report {
    let registry = registry();
    let package = vocabulary(&registry);
    let plan = plan(&registry, &package, rules).unwrap();
    run(registry, plan, &session(slabs()), |runtime| runtime).unwrap()
}

/// [`check`], every rule's selection recorded.
fn recorded(rules: Vec<Value>) -> (Report, RuleOutcomes) {
    let registry = registry();
    let package = vocabulary(&registry);
    let plan = plan(&registry, &package, rules).unwrap();
    Runtime::new(registry)
        .run_session_recorded(&session(slabs()), plan)
        .unwrap()
}

#[test]
fn an_expression_rewrite_judges_every_object_as_its_capability_does() {
    let report = check(vec![capability(), rewrite(30.0)]);
    let evidence = compare(&report, "cover-capability", "cover-expression");
    assert!(evidence.holds(), "{}", evidence.diff());
    assert_eq!(evidence.objects, 2);
    // Both find the short cover and the missing one alike.
    assert_eq!((evidence.found, evidence.open), (2, 0));
    let json = serde_json::to_value(&evidence).unwrap();
    assert_eq!(json["differences"], json!([]));
    assert_eq!(json["capability"], "cover-capability");
}

#[test]
fn a_wrong_rewrite_fails_with_a_diff_naming_the_object() {
    let report = check(vec![capability(), rewrite(50.0)]);
    let evidence = compare(&report, "cover-capability", "cover-expression");
    assert!(!evidence.holds());
    assert_eq!(evidence.differences.len(), 1);
    let difference = &evidence.differences[0];
    assert_eq!(difference.capability, None);
    assert!(matches!(
        difference.expression,
        Some(Outcome::Finding { .. })
    ));
    let diff = evidence.diff();
    assert!(diff.contains("s1"), "{diff}");
    assert!(
        diff.contains("capability reported nothing, expression finding"),
        "{diff}"
    );
}

#[test]
fn a_rule_missing_from_the_report_has_nothing_to_hold_parity_with() {
    let report = check(vec![capability()]);
    let evidence = compare(&report, "cover-capability", "cover-expression");
    assert!(!evidence.holds());
    assert!(evidence.differences.iter().all(|d| d.expression.is_none()));
}

#[test]
fn a_recorded_run_tells_passed_from_not_selected() {
    let rules = || vec![capability(), rewrite_of("wall", 30.0)];
    // Unrecorded, the slab the capability passed and the rewrite never
    // selected read alike.
    let report = check(rules());
    let unrecorded = compare(&report, "cover-capability", "cover-expression");
    assert!(!unrecorded.diff().contains("s1"), "{}", unrecorded.diff());
    let (report, outcomes) = recorded(rules());
    let evidence = compare_recorded(&report, &outcomes, "cover-capability", "cover-expression");
    let diff = evidence.diff();
    assert!(
        diff.contains("test:model/s1: capability passed, expression not selected"),
        "{diff}"
    );
    assert!(
        diff.contains("test:model/w: capability not selected, expression finding"),
        "{diff}"
    );
    // The same rewrite over slabs holds parity, every slab selected alike.
    let (report, outcomes) = recorded(vec![capability(), rewrite(30.0)]);
    let evidence = compare_recorded(&report, &outcomes, "cover-capability", "cover-expression");
    assert!(evidence.holds(), "{}", evidence.diff());
    assert_eq!(evidence.objects, 3);
    let selected = Observations::of_recorded(&report, &outcomes, "cover-expression");
    assert_eq!(
        selected.outcome(&common::id("s1").into()),
        Some(Outcome::Passed)
    );
    assert_eq!(
        selected.outcome(&common::id("w").into()),
        Some(Outcome::NotSelected)
    );
}

#[test]
fn the_contract_compares_messages_word_for_word() {
    let report = check(vec![capability(), rewrite(30.0)]);
    let contract = Parity::contract().compare(
        (
            "cover-capability",
            &Observations::of_report(&report, "cover-capability"),
        ),
        (
            "cover-expression",
            &Observations::of_report(&report, "cover-expression"),
        ),
    );
    // An expression words its own findings, so it is no template.
    assert_eq!(contract.differences.len(), 2, "{}", contract.diff());
    assert!(contract.diff().contains("messages"), "{}", contract.diff());
}
