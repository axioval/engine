//! The expression contract: one golden fixture per node kind.
#![allow(missing_docs)]
use axioval_ir::contract::{Expression, ExpressionError, ParameterValue, ScalarValue};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn fixtures() -> Vec<(String, Value)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/expression");
    let mut fixtures: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            (stem, serde_json::from_str(&text).unwrap())
        })
        .collect();
    fixtures.sort_by(|a, b| a.0.cmp(&b.0));
    fixtures
}

#[test]
fn every_fixture_round_trips() {
    for (name, json) in fixtures() {
        let expression: Expression = serde_json::from_value(json.clone())
            .unwrap_or_else(|error| panic!("{name} does not parse: {error}"));
        expression
            .validate()
            .unwrap_or_else(|error| panic!("{name} is refused: {error}"));
        let kind = name.split_once('-').map_or(name.as_str(), |(kind, _)| kind);
        assert_eq!(expression.kind(), kind, "{name} names its root kind");
        assert_eq!(serde_json::to_value(&expression).unwrap(), json, "{name}");
    }
}

#[test]
fn every_kind_has_a_fixture() {
    let covered: BTreeSet<String> = fixtures()
        .into_iter()
        .map(|(_, json)| json["kind"].as_str().unwrap().to_owned())
        .collect();
    let kinds: BTreeSet<String> = Expression::KINDS.iter().map(|&kind| kind.into()).collect();
    assert_eq!(covered, kinds);
}

#[test]
fn every_scalar_kind_has_a_literal_fixture() {
    let types: BTreeSet<String> = fixtures()
        .into_iter()
        .filter(|(_, json)| json["kind"] == "literal")
        .map(|(_, json)| json["value"]["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        types,
        [
            "boolean", "date", "dateTime", "enum", "integer", "number", "quantity", "string"
        ]
        .map(str::to_owned)
        .into()
    );
}

fn refused(json: &Value) -> String {
    serde_json::from_value::<Expression>(json.clone())
        .unwrap_err()
        .to_string()
}

#[test]
fn unknown_kinds_fields_and_literal_types_are_refused() {
    assert!(refused(&json!({"kind": "loop", "body": {"kind": "null"}})).contains("loop"));
    assert!(
        refused(&json!({"kind": "null", "comment": "x"})).contains("comment"),
        "an unknown field"
    );
    assert!(
        refused(&json!({"kind": "not", "operand": {"kind": "null", "extra": 1}})).contains("extra")
    );
    // Only scalars are literals: a list, a table or a selector is not.
    assert!(
        refused(&json!({"kind": "literal", "value": {"type": "stringList", "value": ["a"]}}))
            .contains("stringList")
    );
    assert!(
        refused(
            &json!({"kind": "literal", "value": {"type": "string", "value": "a", "unit": "m"}})
        )
        .contains("unit")
    );
    assert!(
        refused(&json!({"kind": "compare", "operator": "exists",
            "left": {"kind": "null"}, "right": {"kind": "null"}}))
        .contains("exists")
    );
    // `refused` fails the test unless the date is refused.
    refused(&json!({"kind": "literal", "value": {"type": "date", "value": "04.10.2026"}}));
}

fn invalid(json: &Value) -> ExpressionError {
    serde_json::from_value::<Expression>(json.clone())
        .unwrap()
        .validate()
        .unwrap_err()
}

#[test]
fn structure_serde_cannot_state_is_refused_by_validation() {
    assert_eq!(
        invalid(&json!({"kind": "and", "operands": []})),
        ExpressionError::NoOperands { kind: "and" }
    );
    assert_eq!(
        invalid(&json!({"kind": "oneOf", "operand": {"kind": "null"}, "values": []})),
        ExpressionError::NoOperands { kind: "oneOf" }
    );
    assert_eq!(
        invalid(&json!({"kind": "if", "branches": [], "else": {"kind": "null"}})),
        ExpressionError::NoBranches
    );
    assert_eq!(
        invalid(&json!({"kind": "parameter", "name": " "})),
        ExpressionError::Blank {
            kind: "parameter",
            field: "name"
        }
    );
    assert_eq!(
        invalid(&json!({"kind": "null", "label": ""})),
        ExpressionError::Blank {
            kind: "null",
            field: "label"
        }
    );
    assert_eq!(
        invalid(&json!({"kind": "lookup", "table": "limits", "keys": {}, "column": "maximum"})),
        ExpressionError::NoKeys {
            table: "limits".into()
        }
    );
    let mut deep = json!({"kind": "null"});
    for _ in 0..axioval_ir::contract::MAX_EXPRESSION_DEPTH {
        deep = json!({"kind": "not", "operand": deep});
    }
    assert_eq!(invalid(&deep), ExpressionError::TooDeep);
}

#[test]
fn lookup_keys_serialize_in_column_order() {
    let expression: Expression = serde_json::from_value(json!({
        "kind": "lookup", "table": "limits", "column": "maximum",
        "keys": {"key_2": {"kind": "null"}, "key_1": {"kind": "null"}}
    }))
    .unwrap();
    let text = serde_json::to_string(&expression).unwrap();
    assert!(text.find("key_1").unwrap() < text.find("key_2").unwrap());
}

#[test]
fn literals_convert_to_and_from_parameter_values() {
    let scalar = ScalarValue::Quantity {
        value: 0.04,
        unit: "m".into(),
    };
    let parameter = ParameterValue::from(scalar.clone());
    assert_eq!(ScalarValue::try_from(parameter), Ok(scalar));
    let list = ParameterValue::StringList {
        value: vec!["a".into()],
    };
    assert_eq!(ScalarValue::try_from(list.clone()), Err(list));
}

#[test]
fn children_follow_written_order() {
    let expression: Expression = serde_json::from_value(json!({
        "kind": "if",
        "branches": [{"when": {"kind": "parameter", "name": "a"},
                      "then": {"kind": "parameter", "name": "b"}}],
        "else": {"kind": "parameter", "name": "c"}
    }))
    .unwrap();
    let names: Vec<_> = expression
        .children()
        .into_iter()
        .map(|child| match child {
            Expression::Parameter { name, .. } => name.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
}
