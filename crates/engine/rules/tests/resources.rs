//! Resource objects: selected only by a selector naming their class, judged
//! through the same services as objects, and invisible to every other rule.
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, EvidenceSession, LocationMethod, LocationPolicy, ResourceError,
    ResourceObjects, ResourceRequest, ResourceService, ResourceServiceHandle, SourceSnapshot,
};
use axioval_ir::{NotEvaluatedReason, Object, PropertyValue, Report, Scope};
use axioval_rules::{ObjectCount, PropertyRequired, register_builtins};
use common::runtime::{definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, findings, id, kind, property, rule as compiled, source};
use serde_json::{Value, json};

const REQUIRED: &str = "axioval:capability.property-required";
const COUNT: &str = "axioval:capability.object-count";

/// A wall and two materials: `#7` is named, `#8` is not. Only the wall is
/// an object; the materials are resource objects.
fn model() -> Model {
    Model::default()
        .object("w1", "wall")
        .text("#7", "Attributes", "Name", "Concrete")
}

fn materials() -> Vec<Object> {
    vec![
        Object::new(id("#7"), "material"),
        Object::new(id("#8"), "material"),
    ]
}

/// Lists the materials for the class `material`, nothing for any other.
struct Materials(Vec<SourceSnapshot>);
impl ResourceService for Materials {
    fn source_snapshots(&self) -> &[SourceSnapshot] {
        &self.0
    }
    fn resources(&self, request: &ResourceRequest) -> Result<Vec<Object>, ResourceError> {
        Ok(if request.class() == "material" {
            materials()
        } else {
            Vec::new()
        })
    }
}

fn with_materials(session: EvidenceSession) -> EvidenceSession {
    session
        .with_service(ResourceServiceHandle::new(Arc::new(Materials(vec![
            snapshot(),
        ]))))
        .unwrap()
}

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn name_required(id: &str, applicability: Value) -> Value {
    rule(
        id,
        REQUIRED,
        "error",
        applicability,
        json!({ "property": { "type": "propertyReference", "property": "t.Name",
                              "propertySet": "t.Attributes" } }),
        json!({}),
    )
}

fn check(
    rules: Vec<Value>,
    configure: impl FnOnce(axioval_engine::Runtime) -> axioval_engine::Runtime,
) -> Report {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[REQUIRED],
        &["wall", "material"],
        &["Name"],
        &["Attributes"],
    );
    let plan = plan(&registry, &definitions, rules).unwrap();
    run(registry, plan, &with_materials(session(model())), configure).unwrap()
}

fn subjects(report: &Report) -> Vec<&str> {
    report
        .findings()
        .iter()
        .filter_map(|finding| finding.object_id())
        .map(|id| id.local_id.as_str())
        .collect()
}

#[test]
fn a_rule_naming_a_resource_class_judges_its_resource_objects() {
    let report = check(
        vec![name_required("named", entity("material"))],
        |runtime| runtime,
    );
    assert_eq!(subjects(&report), ["#8"]);
    assert_eq!(report.resources, [Object::new(id("#8"), "material")]);
}

#[test]
fn a_rule_over_every_object_never_sees_a_resource_object() {
    let all = name_required("all", json!({ "kind": "all" }));
    let with = check(vec![all.clone()], |runtime| runtime);
    assert_eq!(subjects(&with), ["w1"]);
    assert!(with.resources.is_empty());
    // Byte for byte the report of a session without resource objects.
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[REQUIRED],
        &["wall"],
        &["Name"],
        &["Attributes"],
    );
    let plan = plan(&registry, &definitions, vec![all]).unwrap();
    let without = run(registry, plan, &session(model()), |runtime| runtime).unwrap();
    assert_eq!(
        serde_json::to_string(&with).unwrap(),
        serde_json::to_string(&without).unwrap()
    );
}

#[test]
fn a_negation_reaches_no_resource_object() {
    let report = check(
        vec![name_required(
            "not-walls",
            json!({ "kind": "not", "operand": entity("wall") }),
        )],
        |runtime| runtime,
    );
    assert!(report.findings().is_empty());
}

#[test]
fn a_rule_reading_another_rule_reaches_the_resource_objects_it_judged() {
    let parent = name_required("parent", entity("material"));
    let child = rule(
        "child",
        REQUIRED,
        "error",
        json!({ "kind": "ruleOutcome", "rule": "parent", "outcome": "failed" }),
        json!({ "property": { "type": "propertyReference", "property": "t.Name",
                              "propertySet": "t.Attributes" } }),
        json!({}),
    );
    let report = check(vec![parent, child], |runtime| runtime);
    let child: Vec<&str> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == "child")
        .filter_map(|finding| finding.object_id())
        .map(|id| id.local_id.as_str())
        .collect();
    assert_eq!(child, ["#8"]);
}

#[test]
fn a_located_resource_object_is_placed_nowhere_and_doubted_by_nothing() {
    let report = check(
        vec![name_required("named", entity("material"))],
        |runtime| {
            runtime.with_locations(LocationPolicy {
                method: LocationMethod::Storeys,
                storey_kinds: vec!["storey".into()],
                space_kinds: vec!["space".into()],
                containment: vec!["contains:backward".into()],
                name: None,
            })
        },
    );
    let location = report.findings()[0].location.as_ref().unwrap();
    assert!(location.storeys.is_empty() && location.spaces.is_empty());
    assert!(location.unresolved.is_none());
}

#[test]
fn a_count_of_a_resource_class_counts_resource_objects() {
    let resources = ResourceObjects::new().with_class(source(), "material", false, Ok(materials()));
    let count = |population: ResourceObjects| {
        model().evaluate_with(
            &ObjectCount,
            &compiled(
                COUNT,
                kind("material"),
                vec![("maximum", common::integer(1))],
            ),
            |services| services.register(population).unwrap(),
        )
    };
    assert_eq!(
        findings(&count(resources)),
        [(
            "source".to_owned(),
            "2 object(s) match the selection in source `test:model`; required at most 1".to_owned()
        )]
    );
    let unread =
        count(ResourceObjects::new().with_class(source(), "material", false, Err("broken".into())));
    let outcome = &unread.not_evaluated_outcomes()[0];
    assert_eq!(outcome.scope(), &Scope::Source(source()));
    assert_eq!(outcome.reason(), &NotEvaluatedReason::IncompleteEvidence);
}

#[test]
fn a_resource_object_is_judged_by_the_capability_as_an_object_is() {
    let resources = ResourceObjects::new().with_class(source(), "material", false, Ok(materials()));
    let evaluation = model()
        .value(
            "#8",
            "Attributes",
            "Name",
            PropertyValue::Reference(id("#1")),
        )
        .evaluate_with(
            &PropertyRequired,
            &compiled(
                REQUIRED,
                kind("material"),
                vec![("property", property(Some("Attributes"), "Name"))],
            ),
            |services| services.register(resources).unwrap(),
        );
    // A reference is a value: both materials hold a name.
    assert!(evaluation.findings().is_empty());
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}
