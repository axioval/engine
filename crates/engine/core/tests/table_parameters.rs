//! Binding table-valued parameters: typed rows reach the capability, and a
//! row that does not fit the declared columns fails compilation.
#![allow(missing_docs)]

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, ColumnKind, CompiledRule, EngineError,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, TableColumn, compile,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{DefinitionPackage, RuleSetPackage};
use serde_json::{Value, json};

const COLUMNS: &[TableColumn] = &[
    TableColumn::required("space_type", ColumnKind::TextPattern),
    TableColumn::required("minimum_area", ColumnKind::Quantity),
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::optional("count", ColumnKind::Integer),
    TableColumn::optional("ratio", ColumnKind::Number),
    TableColumn::optional("strict", ColumnKind::Boolean),
    TableColumn::optional("scope", ColumnKind::Selector),
    TableColumn::optional("reference", ColumnKind::Reference),
    TableColumn::optional("from", ColumnKind::Date),
    TableColumn::optional("until", ColumnKind::DateTime),
];

struct Limits;
impl RuleCapability for Limits {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "limits",
            ParameterType::Table(COLUMNS),
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        CapabilityEvaluation::evaluated(vec![])
    }
}

fn text(value: &str) -> Value {
    json!({"default": value, "translations": {}})
}

fn column(id: &str, kind: &str, required: bool) -> Value {
    let mut column = json!({"id": id, "name": text(id), "kind": kind, "required": required});
    if kind == "quantity" {
        column["unitDimension"] = json!("area");
    }
    column
}

/// The definition's columns, as a package author declares [`COLUMNS`].
fn declared_columns() -> Vec<Value> {
    vec![
        column("space_type", "textPattern", true),
        column("minimum_area", "quantity", true),
        column("label", "string", false),
        column("count", "integer", false),
        column("ratio", "number", false),
        column("strict", "boolean", false),
        column("scope", "selector", false),
        column("reference", "reference", false),
        column("from", "date", false),
        column("until", "dateTime", false),
    ]
}

fn row(space_type: &str, area: f64) -> Value {
    json!({
        "space_type": {"type": "string", "value": space_type},
        "minimum_area": {"type": "quantity", "value": area, "unit": "m2"},
    })
}

/// Packages whose one definition takes `limits` with `parameter` merged in,
/// bound in the rule to `rows` (or left unbound when `None`).
fn packages(parameter: &Value, rows: Option<Vec<Value>>) -> (DefinitionPackage, RuleSetPackage) {
    let mut definitions: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let mut limits = json!({
        "id": "limits",
        "name": text("Limits"),
        "kind": "table",
        "required": true,
        "allowedValues": [],
        "columns": declared_columns(),
    });
    for (key, value) in parameter.as_object().unwrap() {
        limits[key] = value.clone();
    }
    definitions["definitions"]["axioval:example.property-exists"]["parameters"] =
        json!({ "limits": limits });
    let mut ruleset: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    ruleset["root"]["rules"][0]["parameters"] = match rows {
        Some(rows) => json!({"limits": {"type": "table", "value": rows}}),
        None => json!({}),
    };
    (
        serde_json::from_value(definitions).unwrap(),
        serde_json::from_value(ruleset).unwrap(),
    )
}

fn bind(parameter: &Value, rows: Option<Vec<Value>>) -> Result<Vec<CompiledRule>, EngineError> {
    let (definitions, ruleset) = packages(parameter, rows);
    let registry = CapabilityRegistry::new().register(Limits).unwrap();
    compile(&registry, &[definitions], &ruleset).map(|plan| plan.rules().to_vec())
}

fn row_error(rows: Vec<Value>) -> (usize, String) {
    match bind(&json!({}), Some(rows)).unwrap_err() {
        EngineError::InvalidTableRow {
            parameter,
            row,
            detail,
            ..
        } => {
            assert_eq!(parameter, "limits");
            (row, detail)
        }
        other => panic!("expected a table-row error, got {other:?}"),
    }
}

fn contract_error(parameter: &Value) -> String {
    match bind(parameter, Some(vec![row("Office*", 10.0)])).unwrap_err() {
        EngineError::CapabilityContract { detail, .. } => detail,
        other => panic!("expected a contract error, got {other:?}"),
    }
}

