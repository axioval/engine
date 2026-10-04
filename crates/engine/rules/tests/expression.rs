//! `axioval:capability.expression`: a requirement stated as an expression
//! over each selected object.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ElevationInterval, EngineError, EvidenceSession, VerticalExtent,
    VerticalExtentError, VerticalExtentService, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterDefinition, PropertyValueKind};
use axioval_ir::{
    DefinitionPackage, Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension,
    Report,
};
use axioval_rules::register_builtins;
use common::runtime::{definition, definitions, entity, plan, rule, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";
const PREDICATE: &str = "axioval:capability.property-predicate";
const TAKEOFF: &str = "axioval:capability.quantity-takeoff";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

/// Definitions where `Cover` is a quantity and `Class` text, with the
/// expression definition declaring `extra` authored parameters.
fn vocabulary(registry: &CapabilityRegistry, extra: &[Value]) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION, PREDICATE, TAKEOFF],
        &["slab", "pipe"],
        &["Cover", "Class"],
        &["Pset"],
    );
    let cover = package.properties.get_mut("t.Cover").unwrap();
    cover.value_kind = PropertyValueKind::Quantity;
    cover.unit_dimension = Some("length".into());
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

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

fn mm(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "mm"}})
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": format!("t.{name}")})
}

fn class_is(class: &str) -> Value {
    json!({"kind": "compare", "operator": "equals", "left": property("Class"),
        "right": {"kind": "literal", "value": {"type": "string", "value": class}}})
}

/// Slabs with an exposure class and a concrete cover.
fn slabs() -> Model {
    let mut model = Model::default();
    for (local, class, cover) in [
        ("s1", "XC4", 0.045),
        ("s2", "XC4", 0.035),
        ("s3", "XC3", 0.030),
        ("s4", "XC1", 0.030),
        ("s5", "XC1", 0.020),
    ] {
        model = model
            .object(local, "slab")
            .value(local, "Pset", "Class", PropertyValue::String(class.into()))
            .value(
                local,
                "Pset",
                "Cover",
                PropertyValue::Quantity {
                    value: cover,
                    dimension: QuantityDimension::Length,
                },
            );
    }
    model
}

fn subjects(report: &Report, rule: &str) -> Vec<String> {
    let mut subjects: Vec<String> = report
        .findings()
        .iter()
        .filter(|finding| finding.rule_id.to_string() == rule)
        .map(common::subject)
        .collect();
    subjects.sort();
    subjects
}

fn check(package: &DefinitionPackage, rules: Vec<Value>, session: &EvidenceSession) -> Report {
    let registry = registry();
    let plan = plan(&registry, package, rules).unwrap();
    run(registry, plan, session, |runtime| runtime).unwrap()
}

/// Cover at least 40 mm for XC4, 35 mm for XC3, 25 mm otherwise.
fn cover_rule() -> Value {
    let required = json!({"kind": "if", "label": "required cover",
        "branches": [
            {"when": class_is("XC4"), "then": mm(40.0)},
            {"when": class_is("XC3"), "then": mm(35.0)}],
        "else": mm(25.0)});
    rule(
        "cover",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": {
            "kind": "compare", "operator": "greaterThanOrEquals", "label": "cover",
            "left": property("Cover"), "right": required}}}),
        json!({}),
    )
}

/// The same requirement as three single rules, one per exposure class.
fn single_rules() -> Vec<Value> {
    let class = |operator: &str, value: &str| {
        json!({"kind": "property", "propertySet": "t.Pset", "property": "t.Class",
            "operator": operator, "value": {"type": "string", "value": value}})
    };
    let predicate = |id: &str, selector: Value, minimum: f64| {
        rule(
            id,
            PREDICATE,
            "error",
            json!({"kind": "allOf", "operands": [entity("slab"), selector]}),
            json!({
                "property_set": {"type": "string", "value": "t.Pset"},
                "property": {"type": "string", "value": "t.Cover"},
                "operator": {"type": "string", "value": "greater_or_equal"},
                "quantity": {"type": "quantity", "value": minimum, "unit": "mm"},
            }),
            json!({}),
        )
    };
    vec![
        predicate("xc4", class("equals", "XC4"), 40.0),
        predicate("xc3", class("equals", "XC3"), 35.0),
        predicate(
            "other",
            json!({"kind": "allOf", "operands": [class("notEquals", "XC4"), class("notEquals", "XC3")]}),
            25.0,
        ),
    ]
}

