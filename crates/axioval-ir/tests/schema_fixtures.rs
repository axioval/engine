//! Canonical schema fixture contract.
#![allow(missing_docs)]
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{DefinitionPackage, RuleSetPackage};
const D: &str = include_str!("../../../fixtures/schema-v0.1.0/definitions.json");
const R: &str = include_str!("../../../fixtures/schema-v0.1.0/ruleset.json");
#[test]
fn canonical_schema_v010_parses() {
    let d: DefinitionPackage = serde_json::from_str(D).unwrap();
    assert_eq!(
        d.definitions["axioval:example.property-exists"].capability,
        "axioval:capability.property-exists"
    );
    let r: RuleSetPackage = serde_json::from_str(R).unwrap();
    let rule = &r.root.rules[0];
    assert!(matches!(
        rule.parameters["property"],
        ParameterValue::PropertyReference { .. }
    ));
    assert!(matches!(
        rule.applicability,
        Selector::EntityType {
            include_subtypes: true,
            ..
        }
    ));
}

#[test]
fn contract_rejects_unknown_fields() {
    let mutated = D.replacen(
        "\"schemaVersion\":",
        "\"unexpected\":1,\"schemaVersion\":",
        1,
    );
    assert!(serde_json::from_str::<DefinitionPackage>(&mutated).is_err());
}
