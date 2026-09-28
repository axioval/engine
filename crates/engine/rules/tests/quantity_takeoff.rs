//! Information takeoff: the selection counted and its stated or measured
//! quantities summed per group, reported as a grouped table whose values
//! are intervals widened by every undecided member.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, EvidenceSession, PlanArea, PlanAreaError, PlanAreaService,
    PlanAreaServiceHandle, compile,
};
use axioval_ir::{
    Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension, Report, ReportColumn,
    ReportValue, RuleId, RuleSetPackage, Scope,
};
use axioval_rules::register_builtins;
use common::runtime::{definitions, entity, rule, ruleset, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const TAKEOFF: &str = "axioval:capability.quantity-takeoff";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn area(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Area,
    }
}

fn reference(set: Option<&str>, property: &str) -> Value {
    match set {
        Some(set) => {
            json!({ "type": "propertyReference", "propertySet": set, "property": property })
        }
        None => json!({ "type": "propertyReference", "property": property }),
    }
}

fn text(value: &str) -> Value {
    json!({ "type": "string", "value": value })
}

/// A wall of `kind` on `storey` with a net side area, when it states one.
fn wall(model: Model, local: &str, kind: &str, storey: &str, net: Option<f64>) -> Model {
    let model = model
        .object(local, "wall")
        .text(local, "Pset", "TypeName", kind)
        .text(local, "Pset", "Status", "new")
        .edge("contains", storey, local);
    match net {
        Some(net) => model.value(local, "Qto", "NetSideArea", area(net)),
        None => model,
    }
}

/// Two storeys and walls of types `A` and `B`: three of type `A`, two of
/// them on level 1.
fn walls() -> Model {
    let model = Model::default()
        .object("l1", "storey")
        .object("l2", "storey")
        .text("l1", "Pset", "Name", "Level 1")
        .text("l2", "Pset", "Name", "Level 2");
    let model = wall(model, "w1", "A", "l1", Some(10.0));
    let model = wall(model, "w2", "A", "l1", Some(12.5));
    let model = wall(model, "w3", "A", "l2", Some(8.0));
    wall(model, "w4", "B", "l1", Some(20.0))
}

/// Walls selected by `selector`, grouped by type name and storey, counted
/// and their net side areas summed.
fn wall_takeoff(selector: Value) -> Value {
    rule(
        "wall-takeoff",
        TAKEOFF,
        "info",
        selector,
        json!({
            "group_1": reference(None, "t.TypeName"),
            "group_1_name": text("type"),
            "group_2": reference(None, "t.Name"),
            "group_2_path": { "type": "stringList", "value": ["contains:backward"] },
            "group_2_name": text("storey"),
            "measure_1": reference(None, "t.NetSideArea"),
        }),
        json!({}),
    )
}

fn check(set: &RuleSetPackage, session: &EvidenceSession) -> Report {
    let registry = registry();
    let definitions = definitions(
        &registry,
        &[TAKEOFF],
        &["wall", "storey", "space"],
        &["TypeName", "Name", "NetSideArea", "Status"],
        &[],
    );
    let plan = compile(&registry, &[definitions], set).unwrap();
    run(registry, plan, session, |runtime| runtime).unwrap()
}

type Row = (Vec<String>, Vec<ReportValue>);

/// The takeoff rows of `rule`, in table order.
fn rows(report: &Report, rule: &str) -> Vec<Row> {
    report
        .table(&RuleId::new(rule).unwrap(), "takeoff")
        .unwrap()
        .rows()
        .iter()
        .map(|row| {
            assert_eq!(row.scope(), &Scope::Source(source()));
            (row.group().to_vec(), row.values().to_vec())
        })
        .collect()
}

fn row(group: &[&str], values: &[ReportValue]) -> Row {
    (
        group.iter().map(|&value| value.to_owned()).collect(),
        values.to_vec(),
    )
}

fn exact(value: f64) -> ReportValue {
    ReportValue::exact(value)
}

fn between(lower: f64, upper: f64) -> ReportValue {
    ReportValue::measured(lower, upper)
}

fn open(report: &Report) -> Vec<(Scope, NotEvaluatedReason)> {
    report
        .not_evaluated
        .iter()
        .map(|outcome| (outcome.scope.clone(), outcome.reason.clone()))
        .collect()
}