#[test]
fn a_three_branch_requirement_finds_what_three_single_rules_find() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let session = session(slabs());
    let combined = check(&package, vec![cover_rule()], &session);
    let separate = check(&package, single_rules(), &session);
    let mut single: Vec<String> = ["xc4", "xc3", "other"]
        .iter()
        .flat_map(|rule| subjects(&separate, rule))
        .collect();
    single.sort();
    assert_eq!(subjects(&combined, "cover"), single);
    assert_eq!(single, ["s2", "s3", "s5"]);
    assert!(combined.not_evaluated.is_empty());
    let message = &combined
        .findings()
        .iter()
        .find(|finding| common::subject(finding) == "s2")
        .unwrap()
        .message;
    assert!(message.contains("`cover` is false"), "{message}");
    assert!(message.contains("t.Pset.t.Cover = 0.035 m"), "{message}");
    assert!(message.contains("t.Pset.t.Class = `XC4`"), "{message}");
}

/// Bottom and top elevations per object.
struct Extents(BTreeMap<ObjectId, [(f64, f64); 2]>);

impl VerticalExtentService for Extents {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let [bottom, top] = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("extent:{object}"));
        evidence.exact =
            bottom.0.to_bits() == bottom.1.to_bits() && top.0.to_bits() == top.1.to_bits();
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom.0, bottom.1)?,
            ElevationInterval::try_new(top.0, top.1)?,
            evidence,
        )
    }
}

#[test]
fn an_undecidable_measured_input_is_not_evaluated_never_passed() {
    let model = Model::default()
        .object("p40", "pipe")
        .object("p100", "pipe")
        .object("p50", "pipe");
    let extents = Extents(
        [
            ("p40", [(1.0, 1.0), (1.04, 1.04)]),
            ("p100", [(1.0, 1.0), (1.1, 1.1)]),
            ("p50", [(0.9975, 1.0025), (1.0475, 1.0525)]),
        ]
        .into_iter()
        .map(|(local, extent)| (id(local), extent))
        .collect(),
    );
    let session = session(model)
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(extents)),
            &[snapshot()],
        )
        .unwrap();
    let small = rule(
        "small",
        EXPRESSION,
        "error",
        entity("pipe"),
        json!({"requirement": {"type": "expression", "value": {
            "kind": "compare", "operator": "lessThan",
            "left": {"kind": "property", "propertySet": "axioval:measured", "property": "extent_z"},
            "right": mm(50.0)}}}),
        json!({}),
    );
    let registry = registry();
    let report = check(&vocabulary(&registry, &[]), vec![small], &session);
    assert_eq!(subjects(&report, "small"), ["p100"]);
    let open: Vec<_> = report
        .not_evaluated
        .iter()
        .map(|outcome| (outcome.object_id().cloned(), outcome.reason.clone()))
        .collect();
    assert_eq!(
        open,
        [(Some(id("p50")), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        report.not_evaluated[0].message.contains("`requirement`"),
        "{}",
        report.not_evaluated[0].message
    );
}

fn expression_rule(requirement: &Value, extra: Value) -> Value {
    let mut parameters = json!({"requirement": {"type": "expression", "value": requirement}});
    if let (Value::Object(parameters), Value::Object(extra)) = (&mut parameters, extra) {
        parameters.extend(extra);
    }
    rule(
        "r",
        EXPRESSION,
        "error",
        entity("slab"),
        parameters,
        json!({}),
    )
}

fn compiled(package: &DefinitionPackage, rule: Value) -> Result<(), EngineError> {
    plan(&registry(), package, vec![rule]).map(drop)
}

#[test]
fn ill_typed_or_unknown_expressions_fail_compilation_with_their_path() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let cases = [
        (
            json!({"kind": "and", "operands": [class_is("XC4"), property("Height")]}),
            "requirement.and[1]",
        ),
        (
            json!({"kind": "compare", "operator": "lessThan", "left": property("Cover"),
                "right": {"kind": "literal", "value": {"type": "number", "value": 1.0}}}),
            "requirement",
        ),
        (mm(40.0), "requirement"),
        (
            json!({"kind": "compare", "operator": "lessThan",
                "left": {"kind": "property", "propertySet": "axioval:measured", "property": "height"},
                "right": mm(1.0)}),
            "requirement.compare.left",
        ),
    ];
    for (requirement, path) in cases {
        match compiled(&package, expression_rule(&requirement, json!({}))) {
            Err(EngineError::InvalidExpression { path: found, .. }) => {
                assert_eq!(found, path, "{requirement}");
            }
            other => panic!("{requirement}: {other:?}"),
        }
    }
    // Cover is a quantity: the type checker accepts a length bound.
    assert!(compiled(
        &package,
        expression_rule(
            &json!({"kind": "compare", "operator": "lessThan", "left": property("Cover"), "right": mm(1.0)}),
            json!({})
        )
    )
    .is_ok());
}

