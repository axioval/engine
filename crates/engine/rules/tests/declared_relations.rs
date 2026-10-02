//! Relations a ruleset declares between objects (`relations`), followed
//! like relationships a model states.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use axioval_engine::{CapabilityRegistry, EngineError, compile, unknown_relation_objects};
use axioval_ir::{PropertyValue, Report, RuleSetPackage, Scope};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, rule, ruleset, run, session};
use common::{Model, id};
use serde_json::{Value, json};

const COMPARISON: &str = "axioval:capability.property-comparison";
const COUNT: &str = "axioval:capability.relative-count";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Pumps `p1` (capacity 10) and `p2` (capacity 4), rooms `r1` (needs 8)
/// and `r2` (needs 6); each pump and room states the zone it serves or
/// lies in.
fn plant() -> Model {
    Model::default()
        .object("p1", "pump")
        .object("p2", "pump")
        .object("r1", "room")
        .object("r2", "room")
        .value("p1", "Pset", "Capacity", PropertyValue::Decimal(10.0))
        .value("p2", "Pset", "Capacity", PropertyValue::Decimal(4.0))
        .value("r1", "Pset", "Requirement", PropertyValue::Decimal(8.0))
        .value("r2", "Pset", "Requirement", PropertyValue::Decimal(6.0))
        .text("p1", "Pset", "Zone", "A")
        .text("p2", "Pset", "Zone", "B")
        .text("r1", "Pset", "Zone", "A")
        .text("r2", "Pset", "Zone", "B")
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

/// `serves`, from pumps to rooms, by the listed `pairs` (object ids).
fn listed(pairs: &[(&str, &str)]) -> Value {
    let rows: Vec<Value> = pairs
        .iter()
        .map(|(from, to)| {
            json!({ "from": { "type": "string", "value": format!("test:model/{from}") },
                    "to": { "type": "string", "value": format!("test:model/{to}") } })
        })
        .collect();
    serves(json!({ "kind": "pairs", "pairs": { "type": "table", "value": rows } }))
}

/// `serves`, from pumps to rooms, pairing equal zones.
fn by_zone() -> Value {
    serves(json!({
        "kind": "property",
        "from": { "propertySet": "t.Pset", "property": "t.Zone" },
        "to": { "propertySet": "t.Pset", "property": "t.Zone" },
    }))
}

fn serves(by: Value) -> Value {
    json!({ "serves": {
        "id": "serves",
        "name": text("Serves"),
        "from": entity("pump"),
        "to": entity("room"),
        "by": by,
    } })
}

/// Each pump's capacity covers the requirement of every room it serves.
fn capacity() -> Value {
    rule(
        "pump-capacity",
        COMPARISON,
        "error",
        entity("pump"),
        json!({
            "compared_selector": { "type": "selector", "value": entity("room") },
            "compared_property": { "type": "propertyReference", "propertySet": "t.Pset",
                                   "property": "t.Requirement" },
            "target_property": { "type": "propertyReference", "propertySet": "t.Pset",
                                 "property": "t.Capacity" },
            "operator": { "type": "string", "value": "less_or_equal" },
            "factor": { "type": "number", "value": 1.0 },
            "component_mode": { "type": "string", "value": "related" },
            "relationship": { "type": "string", "value": "axioval:derived.relation;id=serves" },
            "quantifier": { "type": "string", "value": "each" },
        }),
        json!({}),
    )
}

fn related(rules: Vec<Value>, relations: Value) -> RuleSetPackage {
    let mut set = ruleset(rules);
    set.relations = serde_json::from_value(relations).unwrap();
    set
}

fn check(set: &RuleSetPackage, model: Model) -> Result<Report, EngineError> {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[COMPARISON, COUNT],
        &["pump", "room"],
        &["Capacity", "Requirement", "Zone"],
        &["Pset"],
    );
    let plan = compile(&registry, &[definitions], set)?;
    run(registry, plan, &session(model), |runtime| runtime)
}

fn flagged(report: &Report) -> Vec<String> {
    let mut flagged: Vec<String> = report.findings().iter().map(common::subject).collect();
    flagged.sort();
    flagged
}

fn open(report: &Report) -> Vec<(Scope, String)> {
    report
        .not_evaluated
        .iter()
        .map(|outcome| (outcome.scope.clone(), outcome.message.clone()))
        .collect()
}

#[test]
fn a_listed_relation_lets_each_pump_be_compared_with_the_rooms_it_serves() {
    let set = related(
        vec![capacity()],
        listed(&[("p1", "r1"), ("p2", "r1"), ("p2", "r2")]),
    );
    let report = check(&set, plant()).unwrap();
    // p1 (10) covers r1 (8); p2 (4) covers neither r1 (8) nor r2 (6).
    assert_eq!(flagged(&report), ["p2", "p2"], "{report:#?}");
    assert!(open(&report).is_empty(), "{report:#?}");
}

