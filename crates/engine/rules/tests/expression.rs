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

#[test]
fn a_rule_constant_parameter_refuses_an_expression_and_a_computed_one_its_type() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let predicate = |parameters: Value| {
        rule(
            "p",
            PREDICATE,
            "error",
            entity("slab"),
            parameters,
            json!({}),
        )
    };
    let mut parameters = json!({
        "property_set": {"type": "string", "value": "t.Pset"},
        "property": {"type": "string", "value": "t.Cover"},
        "operator": {"type": "string", "value": "greater_or_equal"},
        "quantity": {"type": "expression", "value": mm(30.0)},
    });
    assert!(compiled(&package, predicate(parameters.clone())).is_ok());
    // The property is constant for the rule.
    let mut constant = parameters.clone();
    constant["property"] = json!({"type": "expression", "value":
        {"kind": "literal", "value": {"type": "string", "value": "t.Cover"}}});
    match compiled(&package, predicate(constant)) {
        Err(EngineError::InvalidExpression {
            parameter, detail, ..
        }) => {
            assert_eq!(parameter, "property");
            assert!(detail.contains("constant for the rule"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    // A quantity bound computed as text is ill-typed.
    parameters["quantity"] = json!({"type": "expression", "value": property("Class")});
    match compiled(&package, predicate(parameters)) {
        Err(EngineError::InvalidExpression {
            parameter, path, ..
        }) => {
            assert_eq!(
                (parameter.as_str(), path.as_str()),
                ("quantity", "quantity")
            );
        }
        other => panic!("{other:?}"),
    }
}

/// The required parameters of each capability that takes a parameter per
/// object, besides that parameter: enough to compile a rule.
fn required_besides(capability: &str) -> Value {
    match capability {
        PREDICATE => json!({
            "property_set": {"type": "string", "value": "t.Pset"},
            "property": {"type": "string", "value": "t.Cover"},
            "operator": {"type": "string", "value": "greater_or_equal"},
        }),
        "axioval:capability.distance" => json!({
            "counterparts": {"type": "selector", "value": {"kind": "all"}},
        }),
        "axioval:capability.keyed-limit" => json!({
            "limits": {"type": "table", "value": []},
            "quantity": {"type": "string", "value": "plan-area"},
            "key_1": {"type": "propertyReference", "property": "t.Class"},
        }),
        "axioval:capability.stair-geometry" | "axioval:capability.ramp-geometry" => json!({}),
        other => panic!("give `{other}` its required parameters here"),
    }
}

#[test]
fn every_per_object_numeric_parameter_takes_a_well_typed_expression_and_refuses_another() {
    use axioval_engine::{ColumnKind, ParameterType};
    let registry = registry();
    let literal = |kind: &str, value: Value| {
        let mut literal = json!({"kind": "literal", "value": {"type": kind, "value": value}});
        if kind == "quantity" {
            literal["value"]["unit"] = json!("m");
        }
        json!({"type": "expression", "value": literal})
    };
    let text = literal("string", json!("wide"));
    let mut checked = 0;
    for capability in registry.ids().map(str::to_owned).collect::<Vec<_>>() {
        let descriptors = registry.get(&capability).unwrap().parameters();
        let computed: Vec<_> = descriptors
            .iter()
            .filter(|descriptor| descriptor.per_object)
            .collect();
        if computed.is_empty() {
            continue;
        }
        let package = definitions(
            &registry,
            &[&capability],
            &["slab"],
            &["Cover", "Class"],
            &["Pset"],
        );
        for descriptor in computed {
            // A well-typed value and an ill-typed one, as the parameter or
            // as a cell of each numeric column.
            let cases: Vec<(Value, Value)> = match descriptor.parameter_type {
                ParameterType::Integer => vec![(literal("integer", json!(2)), text.clone())],
                ParameterType::Number => vec![(literal("number", json!(0.5)), text.clone())],
                ParameterType::Quantity => vec![(literal("quantity", json!(0.5)), text.clone())],
                ParameterType::Table(columns) => columns
                    .iter()
                    .filter_map(|column| {
                        let good = match column.kind {
                            ColumnKind::Integer => literal("integer", json!(2)),
                            ColumnKind::Number => literal("number", json!(0.5)),
                            ColumnKind::Quantity => literal("quantity", json!(0.5)),
                            _ => return None,
                        };
                        let row =
                            |cell: Value| json!({"type": "table", "value": [{column.id: cell}]});
                        Some((row(good), row(text.clone())))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for (good, bad) in cases {
                let parameters = |value: Value| {
                    let mut parameters = required_besides(&capability);
                    parameters[descriptor.name.as_str()] = value;
                    rule(
                        "r",
                        &capability,
                        "error",
                        json!({"kind": "all"}),
                        parameters,
                        json!({}),
                    )
                };
                let label = format!("{capability} {}", descriptor.name);
                if let Err(error) = compiled(&package, parameters(good)) {
                    panic!("{label}: a well-typed expression is refused: {error}");
                }
                assert!(
                    matches!(
                        compiled(&package, parameters(bad)),
                        Err(EngineError::InvalidExpression { .. })
                    ),
                    "{label}: an ill-typed expression is accepted"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 10,
        "only {checked} parameters are computed per object"
    );
}

/// Walls `w1` (10 m² side) hosting openings of 1.5 and 2.5 m², and `w2`
/// (10 m²) hosting 3 and 2 m²; `o5`, also on `w2`, cannot be read.
fn walls_with_openings() -> Model {
    let area = |value: f64| PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Area,
    };
    let mut model = Model::default();
    for (wall, side) in [("w1", 10.0), ("w2", 10.0)] {
        model = model
            .object(wall, "slab")
            .value(wall, "Pset", "Cover", area(side));
    }
    for (opening, wall, size) in [
        ("o1", "w1", 1.5),
        ("o2", "w1", 2.5),
        ("o3", "w2", 3.0),
        ("o4", "w2", 2.0),
    ] {
        model = model
            .object(opening, "pipe")
            .value(opening, "Pset", "Cover", area(size))
            .text(opening, "Pset", "Class", "opening")
            .edge("Hosts", wall, opening);
    }
    model
}

fn openings(function: &str, value: Option<Value>) -> Value {
    let mut aggregate = json!({"kind": "aggregate", "function": function,
        "over": {"kind": "path", "path": ["Hosts"]},
        "where": class_is_selector("opening")});
    if let Some(value) = value {
        aggregate["value"] = value;
    }
    aggregate
}

#[test]
fn an_aggregate_sums_related_objects_and_compares_with_the_subject() {
    let registry = registry();
    let mut package = vocabulary(&registry, &[]);
    package
        .properties
        .get_mut("t.Cover")
        .unwrap()
        .unit_dimension = Some("area".into());
    // The openings take at most 40 % of the wall's side.
    let share = rule(
        "share",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": {"kind": "compare",
            "operator": "lessThanOrEquals",
            "left": openings("sum", Some(property("Cover"))),
            "right": {"kind": "multiply", "left": property("Cover"),
                      "right": {"kind": "literal", "value": {"type": "number", "value": 0.4}}}}}}),
        json!({}),
    );
    // Every opening is smaller than its wall, read through `of: subject`.
    let smaller = rule(
        "smaller",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": openings("all", Some(json!(
            {"kind": "compare", "operator": "lessThan", "left": property("Cover"),
             "right": {"kind": "multiply", "left": {"kind": "property", "propertySet": "t.Pset",
                       "property": "t.Cover", "of": "subject"},
                       "right": {"kind": "literal", "value": {"type": "number", "value": 0.26}}}})))}}),
        json!({}),
    );
    let report = check(
        &package,
        vec![share, smaller],
        &session(walls_with_openings()),
    );
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
    // w1: 4 m² of 4 m² allowed; w2: 5 m².
    assert_eq!(
        subjects(&report, "share"),
        ["w2"],
        "{:?}",
        report
            .findings()
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    // w2's 3 m² opening is above 26 % of its 10 m².
    assert_eq!(subjects(&report, "smaller"), ["w2"]);
}