fn table_parameter() -> Value {
    let column =
        |id: &str, kind: &str| json!({"id": id, "name": text(id), "kind": kind, "required": false});
    json!({"id": "minimum_cover", "name": text("minimum cover"), "kind": "table", "required": true,
        "columns": [column("class", "textPattern"), {"id": "cover", "name": text("cover"), "kind": "quantity", "required": false, "unitDimension": "length"}]})
}

#[test]
fn authored_parameters_are_read_and_checked() {
    let registry = registry();
    let tolerance =
        json!({"id": "tolerance", "name": text("tolerance"), "kind": "quantity", "required": true});
    let package = vocabulary(&registry, &[table_parameter(), tolerance]);
    let row = |class: &str, cover: f64| json!({"class": {"type": "string", "value": class}, "cover": {"type": "quantity", "value": cover, "unit": "mm"}});
    let required = json!({"kind": "lookup", "table": "minimum_cover", "column": "cover",
        "keys": {"class": property("Class")}});
    let requirement = json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": {"kind": "add", "left": property("Cover"), "right": {"kind": "parameter", "name": "tolerance"}},
        "right": required});
    let parameters = json!({
        "minimum_cover": {"type": "table", "value": [row("XC4", 40.0), row("XC3", 35.0), row("*", 25.0)]},
        "tolerance": {"type": "quantity", "value": 0.0, "unit": "mm"},
    });
    let report = check(
        &package,
        vec![expression_rule(&requirement, parameters.clone())],
        &session(slabs()),
    );
    assert_eq!(subjects(&report, "r"), ["s2", "s3", "s5"]);
    // A tolerance stated as text is refused by its declared kind.
    let mut wrong = parameters.clone();
    wrong["tolerance"] = json!({"type": "string", "value": "5 mm"});
    assert!(matches!(
        compiled(&package, expression_rule(&requirement, wrong)),
        Err(EngineError::InvalidParameterType { .. })
    ));
    // A parameter no definition declares stays unknown.
    let mut extra = parameters;
    extra["undeclared"] = json!({"type": "integer", "value": 1});
    assert!(matches!(
        compiled(&package, expression_rule(&requirement, extra)),
        Err(EngineError::UnknownParameter { .. })
    ));
}

/// The cover beyond 30 mm, derived once.
fn margin_values() -> BTreeMap<String, axioval_ir::contract::ValueDefinition> {
    serde_json::from_value(json!({
        "margin": {"name": text("margin"), "expression": {"kind": "subtract",
            "left": property("Cover"), "right": mm(30.0)}},
        "short": {"name": text("short"), "expression": {"kind": "compare", "operator": "lessThan",
            "left": {"kind": "derived", "name": "margin"}, "right": mm(0.0)}},
    }))
    .unwrap()
}

fn value(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:value", "property": name})
}

