//! Hierarchical classifications: a ruleset's declared class tree, read by
//! the class a row assigns, by a level of the tree, and by a `derivedClass`
//! selector that takes a class together with its descendants.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use axioval_engine::{CapabilityRegistry, EngineError, compile};
use axioval_ir::{
    NotEvaluatedReason, PropertyValue, QuantityDimension, Report, ReportValue, RuleId,
    RuleSetPackage, Scope,
};
use axioval_rules::register_builtins;
use common::runtime::{definitions, rule, ruleset, run, session};
use common::{Model, id};
use serde_json::{Value, json};

const TAKEOFF: &str = "axioval:capability.quantity-takeoff";
const EXISTS: &str = "axioval:capability.property-exists";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn class(id: &str, parent: Option<&str>) -> Value {
    let mut class = json!({ "id": format!("kg-{id}"), "code": id,
                            "name": { "default": format!("Cost group {id}"), "translations": {} } });
    if let Some(parent) = parent {
        class["parent"] = json!(format!("kg-{parent}"));
    }
    class
}

fn named(type_name: &str) -> Value {
    json!({ "kind": "property", "property": "t.TypeName", "operator": "like",
            "value": { "type": "string", "value": type_name } })
}

/// A three-level cost-group tree. Rows assign walls their leaf classes by
/// type name; a wall of an unknown internal type gets the inner class
/// `kg-340`, and a pipe the leaf `kg-411` of another root.
fn cost_groups() -> Value {
    json!({
        "id": "cost-group",
        "name": { "default": "Cost group", "translations": {} },
        "classes": [
            class("300", None), class("330", Some("300")), class("331", Some("330")),
            class("332", Some("330")), class("340", Some("300")), class("341", Some("340")),
            class("342", Some("340")), class("400", None), class("410", Some("400")),
            class("411", Some("410")),
        ],
        "rows": [
            { "selector": named("EXT-LB"), "class": "kg-331" },
            { "selector": named("EXT-NLB"), "class": "kg-332" },
            { "selector": named("INT-LB"), "class": "kg-341" },
            { "selector": named("INT-NLB"), "class": "kg-342" },
            { "selector": named("PIPE*"), "class": "kg-411" },
            { "selector": named("INT*"), "class": "kg-340" },
        ],
    })
}

fn area(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Area,
    }
}

/// Walls of every leaf, a wall only known to be internal (`w5`) and a
/// pipe, each with a net side area.
fn building() -> Model {
    [
        ("w1", "wall", "EXT-LB", 10.0),
        ("w2", "wall", "EXT-NLB", 5.0),
        ("w3", "wall", "INT-LB", 4.0),
        ("w4", "wall", "INT-NLB", 3.0),
        ("w5", "wall", "INT-X", 2.0),
        ("p1", "pipe", "PIPE-100", 1.0),
    ]
    .into_iter()
    .fold(Model::default(), |model, (local, kind, type_name, net)| {
        model
            .object(local, kind)
            .text(local, "Pset", "TypeName", type_name)
            .value(local, "Qto", "NetSideArea", area(net))
    })
}

fn classified(rules: Vec<Value>, classification: Value) -> RuleSetPackage {
    let mut set = ruleset(rules);
    set.classifications = serde_json::from_value(json!({ "cost-group": classification })).unwrap();
    set
}

fn check(set: &RuleSetPackage, model: Model) -> Result<Report, EngineError> {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[TAKEOFF, EXISTS],
        &["wall", "pipe"],
        &["TypeName", "NetSideArea", "Missing"],
        &[],
    );
    let plan = compile(&registry, &[definitions], set)?;
    run(registry, plan, &session(model), |runtime| runtime)
}

fn classification(property: &str) -> Value {
    json!({ "type": "propertyReference", "propertySet": "axioval:classification",
            "property": property })
}

/// Every object `selector` selects, as the subjects of a rule that fails
/// every one of them.
fn selecting(selector: Value) -> Value {
    rule(
        "selected",
        EXISTS,
        "error",
        selector,
        json!({ "property": { "type": "propertyReference", "property": "t.Missing" } }),
        json!({}),
    )
}

fn selected(report: &Report) -> Vec<String> {
    let mut subjects: Vec<String> = report.findings().iter().map(common::subject).collect();
    subjects.sort();
    subjects
}

fn derived_class(class: &str, include_descendants: bool) -> Value {
    json!({ "kind": "derivedClass", "classification": "cost-group", "class": class,
            "includeDescendants": include_descendants })
}