#[test]
fn an_undecided_member_widens_a_count_and_leaves_a_straddling_comparison_open() {
    let registry = registry();
    let mut package = vocabulary(&registry, &[]);
    package
        .properties
        .get_mut("t.Cover")
        .unwrap()
        .unit_dimension = Some("area".into());
    let model = walls_with_openings()
        .object("o5", "pipe")
        .edge("Hosts", "w2", "o5")
        .unreadable("o5");
    let at_most_two = |limit: i64| {
        rule(
            &format!("count{limit}"),
            EXPRESSION,
            "error",
            entity("slab"),
            json!({"requirement": {"type": "expression", "value": {"kind": "compare",
                "operator": "lessThanOrEquals", "left": openings("count", None),
                "right": {"kind": "literal", "value": {"type": "integer", "value": limit}}}}}),
            json!({}),
        )
    };
    let report = check(
        &package,
        vec![at_most_two(2), at_most_two(3)],
        &session(model),
    );
    // w2 holds 2 or 3 openings: at most 2 straddles, at most 3 holds.
    assert!(subjects(&report, "count2").is_empty());
    let open: Vec<_> = report
        .not_evaluated
        .iter()
        .map(|outcome| (outcome.rule_id.to_string(), outcome.object_id().cloned()))
        .collect();
    assert_eq!(open, [("count2".to_owned(), Some(id("w2")))]);
    assert!(
        report.not_evaluated[0].message.contains("2..3"),
        "{}",
        report.not_evaluated[0].message
    );
}

