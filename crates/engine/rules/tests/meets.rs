//! `meets` selectors: objects meeting a selectable capability on their own.
#![allow(missing_docs)]

use axioval_engine::{CapabilityRegistry, EngineError, Runtime, compile};
use axioval_ir::{DefinitionPackage, Object, ObjectId, Project, RuleSetPackage, SourceId};
use axioval_rules::register_builtins;
use serde_json::{Value, json};

fn text(value: &str) -> Value {
    json!({"default": value, "translations": {}})
}

fn packages(selector: &Value) -> (DefinitionPackage, RuleSetPackage) {
    let package = |id: &str| json!({"id": id, "name": text(id), "version": "1.0.0", "authors": []});
    let definitions = json!({
        "schemaVersion": "0.1.0",
        "package": package("test:definitions"),
        "definitions": {"test:none": {
            "id": "test:none",
            "name": text("None may exist"),
            "capability": "axioval:capability.population",
            "parameters": {
                "min": {"id": "min", "name": text("min"), "kind": "integer", "required": false},
                "max": {"id": "max", "name": text("max"), "kind": "integer", "required": false}
            }
        }}
    });
    let ruleset = json!({
        "schemaVersion": "0.1.0",
        "package": package("test:ruleset"),
        "definitionPackages": ["test:definitions"],
        "root": {"id": "root", "name": text("Root"), "rules": [{
            "id": "no-walls",
            "definitionId": "test:none",
            "name": text("No walls"),
            "parameters": {"max": {"type": "integer", "value": 0}},
            "applicability": selector
        }]}
    });
    (
        serde_json::from_value(definitions).unwrap(),
        serde_json::from_value(ruleset).unwrap(),
    )
}

fn meets(capability: &str, parameters: &Value) -> Value {
    json!({"kind": "meets", "capability": capability, "parameters": parameters})
}

fn walls() -> Value {
    meets(
        "axioval:capability.entity",
        &json!({"classes": {"type": "stringList", "value": ["IFCWALL"]}}),
    )
}

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

#[test]
fn a_meets_selector_selects_the_objects_meeting_the_capability() {
    let (definitions, rules) = packages(&walls());
    let plan = compile(&registry(), &[definitions], &rules).unwrap();
    let source = SourceId::new("cad", "model").unwrap();
    let object =
        |local: &str, kind: &str| Object::new(ObjectId::new(source.clone(), local).unwrap(), kind);
    let project = Project::new(vec![object("1", "IFCWALL"), object("2", "IFCSLAB")]).unwrap();
    let report = Runtime::new(registry()).run(&project, plan).unwrap();
    let flagged: Vec<&str> = report
        .findings()
        .iter()
        .map(|finding| finding.object_id.local_id.as_str())
        .collect();
    assert_eq!(flagged, ["1"], "{report:?}");
    assert!(report.not_evaluated().is_empty(), "{report:?}");
}

#[test]
fn compile_refuses_unknown_unselectable_and_mistyped_capabilities() {
    let cases = [
        meets("axioval:capability.nonexistent", &json!({})),
        // Counting is about populations, never one object.
        meets(
            "axioval:capability.population",
            &json!({"min": {"type": "integer", "value": 1}}),
        ),
        meets(
            "axioval:capability.entity",
            &json!({"classes": {"type": "string", "value": "IFCWALL"}}),
        ),
        meets(
            "axioval:capability.entity",
            &json!({"colour": {"type": "string", "value": "red"}}),
        ),
        json!({"kind": "not", "operand": meets(
"axioval:capability.property-value", &json!({}))}),
    ];
    for selector in cases {
        let (definitions, rules) = packages(&selector);
        let error = compile(&registry(), &[definitions], &rules).unwrap_err();
        assert!(
            matches!(
                error,
                EngineError::UnknownCapability(_)
                    | EngineError::CapabilityContract { .. }
                    | EngineError::InvalidParameterType { .. }
                    | EngineError::UnknownParameter { .. }
                    | EngineError::MissingParameter { .. }
            ),
            "{selector}: {error:?}"
        );
    }
}
