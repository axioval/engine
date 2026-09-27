//! Canonical schema fixture contract.
#![allow(missing_docs)]
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{DefinitionPackage, RuleSetPackage};
const D: &str = include_str!("../../../../fixtures/schema-v0.1.0/definitions.json");
const R: &str = include_str!("../../../../fixtures/schema-v0.1.0/ruleset.json");
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
    // Real MCS output: one named target group, a requirement bound to it,
    // citations and an explanatory image. All of it must deserialize.
    let axioval_ir::contract::RuleApplicability::Groups(groups) = &rule.applicability else {
        panic!("the MCS minimal example uses target-group applicability");
    };
    assert!(matches!(
        groups.groups["walls"].selector,
        Selector::EntityType {
            include_subtypes: true,
            ..
        }
    ));
    assert_eq!(rule.requirements[0].target_groups, ["walls"]);
    assert_eq!(rule.explanatory_images.len(), 1);
    assert!(
        d.object_types["axioval:example.ifc.wall"].external_names[0]
            .type_system
            .starts_with("https://")
    );
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
fn list_value_and_selector_quantifier_round_trip() {
    use axioval_ir::PropertyValue;
    let value = PropertyValue::List(vec![
        PropertyValue::String("A-AXIS".into()),
        PropertyValue::String("A-WALL".into()),
    ]);
    let json = serde_json::to_value(&value).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"type": "list", "value": [
            {"type": "string", "value": "A-AXIS"},
            {"type": "string", "value": "A-WALL"},
        ]})
    );
    assert_eq!(
        serde_json::from_value::<PropertyValue>(json).unwrap(),
        value
    );
    let quantified = r#"{"kind":"property","propertySet":"axioval:presentation","property":"Layer","operator":"oneOf","value":{"type":"stringList","value":["A-WALL"]},"quantifier":"all"}"#;
    let selector: Selector = serde_json::from_str(quantified).unwrap();
    assert!(matches!(
        selector,
        Selector::Property {
            quantifier: Some(axioval_ir::contract::Quantifier::All),
            ..
        }
    ));
    assert_eq!(
        serde_json::to_value(&selector).unwrap(),
        serde_json::from_str::<serde_json::Value>(quantified).unwrap()
    );
    assert!(serde_json::from_str::<Selector>(&quantified.replace("\"all\"", "\"some\"")).is_err());
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

#[test]
fn property_selector_text_options_default_and_round_trip() {
    // Without the options a selector reads as before and writes nothing new.
    let plain = r#"{"kind":"property","propertySet":null,"property":"axioval:example.name","operator":"equals","value":{"type":"string","value":"A"}}"#;
    let selector: Selector = serde_json::from_str(plain).unwrap();
    assert!(matches!(
        selector,
        Selector::Property {
            case_sensitive: true,
            trim: false,
            ..
        }
    ));
    assert_eq!(
        serde_json::to_value(&selector).unwrap(),
        serde_json::from_str::<serde_json::Value>(plain).unwrap()
    );
    let folded = r#"{"kind":"property","propertySet":null,"property":"axioval:example.name","operator":"noneOf","value":{"type":"stringList","value":["a","b"]},"caseSensitive":false,"trim":true}"#;
    let selector: Selector = serde_json::from_str(folded).unwrap();
    assert_eq!(
        serde_json::to_value(&selector).unwrap(),
        serde_json::from_str::<serde_json::Value>(folded).unwrap()
    );
    for operator in ["like", "contains", "oneOf", "noneOf"] {
        let json = plain.replace("\"equals\"", &format!("\"{operator}\""));
        assert!(
            serde_json::from_str::<Selector>(&json).is_ok(),
            "{operator}"
        );
    }
}

#[test]
fn packages_without_tables_serialize_without_columns() {
    let d: DefinitionPackage = serde_json::from_str(D).unwrap();
    let written = serde_json::to_string(&d).unwrap();
    assert!(!written.contains("columns"));
    let reread: DefinitionPackage = serde_json::from_str(&written).unwrap();
    assert_eq!(reread, d);
}

#[test]
fn table_parameter_definition_and_value_round_trip() {
    use axioval_ir::contract::{
        ColumnKind, ParameterDefinition, ParameterKind, TableColumnDefinition,
    };
    let definition = r#"{"id":"limits","name":{"default":"Limits","translations":{}},"description":null,"kind":"table","referencedValueKind":null,"required":true,"defaultValue":null,"allowedValues":[],"unitDimension":null,"citations":[],"columns":[{"id":"space_type","name":{"default":"Space type","translations":{}},"kind":"textPattern","required":true},{"id":"minimum_area","name":{"default":"Minimum area","translations":{}},"kind":"quantity","required":false,"unitDimension":"area"}]}"#;
    let parsed: ParameterDefinition = serde_json::from_str(definition).unwrap();
    assert_eq!(parsed.kind, ParameterKind::Table);
    assert_eq!(parsed.columns[0].kind, ColumnKind::TextPattern);
    assert!(!parsed.columns[1].required);
    assert_eq!(
        serde_json::to_value(&parsed).unwrap(),
        serde_json::from_str::<serde_json::Value>(definition).unwrap()
    );
    // `required` defaults to true, as for parameters.
    let column: TableColumnDefinition = serde_json::from_str(
        r#"{"id":"label","name":{"default":"Label","translations":{}},"kind":"string"}"#,
    )
    .unwrap();
    assert!(column.required);

    let value = r#"{"type":"table","value":[{"minimum_area":{"type":"quantity","value":10.0,"unit":"m2"},"space_type":{"type":"string","value":"Office*"}}]}"#;
    let parsed: ParameterValue = serde_json::from_str(value).unwrap();
    let ParameterValue::Table { value: rows } = &parsed else {
        panic!("a table value");
    };
    assert_eq!(rows[0].len(), 2);
    assert_eq!(serde_json::to_string(&parsed).unwrap(), value);
}

#[test]
fn table_contracts_reject_unknown_fields_and_kinds() {
    use axioval_ir::contract::TableColumnDefinition;
    assert!(
        serde_json::from_str::<TableColumnDefinition>(
            r#"{"id":"a","name":{"default":"A","translations":{}},"kind":"string","width":3}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<TableColumnDefinition>(
            r#"{"id":"a","name":{"default":"A","translations":{}},"kind":"table"}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<ParameterValue>(r#"{"type":"table","rows":[]}"#).is_err());
}