/// A rule passing the objects whose `t.<property>` is true, failing those
/// where it is false, and leaving open those where it cannot be read.
fn judged_by(id: &str, property: &str) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value":
            {"kind": "property", "propertySet": "t.Pset", "property": format!("t.{property}")}}}),
        json!({}),
    )
}

fn outcome(rule: &str) -> Value {
    json!({"kind": "ruleOutcome", "rule": rule})
}

#[test]
fn a_composite_rule_reads_other_rules_outcomes_with_kleene_logic() {
    let registry = registry();
    let mut package = definitions(
        &registry,
        &[EXPRESSION],
        &["slab"],
        &["Fire", "Escape", "Temporary"],
        &["Pset"],
    );
    for name in ["t.Fire", "t.Escape", "t.Temporary"] {
        package.properties.get_mut(name).unwrap().value_kind = PropertyValueKind::Boolean;
    }
    let states = [("T", Some(true)), ("F", Some(false)), ("U", None)];
    let mut model = Model::default();
    let mut expected = BTreeMap::new();
    for (fire, fire_value) in states {
        for (escape, escape_value) in states {
            for temporary in [true, false] {
                let local = format!("{fire}{escape}{}", if temporary { "t" } else { "p" });
                model = model.object(&local, "slab").value(
                    &local,
                    "Pset",
                    "Temporary",
                    PropertyValue::Boolean(temporary),
                );
                for (name, value) in [("Fire", fire_value), ("Escape", escape_value)] {
                    model = match value {
                        Some(value) => {
                            model.value(&local, "Pset", name, PropertyValue::Boolean(value))
                        }
                        None => model.unreadable_value(&local, "Pset", name, "IFCBOOLEAN"),
                    };
                }
                // Fails if either fails, unless the object is temporary.
                let both = match (fire, escape) {
                    ("F", _) | (_, "F") => "F",
                    ("T", "T") => "T",
                    _ => "U",
                };
                expected.insert(local, if temporary { "T" } else { both });
            }
        }
    }
    let composite = rule(
        "composite",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": {"kind": "or", "operands": [
            {"kind": "property", "propertySet": "t.Pset", "property": "t.Temporary"},
            {"kind": "and", "operands": [outcome("fire"), outcome("escape")]}]}}}),
        json!({}),
    );
    let report = check(
        &package,
        vec![
            judged_by("fire", "Fire"),
            judged_by("escape", "Escape"),
            composite,
        ],
        &session(model),
    );
    let failed = subjects(&report, "composite");
    let open: Vec<String> = report
        .not_evaluated
        .iter()
        .filter(|outcome| outcome.rule_id.to_string() == "composite")
        .map(|outcome| outcome.object_id().unwrap().local_id.clone())
        .collect();
    for (local, verdict) in expected {
        let found = if failed.contains(&local) {
            "F"
        } else if open.contains(&local) {
            "U"
        } else {
            "T"
        };
        assert_eq!(found, verdict, "{local}");
    }
}