#[test]
fn pairs_may_name_objects_by_external_id_and_a_related_selector_follows_them() {
    let guid = |local: &str| {
        (
            id(local),
            axioval_ir::ExternalId::new("guid", format!("G-{local}")).unwrap(),
        )
    };
    let model = plant().with_external_ids(&[guid("p1"), guid("p2"), guid("r1"), guid("r2")]);
    let mut relations = listed(&[]);
    relations["serves"]["by"] = json!({
        "kind": "pairs",
        "scheme": "guid",
        "pairs": { "type": "table", "value": [
            { "from": { "type": "string", "value": "G-p2" },
              "to": { "type": "string", "value": "G-r2" } },
        ] },
    });
    // Only the pumps serving a room are checked.
    let mut rule = capacity();
    rule["applicability"] = json!({
        "kind": "allOf",
        "operands": [entity("pump"), {
            "kind": "related", "path": ["axioval:derived.relation;id=serves"],
            "selector": entity("room"),
        }],
    });
    let report = check(&related(vec![rule], relations), model).unwrap();
    assert_eq!(flagged(&report), ["p2"], "{report:#?}");
    assert!(open(&report).is_empty(), "{report:#?}");
}

#[test]
fn a_relation_by_equal_values_pairs_each_pump_with_the_rooms_of_its_zone() {
    let report = check(&related(vec![capacity()], by_zone()), plant()).unwrap();
    // p2 (4) serves r2 (6) in zone B.
    assert_eq!(flagged(&report), ["p2"], "{report:#?}");
    assert!(open(&report).is_empty(), "{report:#?}");
}

#[test]
fn a_pair_naming_an_unknown_object_is_reported_and_leaves_its_pump_undecided() {
    let set = related(vec![capacity()], listed(&[("p1", "r1"), ("p2", "r9")]));
    let report = check(&set, plant()).unwrap();
    // p1 is decided; p2 serves a room the model does not hold.
    assert!(flagged(&report).is_empty(), "{report:#?}");
    let open = open(&report);
    assert_eq!(open.len(), 1, "{open:#?}");
    assert_eq!(open[0].0, Scope::Object(id("p2")));
    assert!(
        open[0]
            .1
            .contains("`test:model/r9` is no object of the model"),
        "{open:#?}"
    );
    // The host lists it as such.
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[COMPARISON],
        &["pump", "room"],
        &["Capacity", "Requirement", "Zone"],
        &["Pset"],
    );
    let plan = compile(&registry, &[definitions], &set).unwrap();
    let project = session(plant()).project().clone();
    let unknown = unknown_relation_objects(&plan, &project);
    assert_eq!(unknown.len(), 1);
    assert_eq!(
        (unknown[0].relation.as_str(), unknown[0].row, unknown[0].end),
        ("serves", 2, "to")
    );
    assert_eq!(unknown[0].identity, "test:model/r9");
}

#[test]
fn a_room_whose_zone_cannot_be_read_leaves_every_pump_undecided() {
    let model = plant().object("r3", "room").value(
        "r3",
        "Pset",
        "Zone",
        PropertyValue::List(vec![PropertyValue::String("A".into())]),
    );
    let report = check(&related(vec![capacity()], by_zone()), model).unwrap();
    assert!(flagged(&report).is_empty(), "{report:#?}");
    let open = open(&report);
    assert_eq!(
        open.iter()
            .map(|(scope, _)| scope.clone())
            .collect::<Vec<_>>(),
        [Scope::Object(id("p1")), Scope::Object(id("p2"))],
        "{open:#?}"
    );
}

#[test]
fn malformed_relations_are_refused() {
    let refused = |relations: Value| match check(&related(vec![capacity()], relations), plant()) {
        Err(EngineError::InvalidRelation { detail, .. }) => detail,
        other => panic!("expected an invalid relation, got {other:?}"),
    };
    let mut renamed = listed(&[("p1", "r1")]);
    renamed["serves"]["id"] = json!("other");
    assert!(refused(renamed).contains("not its id"));
    let mut blank = listed(&[("p1", "r1")]);
    blank["serves"]["by"]["pairs"]["value"][0]["to"]["value"] = json!(" ");
    assert!(refused(blank).contains("blank"));
    let mut numeric = listed(&[("p1", "r1")]);
    numeric["serves"]["by"]["pairs"]["value"][0]["to"] = json!({ "type": "integer", "value": 1 });
    assert!(refused(numeric).contains("pairs row 1"));
    let mut looping = by_zone();
    looping["serves"]["from"] = json!({
        "kind": "related", "path": ["axioval:derived.relation;id=serves"],
        "selector": entity("room"),
    });
    assert!(refused(looping).contains("declared relation"));
}
