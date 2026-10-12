//! Catalogues of material data a ruleset declares and the host supplies
//! (#322), read by `axioval:capability.expression` rules as values the
//! catalogue states: joined to each object by its material's name.
#![allow(missing_docs)]

mod common;

use axioval_engine::{CapabilityRegistry, EngineError, ExecutionPlan, SuppliedCatalogue, compile};
use axioval_ir::contract::PropertyValueKind;
use axioval_ir::{DefinitionPackage, NotEvaluatedReason, PropertyValue, QuantityDimension, Report};
use axioval_rules::register_builtins;
use common::Model;
use common::runtime::{definitions, entity, rule, ruleset, run, session};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

/// Three invented materials: one with an empty λ cell.
const MATERIALS: &str = "name,λ (W/(m·K)),density\n\
                         ex-clay-brick,0.81,1800\n\
                         ex-foam-board,0.035,30\n\
                         ex-wood-fibre,,160\n";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION],
        &["wall"],
        &["Material", "Thickness", "Layers"],
        &["Pset"],
    );
    let thickness = package.properties.get_mut("t.Thickness").unwrap();
    thickness.value_kind = PropertyValueKind::Quantity;
    thickness.unit_dimension = Some("length".into());
    package
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

/// The catalogue `materials`, keyed by `name`.
fn materials() -> Value {
    json!({ "materials": {
        "id": "materials",
        "name": text("Materials"),
        "key": "name",
        "columns": [
            { "id": "name", "kind": "string" },
            { "id": "lambda", "header": "λ (W/(m·K))", "kind": "quantity", "unit": "W/m·K" },
            { "id": "density", "kind": "quantity", "unit": "kg/m³" },
        ],
    } })
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": format!("t.{name}")})
}

/// The `column` value of the wall's material.
fn catalogue(column: &str) -> Value {
    json!({"kind": "catalogue", "catalogue": "materials", "key": property("Material"),
           "column": column, "label": column})
}

fn quantity(value: f64, unit: &str) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}})
}

/// λ of the wall's material at most 0.04 W/(m·K).
fn insulating() -> Value {
    json!({"kind": "compare", "operator": "lessThanOrEquals",
           "left": catalogue("lambda"), "right": quantity(0.04, "W/m·K")})
}

fn requirement_rule(requirement: &Value) -> Value {
    rule(
        "r",
        EXPRESSION,
        "error",
        entity("wall"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )
}

/// Walls 0.2 m thick, by their material's name; `None` states no material.
fn walls(walls: &[(&str, Option<&str>)]) -> Model {
    let mut model = Model::default();
    for &(local, material) in walls {
        model = model
            .object(local, "wall")
            .value(
                local,
                "Pset",
                "Thickness",
                PropertyValue::Quantity {
                    value: 0.2,
                    dimension: QuantityDimension::Length,
                },
            )
            .value(
                local,
                "Pset",
                "Material",
                material.map_or(PropertyValue::Null, |name| {
                    PropertyValue::String(name.into())
                }),
            );
    }
    model
}

fn compiled(rules: Vec<Value>, extra: &Value) -> Result<ExecutionPlan, EngineError> {
    let registry = registry();
    let mut set = ruleset(rules);
    set.catalogues = serde_json::from_value(materials()).unwrap();
    if let Some(values) = extra.get("values") {
        set.values = serde_json::from_value(values.clone()).unwrap();
    }
    compile(&registry, &[vocabulary(&registry)], &set)
}

fn supplied(plan: &mut ExecutionPlan) {
    let definition = plan.catalogues()[0].clone();
    let catalogue = SuppliedCatalogue::read(
        &definition,
        MATERIALS.as_bytes(),
        "materials.csv",
        None,
        Some("2026-1"),
    )
    .unwrap();
    plan.supply_catalogue("materials", catalogue).unwrap();
}

fn check(plan: ExecutionPlan, model: Model) -> Report {
    run(registry(), plan, &session(model), |runtime| runtime).unwrap()
}

fn subjects(report: &Report) -> Vec<String> {
    let mut subjects: Vec<String> = report.findings().iter().map(common::subject).collect();
    subjects.sort();
    subjects
}

fn open(report: &Report) -> Vec<(String, NotEvaluatedReason, String)> {
    let mut open: Vec<_> = report
        .not_evaluated
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map(|object| object.local_id.clone())
                    .unwrap_or_default(),
                outcome.reason.clone(),
                outcome.message.clone(),
            )
        })
        .collect();
    open.sort_by(|a, b| a.0.cmp(&b.0));
    open
}

#[test]
fn a_catalogue_joined_by_material_name_gives_lambda_to_an_expression() {
    let mut plan = compiled(vec![requirement_rule(&insulating())], &Value::Null).unwrap();
    supplied(&mut plan);
    let report = check(
        plan,
        walls(&[
            ("brick", Some("ex-clay-brick")),
            ("foam", Some("ex-foam-board")),
            ("mystery", Some("ex-mystery-board")),
        ]),
    );
    // 0.81 W/(m·K) exceeds 0.04; 0.035 meets it.
    assert_eq!(subjects(&report), ["brick"]);
    let finding = &report.findings()[0];
    assert!(
        finding
            .message
            .contains("catalogue materials[ex-clay-brick].lambda = 0.81 m·kg·s⁻³·K⁻¹"),
        "{}",
        finding.message
    );
    // The value is the catalogue's statement, cited with its id, version,
    // file digest and entry, never as the model's.
    let cited: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .filter(|locator| locator.starts_with("axioval:catalogue;"))
        .collect();
    assert_eq!(cited.len(), 1, "{:?}", finding.evidence);
    assert!(
        cited[0].starts_with("axioval:catalogue;id=materials;version=2026-1;sha256=")
            && cited[0].ends_with("#entry=ex-clay-brick"),
        "{}",
        cited[0]
    );
    // A material the catalogue has no entry for is not evaluated, the
    // reason naming the key; never given a default.
    let open = open(&report);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].0, "mystery");
    assert_eq!(open[0].1, NotEvaluatedReason::IncompleteEvidence);
    assert!(
        open[0].2.contains(
            "catalogue `materials` (version 2026-1) has no entry whose `name` is \
             `ex-mystery-board`"
        ),
        "{}",
        open[0].2
    );
}

