//! Groups a ruleset derives from its members (`groupings`), checked like
//! groups a model states.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use axioval_engine::{CapabilityRegistry, EngineError, compile};
use axioval_ir::{NotEvaluatedReason, PropertyValue, Report, RuleSetPackage, Scope};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, rule, ruleset, run, session};
use common::{Model, id};
use serde_json::{Value, json};

const COMPOSITION: &str = "axioval:capability.group-composition";
const KEYED: &str = "axioval:capability.keyed-limit";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Rooms `r1` and `r2` of flat 1, `r3` of flat 2, and `r4` with no flat
/// number; `extra` adds more.
fn rooms(extra: impl FnOnce(Model) -> Model) -> Model {
    let model = Model::default()
        .object("r1", "space")
        .object("r2", "space")
        .object("r3", "space")
        .object("r4", "space")
        .text("r1", "Pset", "FlatNumber", "1")
        .text("r2", "Pset", "FlatNumber", "1")
        .text("r3", "Pset", "FlatNumber", "2");
    extra(model)
}

/// Flats: the spaces grouped by their flat number.
fn flats() -> Value {
    json!({
        "flats": {
            "id": "flats",
            "name": { "default": "Flats", "translations": {} },
            "members": entity("space"),
            "by": { "kind": "property", "propertySet": "t.Pset", "property": "t.FlatNumber" },
        }
    })
}

fn grouped(rules: Vec<Value>, groupings: Value) -> RuleSetPackage {
    let mut set = ruleset(rules);
    set.groupings = serde_json::from_value(groupings).unwrap();
    set
}

/// Every flat has two rooms, and every space is in a flat.
fn composition() -> Value {
    rule(
        "flat-rooms",
        COMPOSITION,
        "error",
        json!({ "kind": "derivedGroup", "grouping": "flats" }),
        json!({
            "relationship": { "type": "string", "value": "axioval:derived.group;by=flats" },
            "direction": { "type": "string", "value": "backward" },
            "requirements": { "type": "table", "value": [
                { "label": { "type": "string", "value": "room" },
                  "count": { "type": "integer", "value": 2 } },
            ] },
            "ungrouped_selector": { "type": "selector", "value": entity("space") },
        }),
        json!({}),
    )
}

/// No flat has more than one room, read from the group's own facts.
fn at_most_one_room() -> Value {
    rule(
        "flat-size",
        KEYED,
        "error",
        json!({ "kind": "derivedGroup", "grouping": "flats" }),
        json!({
            "key_1": { "type": "propertyReference", "propertySet": "axioval:group",
                       "property": "key" },
            "quantity": { "type": "string", "value": "property" },
            "quantity_property": { "type": "propertyReference", "propertySet": "axioval:group",
                                   "property": "members" },
            "limits": { "type": "table", "value": [
                { "maximum": { "type": "number", "value": 1.0 } },
            ] },
        }),
        json!({}),
    )
}

fn check(set: &RuleSetPackage, model: Model) -> Result<Report, EngineError> {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[COMPOSITION, KEYED],
        &["space"],
        &["FlatNumber"],
        &["Pset"],
    );
    let plan = compile(&registry, &[definitions], set)?;
    run(registry, plan, &session(model), |runtime| runtime)
}

