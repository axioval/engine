//! Expression operators norms state their values with, run end to end in
//! `axioval:capability.expression` rules: rounding down or up to a step and
//! interpolating in parameter tables.
#![allow(missing_docs)]

mod common;

use axioval_engine::CapabilityRegistry;
use axioval_ir::contract::{ParameterDefinition, PropertyValueKind};
use axioval_ir::{DefinitionPackage, NotEvaluatedReason, PropertyValue, QuantityDimension, Report};
use axioval_rules::register_builtins;
use common::Model;
use common::runtime::{definition, definitions, entity, plan, rule, run, session};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Definitions where `Cover` and `Span` are lengths and `Class` text, with
/// the expression definition declaring `extra` authored parameters.
fn vocabulary(registry: &CapabilityRegistry, extra: &[Value]) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION],
        &["slab"],
        &["Cover", "Span", "Class"],
        &["Pset"],
    );
    for name in ["t.Cover", "t.Span"] {
        let property = package.properties.get_mut(name).unwrap();
        property.value_kind = PropertyValueKind::Quantity;
        property.unit_dimension = Some("length".into());
    }
    let expression = package
        .definitions
        .get_mut(&definition(EXPRESSION))
        .unwrap();
    for parameter in extra {
        let parameter: ParameterDefinition = serde_json::from_value(parameter.clone()).unwrap();
        expression
            .parameters
            .insert(parameter.id.clone(), parameter);
    }
    package
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": format!("t.{name}")})
}

fn quantity(value: f64, unit: &str) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": unit}})
}

fn length(metres: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value: metres,
        dimension: QuantityDimension::Length,
    }
}

/// Slabs with a cover, a span and an exposure class.
fn slabs(slabs: &[(&str, f64, f64, &str)]) -> Model {
    let mut model = Model::default();
    for &(local, cover, span, class) in slabs {
        model = model
            .object(local, "slab")
            .value(local, "Pset", "Cover", length(cover))
            .value(local, "Pset", "Span", length(span))
            .value(local, "Pset", "Class", PropertyValue::String(class.into()));
    }
    model
}

fn requirement_rule(requirement: &Value, parameters: Value) -> Value {
    let mut all = json!({"requirement": {"type": "expression", "value": requirement}});
    if let (Value::Object(all), Value::Object(parameters)) = (&mut all, parameters) {
        all.extend(parameters);
    }
    rule("r", EXPRESSION, "error", entity("slab"), all, json!({}))
}

fn check(package: &DefinitionPackage, rule: Value, model: Model) -> Report {
    let registry = registry();
    let plan = plan(&registry, package, vec![rule]).unwrap();
    run(registry, plan, &session(model), |runtime| runtime).unwrap()
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
                    .map(|object| object.local_id.to_string())
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
fn a_cover_rounded_down_to_whole_centimetres_is_judged_as_the_norm_states_it() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    // Counted in whole centimetres, rounded down: 39.9 mm counts as 30 mm,
    // and 40 mm as stated is 40 mm.
    let requirement = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": {"kind": "floor", "label": "counted cover", "operand": property("Cover"),
                 "step": quantity(1.0, "cm")},
        "right": quantity(40.0, "mm")});
    let model = slabs(&[
        ("exact", 0.040, 4.0, "XC1"),
        ("short", 0.0399, 4.0, "XC1"),
        ("ample", 0.047, 4.0, "XC1"),
    ]);
    let report = check(&package, requirement_rule(&requirement, json!({})), model);
    assert_eq!(subjects(&report), ["short"]);
    assert!(open(&report).is_empty(), "{:?}", open(&report));
    let finding = &report.findings()[0];
    let explanation = serde_json::to_string(&finding.explanation).unwrap();
    assert!(explanation.contains("0.03 m"), "{explanation}");

    // A step of another dimension fails compilation with its path.
    let wrong = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": {"kind": "ceil", "operand": property("Cover"),
                 "step": {"kind": "literal", "value": {"type": "integer", "value": 1}}},
        "right": quantity(35.0, "mm")});
    let error = plan(
        &registry,
        &package,
        vec![requirement_rule(&wrong, json!({}))],
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("requirement.compare.left"),
        "{error}"
    );
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

fn length_column(id: &str) -> Value {
    json!({"id": id, "name": text(id), "kind": "quantity", "required": false,
        "unitDimension": "length"})
}

/// The minimum cover by span: 20 mm at 2 m, 30 mm at 4 m.
fn cover_by_span() -> (Value, Value) {
    let declared = json!({"id": "cover_by_span", "name": text("cover by span"), "kind": "table",
        "required": true, "columns": [length_column("span"), length_column("cover")]});
    let row = |span: f64, cover: f64| {
        json!({"span": {"type": "quantity", "value": span, "unit": "m"},
            "cover": {"type": "quantity", "value": cover, "unit": "mm"}})
    };
    let rows =
        json!({"cover_by_span": {"type": "table", "value": [row(2.0, 20.0), row(4.0, 30.0)]}});
    (declared, rows)
}

fn interpolated_cover_rule(clamp: bool, rows: Value) -> Value {
    let mut required = json!({"kind": "interpolate", "label": "required cover",
        "table": "cover_by_span", "keyColumn": "span", "key": property("Span"),
        "column": "cover"});
    if clamp {
        required["clamp"] = json!(true);
    }
    let requirement = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": property("Cover"), "right": required});
    requirement_rule(&requirement, rows)
}

#[test]
fn a_cover_is_judged_against_a_requirement_interpolated_by_span() {
    let registry = registry();
    let (declared, rows) = cover_by_span();
    let package = vocabulary(&registry, &[declared]);
    // At 3 m the requirement is 25 mm; 5 m lies beyond the table.
    let model = || {
        slabs(&[
            ("enough", 0.026, 3.0, "XC1"),
            ("thin", 0.024, 3.0, "XC1"),
            ("long", 0.031, 5.0, "XC1"),
        ])
    };
    let report = check(
        &package,
        interpolated_cover_rule(false, rows.clone()),
        model(),
    );
    assert_eq!(subjects(&report), ["thin"]);
    let open = open(&report);
    assert_eq!(open.len(), 1, "{open:?}");
    let (object, reason, message) = &open[0];
    assert_eq!(object, "long");
    assert_eq!(*reason, NotEvaluatedReason::IncompleteEvidence);
    assert!(
        message.contains("`span` 5 m lies beyond its last row's, 4 m")
            && message.contains("`clamp`"),
        "{message}"
    );
    // Clamped, the long slab is held to the last row's 30 mm.
    let report = check(&package, interpolated_cover_rule(true, rows), model());
    assert_eq!(subjects(&report), ["thin"]);
    assert!(report.not_evaluated.is_empty());

    // A key of another dimension fails compilation with its path.
    let (_, rows) = cover_by_span();
    let wrong = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": property("Cover"), "right": {"kind": "interpolate", "table": "cover_by_span",
            "keyColumn": "span", "key": {"kind": "literal", "value": {"type": "number", "value": 3.0}},
            "column": "cover"}});
    let error = plan(&registry, &package, vec![requirement_rule(&wrong, rows)]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requirement.compare.right.interpolate.key"),
        "{error}"
    );
}