#[test]
fn walls_grouped_by_type_and_storey_report_count_and_summed_net_side_area() {
    let report = check(
        &ruleset(vec![wall_takeoff(entity("wall"))]),
        &session(walls()),
    );
    let table = report
        .table(&RuleId::new("wall-takeoff").unwrap(), "takeoff")
        .unwrap();
    assert_eq!(table.group_by(), ["type", "storey"]);
    assert_eq!(
        table.columns(),
        [
            ReportColumn::number("count"),
            ReportColumn::quantity("sum_net_side_area", QuantityDimension::Area),
        ]
    );
    assert_eq!(
        rows(&report, "wall-takeoff"),
        [
            row(&["A", "Level 1"], &[exact(2.0), exact(22.5)]),
            row(&["A", "Level 2"], &[exact(1.0), exact(8.0)]),
            row(&["B", "Level 1"], &[exact(1.0), exact(20.0)]),
        ]
    );
    assert!(report.findings().is_empty());
    assert!(report.not_evaluated.is_empty());
}

#[test]
fn a_member_without_the_property_makes_its_group_sum_unknown() {
    let model = wall(walls(), "w5", "B", "l2", None);
    let report = check(
        &ruleset(vec![wall_takeoff(entity("wall"))]),
        &session(model),
    );
    assert_eq!(
        rows(&report, "wall-takeoff"),
        [
            row(&["A", "Level 1"], &[exact(2.0), exact(22.5)]),
            row(&["A", "Level 2"], &[exact(1.0), exact(8.0)]),
            row(&["B", "Level 1"], &[exact(1.0), exact(20.0)]),
            row(&["B", "Level 2"], &[exact(1.0), ReportValue::Unknown]),
        ]
    );
    assert_eq!(
        open(&report),
        [(
            Scope::Object(id("w5")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
    assert!(report.not_evaluated[0].message.contains("has no value"));
}

#[test]
fn an_undecided_selection_widens_its_group_rather_than_dropping_it() {
    // `w6` states its status as a list, which `equals` does not compare
    // without a quantifier: it may or may not be selected.
    let model = wall(walls(), "w6", "A", "l1", Some(5.0)).value(
        "w6",
        "Pset",
        "Status",
        PropertyValue::List(vec![PropertyValue::String("new".into())]),
    );
    let new_walls = json!({ "kind": "allOf", "operands": [
        entity("wall"),
        { "kind": "property", "property": "t.Status", "operator": "equals",
          "value": { "type": "string", "value": "new" } },
    ] });
    let report = check(&ruleset(vec![wall_takeoff(new_walls)]), &session(model));
    assert_eq!(
        rows(&report, "wall-takeoff")[0],
        row(&["A", "Level 1"], &[between(2.0, 3.0), between(22.5, 27.5)])
    );
    assert_eq!(open(&report).len(), 1);
    assert_eq!(open(&report)[0].0, Scope::Object(id("w6")));
    assert!(
        report.not_evaluated[0]
            .message
            .contains("may or may not count")
    );
}

#[test]
fn several_aggregates_and_a_takeoff_without_groups() {
    let takeoff = rule(
        "totals",
        TAKEOFF,
        "info",
        entity("wall"),
        json!({
            "measure_1": reference(None, "t.NetSideArea"),
            "measure_1_aggregates": { "type": "stringList", "value": ["sum", "min", "max", "mean"] },
            "measure_1_name": text("area"),
        }),
        json!({}),
    );
    let report = check(&ruleset(vec![takeoff]), &session(walls()));
    let table = report
        .table(&RuleId::new("totals").unwrap(), "takeoff")
        .unwrap();
    assert!(table.group_by().is_empty());
    let ids: Vec<&str> = table
        .columns()
        .iter()
        .map(|column| column.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["count", "sum_area", "min_area", "max_area", "mean_area"]
    );
    assert_eq!(
        rows(&report, "totals"),
        [row(
            &[],
            &[
                exact(4.0),
                exact(50.5),
                exact(8.0),
                exact(20.0),
                exact(12.625)
            ]
        )]
    );
}

#[test]
fn malformed_declarations_leave_the_rule_not_evaluated() {
    let declared = |parameters: Value| {
        let report = check(
            &ruleset(vec![rule(
                "bad",
                TAKEOFF,
                "info",
                entity("wall"),
                parameters,
                json!({}),
            )]),
            &session(walls()),
        );
        assert!(report.tables().is_empty());
        assert_eq!(
            open(&report),
            [(Scope::Project, NotEvaluatedReason::InvalidDeclaration)]
        );
        report.not_evaluated[0].message.clone()
    };
    let message = declared(json!({ "group_2": reference(None, "t.Name") }));
    assert!(message.contains("without `group_1`"), "{message}");
    let message = declared(json!({
        "measure_1": reference(None, "t.NetSideArea"),
        "measure_1_aggregates": { "type": "stringList", "value": ["median"] },
    }));
    assert!(message.contains("`median` is unsupported"), "{message}");
    let message = declared(json!({
        "measure_1": reference(None, "t.NetSideArea"),
        "measure_1_name": text("count"),
        "measure_1_aggregates": { "type": "stringList", "value": [] },
    }));
    assert!(message.contains("is empty"), "{message}");
    let message = declared(json!({
        "group_1": reference(None, "t.Name"),
        "group_1_name": text("sum_area"),
        "measure_1": reference(None, "t.NetSideArea"),
        "measure_1_name": text("area"),
    }));
    assert!(message.contains("twice"), "{message}");
}

/// Footprint areas as stated intervals.
struct Footprints(BTreeMap<ObjectId, (f64, f64)>);

impl PlanAreaService for Footprints {
    #[allow(clippy::float_cmp)]
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (lower, upper) = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("footprint:{object}"));
        evidence.exact = lower == upper;
        PlanArea::try_new(lower, upper, evidence)
    }

    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        _: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::UnknownObject(first.clone()))
    }
}