fn found(report: &Report, rule: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(|finding| (common::subject(finding), finding.message.clone()))
        .collect();
    found.sort();
    found
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
fn rooms_sharing_a_flat_number_form_one_group_and_a_room_without_one_is_ungrouped() {
    let report = check(&grouped(vec![composition()], flats()), rooms(|model| model)).unwrap();
    let flat = |key: &str| format!("axioval:group/flats/{key}");
    let findings = found(&report, "flat-rooms");
    assert_eq!(findings.len(), 2, "{findings:#?}");
    // Flat 1 has its two rooms; flat 2 has one.
    assert_eq!(findings[0].0, flat("2"), "{findings:#?}");
    assert!(
        findings[0].1.contains("has 1 of 2 required member(s)"),
        "{findings:#?}"
    );
    assert_eq!(findings[1].0, "r4");
    assert!(findings[1].1.contains("in no group"), "{findings:#?}");
    assert!(open(&report, "flat-rooms").is_empty(), "{report:#?}");
    // The report carries the derived groups it names.
    assert!(
        report
            .resources
            .iter()
            .any(|object| object.id.local_id == flat("2") && object.kind == "axioval:group")
    );
}

#[test]
fn a_groups_key_and_member_count_are_its_own_facts() {
    let report = check(
        &grouped(vec![at_most_one_room()], flats()),
        rooms(|model| model),
    )
    .unwrap();
    let findings = found(&report, "flat-size");
    assert_eq!(
        findings.len(),
        1,
        "{findings:#?} {:#?}",
        report.not_evaluated
    );
    assert_eq!(findings[0].0, "axioval:group/flats/1");
    assert!(open(&report, "flat-size").is_empty(), "{report:#?}");
}

#[test]
fn a_room_whose_flat_number_cannot_be_read_leaves_every_flat_undecided() {
    let model = rooms(|model| {
        model.object("r5", "space").value(
            "r5",
            "Pset",
            "FlatNumber",
            PropertyValue::List(vec![PropertyValue::String("1".into())]),
        )
    });
    let report = check(&grouped(vec![composition()], flats()), model).unwrap();
    let open = open(&report, "flat-rooms");
    // The list of flats is incomplete, and so is every flat.
    assert!(
        open.iter()
            .any(|(scope, _)| matches!(scope, Scope::Source(_))),
        "{open:#?}"
    );
    assert!(
        found(&report, "flat-rooms")
            .iter()
            .all(|(subject, _)| !subject.starts_with("axioval:group/")),
        "{report:#?}"
    );
    // Every flat is open: the rule as a whole, as the template leaves it
    // (divergence D40), or each flat and the ungrouped room.
    assert!(open.iter().any(|(scope, _)| *scope == Scope::Project
        || *scope == Scope::Object(id("r4"))
        || matches!(scope, Scope::Object(object) if object.local_id.starts_with("axioval:group/"))));
}

#[test]
fn a_grouping_naming_a_rule_or_another_grouping_is_refused() {
    let mut definitions = flats();
    definitions["flats"]["members"] = json!({ "kind": "derivedGroup", "grouping": "flats" });
    assert!(matches!(
        check(
            &grouped(vec![composition()], definitions),
            rooms(|model| model)
        ),
        Err(EngineError::InvalidGrouping { .. })
    ));
    let mut bad_id = flats();
    bad_id["flats"]["id"] = json!("other");
    assert!(matches!(
        check(&grouped(vec![composition()], bad_id), rooms(|model| model)),
        Err(EngineError::InvalidGrouping { .. })
    ));
    // A selector naming an undeclared grouping is an unknown concept.
    assert!(matches!(
        check(&ruleset(vec![composition()]), rooms(|model| model)),
        Err(EngineError::UnknownConcept { .. })
    ));
}

/// Each room's code in `Uniclass`, as its source states it.
struct Codes(Vec<axioval_engine::SourceSnapshot>);

impl axioval_engine::ClassificationService for Codes {
    fn source_snapshots(&self) -> &[axioval_engine::SourceSnapshot] {
        &self.0
    }
    fn classifications(
        &self,
        object: &axioval_ir::ObjectId,
    ) -> Result<Vec<axioval_engine::ClassificationAssignment>, axioval_engine::ClassificationError>
    {
        let code = match object.local_id.as_str() {
            "r1" | "r2" => "SL_20",
            "r3" => "SL_30",
            _ => return Ok(Vec::new()),
        };
        Ok(vec![axioval_engine::ClassificationAssignment {
            system: Some("Uniclass".into()),
            codes: vec![Some(code.into())],
        }])
    }
}

#[test]
fn rooms_sharing_a_classification_code_form_one_group() {
    let registry = registry();
    let definitions = definitions(&registry, &[COMPOSITION], &["space"], &[], &[]);
    let mut composition = composition();
    composition["applicability"]["grouping"] = json!("zones");
    composition["parameters"]["relationship"]["value"] = json!("axioval:derived.group;by=zones");
    let set = grouped(
        vec![composition],
        json!({
            "zones": {
                "id": "zones",
                "name": { "default": "Zones", "translations": {} },
                "members": entity("space"),
                "by": { "kind": "classification", "system": "Uniclass" },
            }
        }),
    );
    let plan = compile(&registry, &[definitions], &set).unwrap();
    let session = session(rooms(|model| model))
        .with_service(axioval_engine::ClassificationServiceHandle::new(
            std::sync::Arc::new(Codes(vec![common::runtime::snapshot()])),
        ))
        .unwrap();
    let report = run(registry, plan, &session, |runtime| runtime).unwrap();
    let findings = found(&report, "flat-rooms");
    assert_eq!(
        findings
            .iter()
            .map(|(subject, _)| subject.as_str())
            .collect::<Vec<_>>(),
        ["axioval:group/zones/SL_30", "r4"],
        "{findings:#?}"
    );
    assert!(open(&report, "flat-rooms").is_empty(), "{report:#?}");
}