type Row = (Vec<String>, Vec<ReportValue>);

fn takeoff(groups: &[&str]) -> Value {
    let mut parameters = json!({ "measure_1": { "type": "propertyReference",
                                                "property": "t.NetSideArea" } });
    for (n, group) in groups.iter().enumerate() {
        parameters[format!("group_{}", n + 1)] = classification(group);
    }
    rule(
        "takeoff",
        TAKEOFF,
        "info",
        json!({ "kind": "all" }),
        parameters,
        json!({}),
    )
}

fn rows(report: &Report) -> Vec<Row> {
    report
        .table(&RuleId::new("takeoff").unwrap(), "takeoff")
        .unwrap()
        .rows()
        .iter()
        .map(|row| (row.group().to_vec(), row.values().to_vec()))
        .collect()
}

fn row(group: &[&str], count: f64, sum: f64) -> Row {
    (
        group.iter().map(|&value| value.to_owned()).collect(),
        vec![ReportValue::exact(count), ReportValue::exact(sum)],
    )
}

#[test]
fn walls_are_classified_by_their_leaf_classes() {
    let report = check(
        &classified(vec![takeoff(&["cost-group"])], cost_groups()),
        building(),
    )
    .unwrap();
    assert_eq!(
        rows(&report),
        [
            row(&["kg-331"], 1.0, 10.0),
            row(&["kg-332"], 1.0, 5.0),
            row(&["kg-340"], 1.0, 2.0),
            row(&["kg-341"], 1.0, 4.0),
            row(&["kg-342"], 1.0, 3.0),
            row(&["kg-411"], 1.0, 1.0),
        ]
    );
    assert!(report.not_evaluated.is_empty());
}

#[test]
fn a_takeoff_groups_by_the_first_and_second_level() {
    let report = check(
        &classified(
            vec![takeoff(&["cost-group;level=1", "cost-group;level=2"])],
            cost_groups(),
        ),
        building(),
    )
    .unwrap();
    assert_eq!(
        rows(&report),
        [
            row(&["kg-300", "kg-330"], 2.0, 15.0),
            row(&["kg-300", "kg-340"], 3.0, 9.0),
            row(&["kg-400", "kg-410"], 1.0, 1.0),
        ]
    );
    assert!(report.not_evaluated.is_empty());
}

#[test]
fn a_class_above_the_level_read_has_no_class_there() {
    // `w5` is only known to be an internal wall: it has no level-3 class.
    let report = check(
        &classified(vec![takeoff(&["cost-group;level=3"])], cost_groups()),
        building(),
    )
    .unwrap();
    assert_eq!(
        rows(&report),
        [
            row(&["-"], 1.0, 2.0),
            row(&["kg-331"], 1.0, 10.0),
            row(&["kg-332"], 1.0, 5.0),
            row(&["kg-341"], 1.0, 4.0),
            row(&["kg-342"], 1.0, 3.0),
            row(&["kg-411"], 1.0, 1.0),
        ]
    );
}

#[test]
fn an_inner_class_selects_its_descendants() {
    let external = check(
        &classified(
            vec![selecting(derived_class("kg-330", true))],
            cost_groups(),
        ),
        building(),
    )
    .unwrap();
    assert_eq!(selected(&external), ["w1", "w2"]);
    let building_construction = check(
        &classified(
            vec![selecting(derived_class("kg-300", true))],
            cost_groups(),
        ),
        building(),
    )
    .unwrap();
    assert_eq!(
        selected(&building_construction),
        ["w1", "w2", "w3", "w4", "w5"]
    );
    // Without descendants a class selects only the objects assigned it.
    let internal = check(
        &classified(
            vec![selecting(derived_class("kg-340", false))],
            cost_groups(),
        ),
        building(),
    )
    .unwrap();
    assert_eq!(selected(&internal), ["w5"]);
    let leaf = check(
        &classified(
            vec![selecting(derived_class("kg-331", true))],
            cost_groups(),
        ),
        building(),
    )
    .unwrap();
    assert_eq!(selected(&leaf), ["w1"]);
}

#[test]
fn a_level_reads_as_a_property_in_every_selector() {
    let report = check(
        &classified(
            vec![selecting(json!({ "kind": "property",
                "propertySet": "axioval:classification", "property": "cost-group;level=2",
                "operator": "equals", "value": { "type": "string", "value": "kg-340" } }))],
            cost_groups(),
        ),
        building(),
    )
    .unwrap();
    assert_eq!(selected(&report), ["w3", "w4", "w5"]);
}