/// Offices, a lab, a store no row classifies, and `unsure`, whose name is a
/// list the first row compares without a quantifier.
fn spaces() -> Model {
    Model::default()
        .object("o1", "space")
        .object("o2", "space")
        .object("lab", "space")
        .object("store", "space")
        .object("unsure", "space")
        .text("o1", "Pset", "Name", "Office 1")
        .text("o2", "Pset", "Name", "Office 2")
        .text("lab", "Pset", "Name", "Lab 1")
        .text("store", "Pset", "Name", "Store")
        .value(
            "unsure",
            "Pset",
            "Name",
            PropertyValue::List(vec![PropertyValue::String("Office 3".into())]),
        )
}

fn footprints() -> Footprints {
    Footprints(BTreeMap::from([
        (id("o1"), (20.0, 20.0)),
        (id("o2"), (9.5, 10.5)),
        (id("lab"), (30.0, 30.5)),
        (id("store"), (5.0, 5.0)),
        (id("unsure"), (2.0, 2.0)),
    ]))
}

/// Spaces grouped by their derived use, footprints summed and the largest
/// taken.
fn space_takeoff() -> RuleSetPackage {
    let like = |pattern: &str| {
        json!({ "kind": "property", "property": "t.Name", "operator": "like",
                "value": { "type": "string", "value": pattern } })
    };
    let mut set = ruleset(vec![rule(
        "space-takeoff",
        TAKEOFF,
        "info",
        entity("space"),
        json!({
            "group_1": reference(Some("axioval:classification"), "space-use"),
            "group_1_name": text("use"),
            "measure_1": reference(Some("axioval:measured"), "area"),
            "measure_1_name": text("footprint"),
            "measure_1_aggregates": { "type": "stringList", "value": ["sum", "max"] },
        }),
        json!({}),
    )]);
    set.classifications = serde_json::from_value(json!({ "space-use": {
        "id": "space-use",
        "name": { "default": "Space use", "translations": {} },
        "rows": [
            { "selector": like("Office*"), "class": "office" },
            { "selector": like("*Lab*"), "class": "lab" },
        ],
    } }))
    .unwrap();
    set
}

#[test]
fn with_geometry_spaces_grouped_by_use_report_summed_footprint_intervals() {
    let session = session(spaces())
        .with_host_service(
            PlanAreaServiceHandle::new(Arc::new(footprints())),
            &[snapshot()],
        )
        .unwrap();
    let report = check(&space_takeoff(), &session);
    let table = report
        .table(&RuleId::new("space-takeoff").unwrap(), "takeoff")
        .unwrap();
    assert_eq!(
        table.columns()[1],
        ReportColumn::quantity("sum_footprint", QuantityDimension::Area)
    );
    // `unsure` may be in any group: every count and sum is widened by it,
    // and it is reported not evaluated rather than dropped.
    assert_eq!(
        rows(&report, "space-takeoff"),
        [
            row(&["-"], &[between(1.0, 2.0), between(5.0, 7.0), exact(5.0)]),
            row(
                &["lab"],
                &[between(1.0, 2.0), between(30.0, 32.5), between(30.0, 30.5)]
            ),
            row(
                &["office"],
                &[between(2.0, 3.0), between(29.5, 32.5), exact(20.0)]
            ),
        ]
    );
    assert_eq!(open(&report).len(), 1);
    assert_eq!(open(&report)[0].0, Scope::Object(id("unsure")));
    assert!(
        report.not_evaluated[0]
            .message
            .contains("may count in any group")
    );
}

#[test]
fn without_geometry_measured_sums_are_unknown_and_reported_once() {
    let report = check(&space_takeoff(), &session(spaces()));
    let sums: Vec<ReportValue> = rows(&report, "space-takeoff")
        .into_iter()
        .map(|(_, values)| values[1].clone())
        .collect();
    assert_eq!(sums, vec![ReportValue::Unknown; 3]);
    assert!(open(&report).contains(&(Scope::Source(source()), NotEvaluatedReason::MissingService)));
}