#[test]
fn rules_reading_each_others_outcomes_through_expressions_fail_compilation() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let reads = |id: &str, other: &str| {
        rule(
            id,
            EXPRESSION,
            "error",
            entity("slab"),
            json!({"requirement": {"type": "expression", "value": outcome(other)}}),
            json!({}),
        )
    };
    let error = plan(&registry, &package, vec![reads("a", "b"), reads("b", "a")])
        .map(drop)
        .unwrap_err();
    assert!(error.to_string().contains("cycle"), "{error}");
    let undefined = plan(&registry, &package, vec![reads("a", "missing")])
        .map(drop)
        .unwrap_err();
    assert!(undefined.to_string().contains("`missing`"), "{undefined}");
}

#[test]
fn a_finding_count_reads_how_many_findings_another_rule_reported() {
    let registry = registry();
    let package = vocabulary(&registry, &[]);
    let count = rule(
        "count",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": {"kind": "compare",
            "operator": "equals", "left": {"kind": "findingCount", "rule": "cover"},
            "right": {"kind": "literal", "value": {"type": "integer", "value": 0}}}}}),
        json!({}),
    );
    let report = check(&package, vec![cover_rule(), count], &session(slabs()));
    assert_eq!(subjects(&report, "count"), subjects(&report, "cover"));
}

#[test]
fn a_slope_rule_grades_its_findings_by_how_far_the_slope_exceeds_the_limit() {
    let registry = registry();
    let mut package = definitions(&registry, &[EXPRESSION], &["ramp"], &["Slope"], &["Pset"]);
    package.properties.get_mut("t.Slope").unwrap().value_kind = PropertyValueKind::Number;
    let mut model = Model::default();
    for (local, slope) in [("gentle", 5.0), ("slight", 6.5), ("steep", 8.0)] {
        model = model.object(local, "ramp").value(
            local,
            "Pset",
            "Slope",
            PropertyValue::Decimal(slope),
        );
    }
    let slope = json!({"kind": "property", "propertySet": "t.Pset", "property": "t.Slope", "label": "slope"});
    let six = json!({"kind": "literal", "value": {"type": "number", "value": 6.0}});
    let ramp = rule(
        "ramp",
        EXPRESSION,
        "error",
        entity("ramp"),
        json!({
            "requirement": {"type": "expression", "value":
                {"kind": "compare", "operator": "lessThanOrEquals", "left": slope, "right": six}},
            "deviation": {"type": "expression", "value":
                {"kind": "subtract", "left": slope, "right": six, "label": "excess"}},
            "message": {"type": "string", "value": "the ramp slopes {slope} %, {excess} % over 6 %"},
        }),
        // Up to 1 % over is minor; beyond, the rule's own (major) severity.
        json!({"severityBands": [{"below": 1.0, "severity": "warning"}]}),
    );
    let report = check(&package, vec![ramp], &session(model));
    let graded: BTreeMap<String, (axioval_ir::Severity, String)> = report
        .findings()
        .iter()
        .map(|finding| {
            (
                common::subject(finding),
                (finding.severity.clone(), finding.message.clone()),
            )
        })
        .collect();
    assert_eq!(graded.len(), 2, "{graded:?}");
    assert_eq!(graded["slight"].0, axioval_ir::Severity::Warning);
    assert_eq!(graded["steep"].0, axioval_ir::Severity::Error);
    assert_eq!(graded["slight"].1, "the ramp slopes 6.5 %, 0.5 % over 6 %");
    assert_eq!(graded["steep"].1, "the ramp slopes 8 %, 2 % over 6 %");
}
