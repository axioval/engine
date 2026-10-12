//! `pairs` in rules: consecutive members of an ordered sequence, read by
//! the expression capability, its findings naming and relating the pairs
//! that decide them.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityRegistry, EvidenceSession};
use axioval_ir::contract::PropertyValueKind;
use axioval_ir::{DefinitionPackage, NotEvaluatedReason, PropertyValue, QuantityDimension, Report};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, plan, rule, run, session};
use common::{Model, id};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Definitions where a riser's `Rise` and the `Level` it starts at are
/// lengths.
fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION],
        &["slab", "pipe"],
        &["Rise", "Level"],
        &["Pset"],
    );
    for name in ["t.Rise", "t.Level"] {
        let property = package.properties.get_mut(name).unwrap();
        property.value_kind = PropertyValueKind::Quantity;
        property.unit_dimension = Some("length".into());
    }
    package
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": format!("t.{name}")})
}

fn mm(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "mm"}})
}

fn metres(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

/// A flight `f1` (a slab) hosting risers (pipes) at `levels` with `rises`,
/// in metres; a level of `None` is measured within the interval given.
fn flight(risers: &[(&str, PropertyValue, f64)]) -> Model {
    let mut model = Model::default().object("f1", "slab");
    for (riser, level, rise) in risers {
        model = model
            .object(riser, "pipe")
            .value(riser, "Pset", "Level", level.clone())
            .value(riser, "Pset", "Rise", metres(*rise))
            .edge("Hosts", "f1", riser);
    }
    model
}

/// Consecutive risers of the flight, ordered by level, differ by at most
/// 5 mm.
fn consecutive_rule() -> Value {
    let difference = json!({"kind": "abs", "operand": {"kind": "subtract",
        "left": property("Rise"), "right": {"kind": "previous", "operand": property("Rise")}}});
    rule(
        "consecutive",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": {"kind": "pairs",
            "function": "all", "label": "consecutive risers",
            "over": {"kind": "path", "path": ["Hosts"]}, "by": property("Level"),
            "value": {"kind": "compare", "operator": "lessThanOrEquals",
                "left": difference, "right": mm(5.0)}}}}),
        json!({}),
    )
}

fn check(package: &DefinitionPackage, rules: Vec<Value>, session: &EvidenceSession) -> Report {
    let registry = registry();
    let plan = plan(&registry, package, rules).unwrap();
    run(registry, plan, session, |runtime| runtime).unwrap()
}

/// #350: risers of 172, 175 and 181 mm with a 5 mm limit on consecutive
/// differences: the flight's finding is at the pair 175 → 181, naming and
/// relating both risers and neither the first.
#[test]
fn consecutive_risers_differing_by_more_than_the_limit_are_named() {
    let registry = registry();
    let package = vocabulary(&registry);
    // Listed out of order: the order is the levels'.
    let model = flight(&[
        ("r3", metres(0.347), 0.181),
        ("r1", metres(0.0), 0.172),
        ("r2", metres(0.172), 0.175),
    ]);
    let report = check(&package, vec![consecutive_rule()], &session(model));
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    let findings = report.findings();
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(common::subject(finding), "f1");
    assert_eq!(finding.related, [id("r2"), id("r3")], "{}", finding.message);
    let pair = format!("pairs {} → {}", id("r2"), id("r3"));
    assert!(
        finding.message.contains(&pair) && !finding.message.contains(&id("r1").to_string()),
        "{}",
        finding.message
    );
    // Within the limit, the flight passes.
    let even = flight(&[
        ("r1", metres(0.0), 0.172),
        ("r2", metres(0.172), 0.175),
        ("r3", metres(0.347), 0.177),
    ]);
    let report = check(&package, vec![consecutive_rule()], &session(even));
    assert!(report.findings().is_empty() && report.not_evaluated.is_empty());
}

/// Levels measured only within intervals that may come either way round
/// leave the flight open, never passed, though every listed pair holds.
#[test]
fn an_undecided_order_leaves_the_flight_open() {
    let registry = registry();
    let package = vocabulary(&registry);
    let level = |lower: f64, upper: f64| PropertyValue::Measured {
        lower,
        upper,
        dimension: Some(QuantityDimension::Length),
    };
    let model = flight(&[
        ("r1", metres(0.0), 0.172),
        ("r2", level(0.17, 0.18), 0.175),
        ("r3", level(0.175, 0.185), 0.177),
    ]);
    let report = check(&package, vec![consecutive_rule()], &session(model));
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated.len(), 1, "{:?}", report.not_evaluated);
    let open = &report.not_evaluated[0];
    assert_eq!(open.reason, NotEvaluatedReason::IncompleteEvidence);
    assert!(
        open.message.contains("undecided by their order"),
        "{}",
        open.message
    );
}