#[test]
fn a_capability_receives_the_typed_rows_in_declared_order() {
    let mut full = row("Office", 12.0);
    full["label"] = json!({"type": "string", "value": "single office"});
    full["count"] = json!({"type": "integer", "value": 2});
    full["ratio"] = json!({"type": "number", "value": 0.5});
    full["strict"] = json!({"type": "boolean", "value": true});
    full["scope"] = json!({"type": "selector", "value": {
        "kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true
    }});
    full["reference"] = json!({"type": "reference", "value": "axioval:example.office"});
    full["from"] = json!({"type": "date", "value": "2026-01-01"});
    full["until"] = json!({"type": "dateTime", "value": "2026-12-31T23:00:00Z"});
    let rules = bind(&json!({}), Some(vec![full, row("Office*", 10.0)])).unwrap();
    let ParameterValue::Table { value: rows } = &rules[0].parameters["limits"] else {
        panic!("limits is a table");
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].len(), 10);
    assert_eq!(
        rows[0]["from"],
        ParameterValue::Date {
            value: "2026-01-01".parse().unwrap()
        }
    );
    assert_eq!(
        rows[1]["space_type"],
        ParameterValue::String {
            value: "Office*".into()
        }
    );
    assert_eq!(
        rows[1]["minimum_area"],
        ParameterValue::Quantity {
            value: 10.0,
            unit: "m2".into()
        }
    );
}

#[test]
fn an_empty_table_and_a_default_table_bind() {
    assert!(bind(&json!({}), Some(vec![])).is_ok());
    let default = json!({
        "required": false,
        "defaultValue": {"type": "table", "value": [row("*", 5.0)]},
    });
    let (definitions, ruleset) = packages(&default, None);
    let registry = CapabilityRegistry::new().register(OptionalLimits).unwrap();
    let plan = compile(&registry, &[definitions], &ruleset).unwrap();
    assert!(matches!(
        &plan.rules()[0].parameters["limits"],
        ParameterValue::Table { value } if value.len() == 1
    ));
}

struct OptionalLimits;
impl RuleCapability for OptionalLimits {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::optional(
            "limits",
            ParameterType::Table(COLUMNS),
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        CapabilityEvaluation::evaluated(vec![])
    }
}

#[test]
fn a_default_table_is_checked_like_a_bound_one() {
    let mut bad = row("*", 5.0);
    bad["extra"] = json!({"type": "string", "value": "x"});
    let default = json!({
        "required": false,
        "defaultValue": {"type": "table", "value": [bad]},
    });
    let (definitions, ruleset) = packages(&default, None);
    let registry = CapabilityRegistry::new().register(OptionalLimits).unwrap();
    assert!(matches!(
        compile(&registry, &[definitions], &ruleset),
        Err(EngineError::InvalidTableRow { row: 0, .. })
    ));
}

#[test]
fn an_unknown_column_fails_compilation() {
    let mut extra = row("Office", 12.0);
    extra["colour"] = json!({"type": "string", "value": "red"});
    let (row, detail) = row_error(vec![row("*", 1.0), extra]);
    assert_eq!(row, 1);
    assert_eq!(detail, "unknown column `colour`");
}

#[test]
fn a_cell_of_another_kind_fails_compilation() {
    for (column, cell, kind) in [
        (
            "minimum_area",
            json!({"type": "number", "value": 12.0}),
            "quantity",
        ),
        (
            "space_type",
            json!({"type": "integer", "value": 1}),
            "textPattern",
        ),
        (
            "label",
            json!({"type": "enum", "value": "office"}),
            "string",
        ),
        ("count", json!({"type": "number", "value": 2.0}), "integer"),
        ("ratio", json!({"type": "integer", "value": 1}), "number"),
        (
            "strict",
            json!({"type": "string", "value": "true"}),
            "boolean",
        ),
        (
            "scope",
            json!({"type": "string", "value": "walls"}),
            "selector",
        ),
        (
            "reference",
            json!({"type": "string", "value": "x"}),
            "reference",
        ),
        ("label", json!({"type": "table", "value": []}), "string"),
        (
            "from",
            json!({"type": "string", "value": "2026-01-01"}),
            "date",
        ),
        (
            "until",
            json!({"type": "date", "value": "2026-01-01"}),
            "dateTime",
        ),
    ] {
        let mut cells = row("Office", 12.0);
        cells[column] = cell;
        let (row, detail) = row_error(vec![cells]);
        assert_eq!(row, 0);
        assert_eq!(detail, format!("column `{column}` takes a {kind} cell"));
    }
}

