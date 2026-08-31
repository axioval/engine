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
fn selector_parameter_round_trips_nested_contract() {
    let json = r#"{"type":"selector","value":{"kind":"not","operand":{"kind":"entityType","objectType":"axioval:example.wall","includeSubtypes":true}}}"#;
    let value: ParameterValue = serde_json::from_str(json).unwrap();
    match &value {
        ParameterValue::Selector { value } => {
            assert!(matches!(value.as_ref(), Selector::Not { .. }));
        }
        _ => panic!("expected selector parameter"),
    }
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
}

#[test]
fn canonical_quantity_round_trips_with_dimension() {
    let value = axioval_ir::PropertyValue::Quantity {
        value: 2.5,
        dimension: axioval_ir::QuantityDimension::Area,
    };
    let json = serde_json::to_value(&value).unwrap();
    assert_eq!(json["type"], "quantity");
    assert_eq!(json["value"]["dimension"], "area");
    assert_eq!(
        serde_json::from_value::<axioval_ir::PropertyValue>(json).unwrap(),
        value
    );
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