#[test]
fn an_empty_cell_is_null_and_never_not_evaluated() {
    let mut plan = compiled(
        vec![requirement_rule(&json!({"kind": "isUndefined",
                                      "operand": catalogue("lambda")}))],
        &Value::Null,
    )
    .unwrap();
    supplied(&mut plan);
    let report = check(
        plan,
        walls(&[
            ("fibre", Some("ex-wood-fibre")),
            ("foam", Some("ex-foam-board")),
            ("none", None),
        ]),
    );
    // The catalogue states no λ for wood fibre, and a wall stating no
    // material names no entry: both `null`. The foam's λ is stated.
    assert_eq!(subjects(&report), ["foam"]);
    assert!(open(&report).is_empty(), "{:?}", open(&report));
}

#[test]
fn catalogue_values_compose_with_arithmetic_and_derived_values() {
    // A thermal resistance d / λ, derived per wall, at least 2.5 m²·K/W.
    let values = json!({"values": {"resistance": {
        "name": text("Thermal resistance"),
        "expression": {"kind": "divide", "left": property("Thickness"),
                       "right": catalogue("lambda")},
    }}});
    let requirement = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": {"kind": "derived", "name": "resistance"},
        "right": quantity(2.5, "m²·K/W")});
    let mut plan = compiled(vec![requirement_rule(&requirement)], &values).unwrap();
    supplied(&mut plan);
    let report = check(
        plan,
        walls(&[
            ("brick", Some("ex-clay-brick")),
            ("foam", Some("ex-foam-board")),
            ("mystery", Some("ex-mystery-board")),
        ]),
    );
    // 0.2 / 0.81 ≈ 0.25 falls short; 0.2 / 0.035 ≈ 5.7 meets it.
    assert_eq!(subjects(&report), ["brick"]);
    let open = open(&report);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(open[0].2.contains("`ex-mystery-board`"), "{}", open[0].2);
}

#[test]
fn a_catalogue_the_host_did_not_supply_leaves_every_reading_object_open() {
    let plan = compiled(vec![requirement_rule(&insulating())], &Value::Null).unwrap();
    let report = check(plan, walls(&[("foam", Some("ex-foam-board"))]));
    assert!(report.findings().is_empty());
    let open = open(&report);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(
        open[0].2.contains("none were supplied") && open[0].2.contains("`ex-foam-board`"),
        "{}",
        open[0].2
    );
}

#[test]
fn reads_and_declarations_are_checked_when_compiled_and_supplied() {
    let refused = |requirement: Value| {
        compiled(vec![requirement_rule(&requirement)], &Value::Null)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    };
    let read = |catalogue: &str, key: Value, column: &str| {
        json!({"kind": "isDefined", "operand": {"kind": "catalogue", "catalogue": catalogue,
                                                 "key": key, "column": column}})
    };
    assert!(
        refused(read("products", property("Material"), "lambda"))
            .contains("no catalogue `products`")
    );
    assert!(
        refused(read("materials", property("Material"), "colour"))
            .contains("no value column `colour`")
    );
    // The key column is the key, not a value.
    assert!(
        refused(read("materials", property("Material"), "name")).contains("no value column `name`")
    );
    assert!(refused(read("materials", property("Thickness"), "lambda")).contains("text is needed"));
    // λ is a thermal conductivity, never a length.
    assert!(
        !refused(
            json!({"kind": "compare", "operator": "lessThan", "left": catalogue("lambda"),
                        "right": quantity(1.0, "m")})
        )
        .is_empty()
    );

    let mut plan = compiled(vec![requirement_rule(&insulating())], &Value::Null).unwrap();
    let definition = plan.catalogues()[0].clone();
    let twice = "name,λ (W/(m·K)),density\nex-a,1,1\nex-a,2,2\n";
    let catalogue =
        SuppliedCatalogue::read(&definition, twice.as_bytes(), "m.csv", None, None).unwrap();
    assert!(matches!(
        plan.supply_catalogue("materials", catalogue),
        Err(EngineError::InvalidCatalogue { detail, .. }) if detail.contains("both name the key `ex-a`")
    ));
    assert!(matches!(
        plan.supply_catalogue(
            "products",
            SuppliedCatalogue::new("p.csv", "00", None, Vec::new())
        ),
        Err(EngineError::InvalidCatalogue { .. })
    ));
    // A header the declaration does not name is refused when read.
    assert!(
        SuppliedCatalogue::read(&definition, b"name,lambda\nex-a,1\n", "m.csv", None, None)
            .is_err()
    );

    let registry = registry();
    let mut set = ruleset(vec![requirement_rule(&insulating())]);
    let mut declared = materials();
    declared["materials"]["key"] = json!("density");
    set.catalogues = serde_json::from_value(declared).unwrap();
    assert!(matches!(
        compile(&registry, &[vocabulary(&registry)], &set),
        Err(EngineError::InvalidCatalogue { detail, .. }) if detail.contains("not string")
    ));
}