#[test]
fn a_missing_required_cell_fails_compilation() {
    let mut cells = row("Office", 12.0);
    cells.as_object_mut().unwrap().remove("minimum_area");
    let (_, detail) = row_error(vec![cells]);
    assert_eq!(detail, "required column `minimum_area` is empty");
}

#[test]
fn a_pattern_ending_in_a_lone_backslash_fails_compilation() {
    let (_, detail) = row_error(vec![row("Office\\", 12.0)]);
    assert_eq!(detail, "column `space_type` takes a textPattern cell");
    assert!(bind(&json!({}), Some(vec![row("Office\\*", 12.0)])).is_ok());
}

#[test]
fn a_selector_cell_names_only_known_concepts() {
    let mut cells = row("Office", 12.0);
    cells["scope"] = json!({"type": "selector", "value": {
        "kind": "entityType", "objectType": "axioval:example.unknown", "includeSubtypes": true
    }});
    assert!(matches!(
        bind(&json!({}), Some(vec![cells])),
        Err(EngineError::UnknownConcept { .. })
    ));
}

#[test]
fn a_table_value_for_a_scalar_parameter_fails_compilation() {
    let (definitions, mut ruleset) = serde_json::from_str::<Value>(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .map(|value| serde_json::from_value::<DefinitionPackage>(value).unwrap())
    .map(|definitions| {
        let ruleset: RuleSetPackage = serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/ruleset.json"
        ))
        .unwrap();
        (definitions, ruleset)
    })
    .unwrap();
    ruleset.root.rules[0]
        .parameters
        .insert("property".into(), ParameterValue::Table { value: vec![] });
    let registry = CapabilityRegistry::new().register(Scalar).unwrap();
    assert!(matches!(
        compile(&registry, &[definitions], &ruleset),
        Err(EngineError::InvalidParameterType { .. })
    ));
}

struct Scalar;
impl RuleCapability for Scalar {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        CapabilityEvaluation::evaluated(vec![])
    }
}

#[test]
fn definition_columns_must_be_the_capabilitys() {
    let mut reordered = declared_columns();
    reordered.reverse();
    assert!(bind(&json!({ "columns": reordered }), Some(vec![row("*", 1.0)])).is_ok());

    let mut missing = declared_columns();
    missing.pop();
    let mut other_kind = declared_columns();
    other_kind[2]["kind"] = json!("textPattern");
    let mut optional = declared_columns();
    optional[0]["required"] = json!(false);
    let mut duplicate = declared_columns();
    duplicate[7] = column("label", "string", false);
    for columns in [missing, other_kind, optional, duplicate] {
        assert_eq!(
            contract_error(&json!({ "columns": columns })),
            "table parameter `limits` columns differ"
        );
    }
}

#[test]
fn only_a_table_declares_columns_and_a_table_has_no_allowed_values() {
    assert_eq!(
        contract_error(&json!({
            "allowedValues": [{"type": "table", "value": [row("Office*", 10.0)]}],
        })),
        "table parameter `limits` must not declare allowedValues"
    );
    assert_eq!(
        contract_error(&json!({ "kind": "stringList" })),
        "parameter signature differs"
    );
    let (definitions, ruleset) = serde_json::from_str::<Value>(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .map(|mut value| {
        value["definitions"]["axioval:example.property-exists"]["parameters"]["property"]
            ["columns"] = json!([column("label", "string", false)]);
        let ruleset: RuleSetPackage = serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/ruleset.json"
        ))
        .unwrap();
        (
            serde_json::from_value::<DefinitionPackage>(value).unwrap(),
            ruleset,
        )
    })
    .unwrap();
    let registry = CapabilityRegistry::new().register(Scalar).unwrap();
    assert!(matches!(
        compile(&registry, &[definitions], &ruleset),
        Err(EngineError::CapabilityContract { detail, .. })
            if detail == "only a table parameter declares columns, not `property`"
    ));
}