#[test]
fn a_derived_value_reads_alike_in_selectors_requirements_predicates_and_takeoffs() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let selected = rule(
        "selected",
        EXPRESSION,
        "error",
        json!({"kind": "allOf", "operands": [entity("slab"),
            {"kind": "property", "propertySet": "axioval:value", "property": "short",
             "operator": "equals", "value": {"type": "boolean", "value": true}}]}),
        json!({"requirement": {"type": "expression", "value":
            {"kind": "literal", "value": {"type": "boolean", "value": false}}}}),
        json!({}),
    );
    let required = rule(
        "required",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value":
            {"kind": "not", "operand": {"kind": "derived", "name": "short"}}}}),
        json!({}),
    );
    let predicate = rule(
        "predicate",
        PREDICATE,
        "error",
        entity("slab"),
        json!({
            "property_set": {"type": "string", "value": "axioval:value"},
            "property": {"type": "string", "value": "margin"},
            "operator": {"type": "string", "value": "greater_or_equal"},
            "quantity": {"type": "quantity", "value": 0.0, "unit": "m"},
        }),
        json!({}),
    );
    let takeoff = rule(
        "takeoff",
        TAKEOFF,
        "info",
        entity("slab"),
        json!({
            "measure_1": {"type": "propertyReference", "propertySet": "axioval:value", "property": "margin"},
            "measure_1_name": {"type": "string", "value": "margin"},
            "measure_1_aggregates": {"type": "stringList", "value": ["sum"]},
        }),
        json!({}),
    );
    let mut ruleset = common::runtime::ruleset(vec![selected, required, predicate, takeoff]);
    ruleset.values = margin_values();
    let registry = self::registry();
    let plan =
        axioval_engine::compile(&registry, std::slice::from_ref(&package), &ruleset).unwrap();
    let report = run(registry, plan, &session(slabs()), |runtime| runtime).unwrap();
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    // s2 (35 mm) and s1 (45 mm) are above 30 mm; s3, s4 (30 mm) at it.
    for rule in ["selected", "required", "predicate"] {
        assert_eq!(subjects(&report, rule), ["s5"], "{rule}");
    }
    let table = report
        .table(&axioval_ir::RuleId::new("takeoff").unwrap(), "takeoff")
        .unwrap();
    let total = &table.rows()[0].values()[1];
    // 15 + 5 + 0 + 0 - 10 mm: the sum holds 10 mm.
    let (lower, upper) = match total {
        axioval_ir::ReportValue::Exact { value } => (*value, *value),
        axioval_ir::ReportValue::Interval { lower, upper } => (*lower, *upper),
        other => panic!("{other:?}"),
    };
    assert!(
        lower - 1e-12 <= 0.010 && 0.010 <= upper + 1e-12,
        "{lower}..{upper}"
    );
}

#[test]
fn values_reading_one_another_in_a_cycle_fail_compilation_naming_it() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let mut ruleset = common::runtime::ruleset(vec![expression_rule(
        &json!({"kind": "literal", "value": {"type": "boolean", "value": true}}),
        json!({}),
    )]);
    ruleset.values = serde_json::from_value(json!({
        "a": {"name": text("a"), "expression": {"kind": "add", "left": value("b"), "right": mm(1.0)}},
        "b": {"name": text("b"), "expression": {"kind": "derived", "name": "c"}},
        "c": {"name": text("c"), "expression": {"kind": "derived", "name": "a"}},
    }))
    .unwrap();
    let error = axioval_engine::compile(&registry, std::slice::from_ref(&package), &ruleset)
        .map(drop)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "value `a`: the values read one another: a → b → c → a"
    );
    ruleset.values = serde_json::from_value(json!({
        "a": {"name": text("a"), "expression": {"kind": "derived", "name": "missing"}},
    }))
    .unwrap();
    let error = axioval_engine::compile(&registry, std::slice::from_ref(&package), &ruleset)
        .map(drop)
        .unwrap_err();
    assert!(error.to_string().contains("`missing`"), "{error}");
}

fn thick(minimum: f64) -> Value {
    json!({"kind": "expression", "expression": {"kind": "compare",
        "operator": "greaterThan", "left": property("Cover"), "right": mm(minimum)}})
}

/// A rule flagging every object its applicability selects.
fn flag(id: &str, applicability: Value) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        applicability,
        json!({"requirement": {"type": "expression", "value":
            {"kind": "literal", "value": {"type": "boolean", "value": false}}}}),
        json!({}),
    )
}

