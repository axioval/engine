//! Built-in property capability tests against canonical schema fixtures.
#![allow(missing_docs)]

use axioval_engine::{CapabilityRegistry, Runtime, compile};
use axioval_ir::contract::{ComparisonOperator, Selector, Severity as RuleSeverity};
use axioval_ir::{
    DefinitionPackage, Evidence, Object, ObjectId, Project, Property, PropertyValue,
    RuleSetPackage, SourceId,
};
use axioval_rules::register_builtins;

fn packages() -> (DefinitionPackage, RuleSetPackage) {
    (
        serde_json::from_str(include_str!(
            "../../../fixtures/schema-v0.1.0/definitions.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!("../../../fixtures/schema-v0.1.0/ruleset.json")).unwrap(),
    )
}
fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}
fn object() -> Object {
    Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        "axioval:example.ifc.wall",
    )
}
#[test]
fn property_exists_reports_missing_exact_evidence() {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let (definitions, rules) = packages();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry)
        .run(&Project::new(vec![object()]).unwrap(), plan)
        .unwrap();
    assert_eq!(report.findings().len(), 1);
}
#[test]
fn property_exists_accepts_exact_non_ifc_source_evidence() {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let (definitions, rules) = packages();
    let property = Property::new(
        "axioval:example.ifc.pset-wall-common",
        "axioval:example.ifc.reference",
        PropertyValue::String("EI60".into()),
    )
    .unwrap()
    .with_evidence(Evidence::exact(source(), "native property"));
    let project = Project::new(vec![object().with_property(property)]).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    assert!(
        Runtime::new(registry)
            .run(&project, plan)
            .unwrap()
            .findings()
            .is_empty()
    );
}

#[test]
fn property_applicability_exists_selects_exact_matching_objects() {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let (definitions, mut rules) = packages();
    rules.root.rules[0].applicability = Selector::Property {
        property_set: Some("Pset.Trigger".into()),
        property: "Enabled".into(),
        operator: ComparisonOperator::Exists,
        value: None,
    };
    let trigger = Property::new("Pset.Trigger", "Enabled", PropertyValue::Boolean(true))
        .unwrap()
        .with_evidence(Evidence::exact(source(), "native trigger"));
    let project = Project::new(vec![object().with_property(trigger)]).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    assert_eq!(
        Runtime::new(registry)
            .run(&project, plan)
            .unwrap()
            .findings()
            .len(),
        1
    );
}

#[test]
fn info_severity_is_preserved_in_findings() {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let (definitions, mut rules) = packages();
    rules.root.rules[0].severity = RuleSeverity::Info;
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry)
        .run(&Project::new(vec![object()]).unwrap(), plan)
        .unwrap();
    assert_eq!(report.findings()[0].severity, axioval_ir::Severity::Info);
}