#[test]
fn an_undecided_class_is_neither_selected_nor_dropped() {
    // `w6` states its type name as a list the first row cannot compare.
    let model = || {
        building()
            .object("w6", "wall")
            .value(
                "w6",
                "Pset",
                "TypeName",
                PropertyValue::List(vec![PropertyValue::String("EXT-LB".into())]),
            )
            .value("w6", "Qto", "NetSideArea", area(7.0))
    };
    let report = check(
        &classified(
            vec![selecting(derived_class("kg-330", true))],
            cost_groups(),
        ),
        model(),
    )
    .unwrap();
    assert_eq!(selected(&report), ["w1", "w2"]);
    assert!(
        report
            .not_evaluated
            .iter()
            .any(|outcome| outcome.scope == Scope::Object(id("w6"))
                && outcome.reason == NotEvaluatedReason::InvalidEvidence)
    );
    // The takeoff widens every level-1 group `w6` may belong to.
    let report = check(
        &classified(vec![takeoff(&["cost-group;level=1"])], cost_groups()),
        model(),
    )
    .unwrap();
    assert_eq!(
        rows(&report),
        [
            (
                vec!["kg-300".to_owned()],
                vec![
                    ReportValue::measured(5.0, 6.0),
                    ReportValue::measured(24.0, 31.0)
                ]
            ),
            (
                vec!["kg-400".to_owned()],
                vec![
                    ReportValue::measured(1.0, 2.0),
                    ReportValue::measured(1.0, 8.0)
                ]
            ),
        ]
    );
}

mod compilation {
    use super::*;

    fn refused(classification: Value, rules: Vec<Value>) -> String {
        match check(&classified(rules, classification), building()) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("compiled"),
        }
    }

    fn invalid(classification: Value) -> String {
        refused(classification, vec![takeoff(&["cost-group"])])
    }

    #[test]
    fn malformed_trees_are_refused() {
        let mut cycle = cost_groups();
        cycle["classes"][0]["parent"] = json!("kg-331");
        assert!(invalid(cycle).contains("cycle"));
        let mut orphan = cost_groups();
        orphan["classes"][1]["parent"] = json!("kg-999");
        assert!(invalid(orphan).contains("undeclared parent `kg-999`"));
        let mut code = cost_groups();
        code["classes"][2]["code"] = json!("332");
        assert!(invalid(code).contains("code `332`"));
        let mut twice = cost_groups();
        twice["classes"][2]["id"] = json!("kg-332");
        assert!(invalid(twice).contains("declared twice"));
        let mut undeclared = cost_groups();
        undeclared["rows"][0]["class"] = json!("kg-339");
        assert!(invalid(undeclared).contains("undeclared class `kg-339`"));
    }

    #[test]
    fn a_level_outside_the_tree_is_an_unknown_concept() {
        for level in [
            "cost-group;level=4",
            "cost-group;level=0",
            "cost-group;depth=1",
        ] {
            let error = refused(cost_groups(), vec![takeoff(&[level])]);
            assert!(error.contains("unknown axioval:classification"), "{error}");
        }
        // A flat classification has no levels.
        let mut flat = cost_groups();
        flat.as_object_mut().unwrap().remove("classes");
        for row in flat["rows"].as_array_mut().unwrap() {
            row["class"] = json!("wall");
        }
        let error = refused(flat, vec![takeoff(&["cost-group;level=1"])]);
        assert!(error.contains("unknown axioval:classification"), "{error}");
    }

    #[test]
    fn a_derived_class_selector_names_a_declared_class() {
        let error = refused(
            cost_groups(),
            vec![selecting(derived_class("kg-339", true))],
        );
        assert!(error.contains("cost-group/kg-339"), "{error}");
        let mut other = derived_class("kg-330", true);
        other["classification"] = json!("space-use");
        let error = refused(cost_groups(), vec![selecting(other)]);
        assert!(error.contains("space-use"), "{error}");
    }

    #[test]
    fn a_derived_class_selector_in_its_own_classification_is_a_cycle() {
        let mut cyclic = cost_groups();
        cyclic["rows"][0]["selector"] = derived_class("kg-340", true);
        let error = invalid(cyclic);
        assert!(error.contains("cycle"), "{error}");
    }
}