#[test]
fn an_expression_selector_composes_with_every_selector_combinator() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    // s1 45 mm, s2 35 mm, s3 and s4 30 mm, s5 20 mm; s1 hosts s5.
    let model = slabs().edge("Hosts", "s1", "s5");
    let slab = entity("slab");
    let rules = vec![
        flag(
            "all",
            json!({"kind": "allOf", "operands": [slab, thick(30.0)]}),
        ),
        flag(
            "any",
            json!({"kind": "anyOf", "operands": [thick(40.0), class_is_selector("XC1")]}),
        ),
        flag(
            "not",
            json!({"kind": "allOf", "operands": [slab, {"kind": "not", "operand": thick(25.0)}]}),
        ),
        flag(
            "related",
            json!({"kind": "related", "path": ["Hosts"], "selector": {"kind": "not", "operand": thick(25.0)}}),
        ),
    ];
    let report = check(&package, rules, &session(model));
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    assert_eq!(subjects(&report, "all"), ["s1", "s2"]);
    assert_eq!(subjects(&report, "any"), ["s1", "s4", "s5"]);
    assert_eq!(subjects(&report, "not"), ["s5"]);
    assert_eq!(subjects(&report, "related"), ["s1"]);
}

fn class_is_selector(class: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": "t.Class",
        "operator": "equals", "value": {"type": "string", "value": class}})
}

#[test]
fn a_straddling_measured_value_leaves_the_object_open_in_every_rule_selecting_by_it() {
    let model = Model::default()
        .object("p40", "pipe")
        .object("p100", "pipe")
        .object("p50", "pipe");
    let extents = Extents(
        [
            ("p40", [(1.0, 1.0), (1.04, 1.04)]),
            ("p100", [(1.0, 1.0), (1.1, 1.1)]),
            ("p50", [(0.9975, 1.0025), (1.0475, 1.0525)]),
        ]
        .into_iter()
        .map(|(local, extent)| (id(local), extent))
        .collect(),
    );
    let session = session(model)
        .with_host_service(
            VerticalExtentServiceHandle::new(Arc::new(extents)),
            &[snapshot()],
        )
        .unwrap();
    let small = json!({"kind": "allOf", "operands": [entity("pipe"), {"kind": "expression",
        "expression": {"kind": "compare", "operator": "lessThan",
            "left": {"kind": "property", "propertySet": "axioval:measured", "property": "extent_z"},
            "right": mm(50.0)}}]});
    let predicate = rule(
        "labelled",
        PREDICATE,
        "error",
        small.clone(),
        json!({
            "property_set": {"type": "string", "value": "t.Pset"},
            "property": {"type": "string", "value": "t.Class"},
            "operator": {"type": "string", "value": "is_defined"},
        }),
        json!({}),
    );
    let registry = registry();
    let report = check(
        &vocabulary(&registry, &[]),
        vec![flag("small", small), predicate],
        &session,
    );
    assert_eq!(subjects(&report, "small"), ["p40"]);
    assert_eq!(subjects(&report, "labelled"), ["p40"]);
    for rule in ["small", "labelled"] {
        let open: Vec<_> = report
            .not_evaluated
            .iter()
            .filter(|outcome| outcome.rule_id.to_string() == rule)
            .map(|outcome| (outcome.object_id().cloned(), outcome.reason.clone()))
            .collect();
        assert_eq!(
            open,
            [(Some(id("p50")), NotEvaluatedReason::IncompleteEvidence)],
            "{rule}"
        );
    }
}

#[test]
fn an_ill_typed_selector_expression_fails_compilation() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let selector = json!({"kind": "allOf", "operands": [entity("slab"),
        {"kind": "expression", "expression": property("Cover")}]});
    match compiled(&package, flag("r", selector)) {
        Err(EngineError::InvalidExpression {
            rule,
            parameter,
            path,
            ..
        }) => {
            assert_eq!((rule.as_str(), parameter.as_str()), ("r", "applicability"));
            assert_eq!(path, "selector.expression");
        }
        other => panic!("{other:?}"),
    }
    let reads_parameter = json!({"kind": "expression", "expression": {"kind": "compare",
        "operator": "lessThan", "left": property("Cover"), "right": {"kind": "parameter", "name": "minimum"}}});
    assert!(matches!(
        compiled(&package, flag("r", reads_parameter)),
        Err(EngineError::InvalidExpression { .. })
    ));
}
