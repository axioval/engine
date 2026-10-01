//! Information takeoff: the selection counted and its stated or measured
//! quantities summed per group, reported as a grouped table whose values
//! are intervals widened by every undecided member.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryCoverageServiceHandle, BoundaryPlacement, CapabilityRegistry, CoverageAreas,
    EvidenceSession, MeasuredBoundary, PlanArea, PlanAreaError, PlanAreaService,
    PlanAreaServiceHandle, SurfaceAreaInterval, compile,
};
use axioval_ir::{
    BODY_SET, ColumnExactness, Evidence, NotEvaluatedReason, ObjectId, PropertyValue,
    QuantityDimension, Report, ReportColumn, ReportValue, RuleId, RuleSetPackage, Scope,
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
        &["wall", "storey", "space", "door", "window", "column"],
        &["TypeName", "Name", "NetSideArea", "Status"],
        &["Pset_WallCommon"],
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
            ReportColumn::number("count").with_exactness(ColumnExactness::Exact),
            ReportColumn::quantity("sum_net_side_area", QuantityDimension::Area)
                .with_exactness(ColumnExactness::Exact),
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
            .with_exactness(ColumnExactness::Bounded)
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

fn strings(values: &[&str]) -> Value {
    json!({ "type": "stringList", "value": values })
}

fn selector(value: Value) -> Value {
    json!({ "type": "selector", "value": value })
}

/// The ids of a takeoff's columns, in order.
fn column_ids(report: &Report, rule: &str) -> Vec<String> {
    report
        .table(&RuleId::new(rule).unwrap(), "takeoff")
        .unwrap()
        .columns()
        .iter()
        .map(|column| column.id.clone())
        .collect()
}

fn listed(value: &str) -> ReportValue {
    ReportValue::text(value)
}

/// Each space's declared boundaries, the evidence exact where `exact`.
struct Boundaries(BTreeMap<ObjectId, (Vec<MeasuredBoundary>, bool)>);

impl BoundaryCoverageService for Boundaries {
    fn measure_boundary_coverage(
        &self,
        request: &BoundaryCoverageRequest,
    ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
        let (boundaries, exact) = self
            .0
            .get(request.space())
            .cloned()
            .ok_or_else(|| BoundaryCoverageError::UnknownSpace(request.space().clone()))?;
        let area = |value: f64| SurfaceAreaInterval::exact(value).unwrap();
        let mut evidence = Evidence::exact(source(), format!("coverage:{}", request.space()));
        evidence.exact = exact;
        BoundaryCoverage::try_new(
            request.clone(),
            CoverageAreas {
                surface: area(59.0),
                covered: area(59.0),
                uncovered: area(0.0),
                overlap: area(0.0),
            },
            boundaries,
            vec![],
            evidence,
        )
    }
}

/// A boundary of `area` (m², an interval when two values) bounding against
/// `element`, when it names one.
fn boundary(local: &str, element: Option<&str>, area: (f64, f64)) -> MeasuredBoundary {
    MeasuredBoundary::new(
        id(local),
        element.map(id),
        BoundaryPlacement::OnSurface {
            area: SurfaceAreaInterval::try_new(area.0, area.1).unwrap(),
        },
    )
}

/// Rooms `office` and `lab` between walls `wa1` (new) and `wa2`, window
/// `wi1` and door `d1`; `wa2` states its status as a list, which `equals`
/// cannot decide without a quantifier.
fn rooms() -> (Model, Boundaries) {
    let model = Model::default()
        .object("office", "space")
        .object("lab", "space")
        .object("wa1", "wall")
        .object("wa2", "wall")
        .object("wi1", "window")
        .object("d1", "door")
        .object("c1", "column")
        .text("office", "Pset", "Name", "Office")
        .text("lab", "Pset", "Name", "Lab")
        .text("wa1", "Pset", "Status", "new")
        .value(
            "wa2",
            "Pset",
            "Status",
            PropertyValue::List(vec![PropertyValue::String("new".into())]),
        );
    let boundaries = Boundaries(BTreeMap::from([
        (
            id("office"),
            (
                vec![
                    boundary("b1", Some("wa1"), (10.0, 10.0)),
                    boundary("b2", Some("wa2"), (7.5, 7.5)),
                    boundary("b3", Some("wi1"), (2.0, 2.0)),
                    boundary("b4", Some("d1"), (1.8, 1.8)),
                    // A virtual boundary bounds against no element.
                    boundary("b5", None, (3.0, 3.0)),
                    boundary("b6", Some("c1"), (0.5, 0.5)),
                ],
                true,
            ),
        ),
        (
            id("lab"),
            (
                vec![
                    boundary("b7", Some("wa2"), (12.0, 12.5)),
                    boundary("b8", Some("d1"), (1.8, 1.8)),
                ],
                false,
            ),
        ),
    ]));
    (model, boundaries)
}

fn boundary_session(model: Model, boundaries: Boundaries) -> EvidenceSession {
    session(model)
        .with_host_service(
            BoundaryCoverageServiceHandle::new(Arc::new(boundaries)),
            &[snapshot()],
        )
        .unwrap()
}

/// Spaces by name, with the areas of their boundaries against walls
/// (selected by `walls`), windows and doors.
fn boundary_takeoff(walls: Value) -> RuleSetPackage {
    ruleset(vec![rule(
        "space-takeoff",
        TAKEOFF,
        "info",
        entity("space"),
        json!({
            "group_1": reference(None, "t.Name"),
            "group_1_name": text("space"),
            "measure_1_kind": text("boundary_area"),
            "measure_1_bounding": selector(walls),
            "measure_1_name": text("wall_area"),
            "measure_2_kind": text("boundary_area"),
            "measure_2_bounding": selector(entity("window")),
            "measure_2_name": text("window_area"),
            "measure_3_kind": text("boundary_area"),
            "measure_3_bounding": selector(entity("door")),
            "measure_3_name": text("door_area"),
        }),
        json!({}),
    )])
}

#[test]
fn a_space_takeoff_lists_wall_window_and_door_boundary_areas() {
    let (model, boundaries) = rooms();
    let report = check(
        &boundary_takeoff(entity("wall")),
        &boundary_session(model, boundaries),
    );
    let table = report
        .table(&RuleId::new("space-takeoff").unwrap(), "takeoff")
        .unwrap();
    let area = |name: &str| {
        ReportColumn::quantity(name, QuantityDimension::Area)
            .with_exactness(ColumnExactness::Bounded)
    };
    assert_eq!(
        table.columns(),
        [
            ReportColumn::number("count").with_exactness(ColumnExactness::Exact),
            area("sum_wall_area"),
            area("sum_window_area"),
            area("sum_door_area"),
        ]
    );
    // A boundary against no element, or one of another kind, counts in no
    // column; the lab's wall boundary is measured within half a square
    // metre.
    assert_eq!(
        rows(&report, "space-takeoff"),
        [
            row(
                &["Lab"],
                &[exact(1.0), between(12.0, 12.5), exact(0.0), exact(1.8)]
            ),
            row(
                &["Office"],
                &[exact(1.0), exact(17.5), exact(2.0), exact(1.8)]
            ),
        ]
    );
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
}

#[test]
fn an_undecided_bounding_element_widens_the_area_and_a_misplaced_one_leaves_it_unknown() {
    let (model, mut boundaries) = rooms();
    let model = model
        .object("store", "space")
        .text("store", "Pset", "Name", "Store");
    boundaries.0.insert(
        id("store"),
        (
            vec![MeasuredBoundary::new(
                id("b9"),
                Some(id("wa1")),
                BoundaryPlacement::OffSurface,
            )],
            true,
        ),
    );
    let new_walls = json!({ "kind": "allOf", "operands": [
        entity("wall"),
        { "kind": "property", "property": "t.Status", "operator": "equals",
          "value": { "type": "string", "value": "new" } },
    ] });
    let report = check(
        &boundary_takeoff(new_walls),
        &boundary_session(model, boundaries),
    );
    let walls: Vec<ReportValue> = rows(&report, "space-takeoff")
        .into_iter()
        .map(|(_, values)| values[1].clone())
        .collect();
    // `wa2` may or may not be a new wall: its boundary may add its area.
    assert_eq!(
        walls,
        [
            between(0.0, 12.5),
            between(10.0, 17.5),
            ReportValue::Unknown
        ]
    );
    let open: Vec<(&Scope, &str)> = report
        .not_evaluated
        .iter()
        .map(|outcome| (&outcome.scope, outcome.message.as_str()))
        .collect();
    assert_eq!(open.len(), 3, "{open:?}");
    assert!(
        open.iter()
            .any(|(scope, message)| **scope == Scope::Object(id("office"))
                && message.contains("may or may not include it"))
    );
    assert!(
        open.iter()
            .any(|(scope, message)| **scope == Scope::Object(id("store"))
                && message.contains("lies on no face of the space's body"))
    );
}

/// Walls of types `A` and `B` stating `Pset_WallCommon` in part.
fn common_walls() -> Model {
    let transmittance = |value: f64| PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Other {
            exponents: [0, 1, -3, 0, -1, 0, 0],
        },
    };
    let set = "Pset_WallCommon";
    Model::default()
        .object("c1", "wall")
        .object("c2", "wall")
        .object("c3", "wall")
        .text("c1", "Pset", "TypeName", "A")
        .text("c2", "Pset", "TypeName", "A")
        .text("c3", "Pset", "TypeName", "B")
        .text("c1", set, "FireRating", "F90")
        .text("c2", set, "FireRating", "F30")
        .text("c3", set, "FireRating", "F90")
        .value("c1", set, "IsExternal", PropertyValue::Boolean(true))
        .value("c2", set, "IsExternal", PropertyValue::Boolean(false))
        .value("c1", set, "ThermalTransmittance", transmittance(0.24))
        .value("c2", set, "ThermalTransmittance", transmittance(0.28))
}

fn set_takeoff(extra: Value) -> RuleSetPackage {
    let mut parameters = json!({
        "group_1": reference(None, "t.TypeName"),
        "group_1_name": text("type"),
        "measure_1_kind": text("property_set"),
        "measure_1_property_set": text("t.Pset_WallCommon"),
    });
    for (key, value) in extra.as_object().unwrap() {
        parameters[key] = value.clone();
    }
    ruleset(vec![rule(
        "set-takeoff",
        TAKEOFF,
        "info",
        entity("wall"),
        parameters,
        json!({}),
    )])
}

#[test]
fn a_property_set_column_expands_into_its_properties() {
    let report = check(&set_takeoff(json!({})), &session(common_walls()));
    // One column per property found on any member, by property name.
    assert_eq!(
        column_ids(&report, "set-takeoff"),
        [
            "count",
            "values_fire_rating",
            "values_is_external",
            "values_thermal_transmittance"
        ]
    );
    let u = "kg·s⁻³·K⁻¹";
    assert_eq!(
        rows(&report, "set-takeoff"),
        [
            row(
                &["A"],
                &[
                    exact(2.0),
                    listed("F30, F90"),
                    listed("false, true"),
                    listed(&format!("0.24 {u}, 0.28 {u}")),
                ]
            ),
            row(
                &["B"],
                &[exact(1.0), listed("F90"), listed("-"), listed("-")]
            ),
        ]
    );
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );

    // Numeric aggregates apply to the numeric properties; a member without
    // one leaves its group's aggregate unknown.
    let report = check(
        &set_takeoff(json!({
            "measure_1_aggregates": strings(&["max"]),
            "measure_1_name": text("common"),
        })),
        &session(common_walls()),
    );
    let table = report
        .table(&RuleId::new("set-takeoff").unwrap(), "takeoff")
        .unwrap();
    assert_eq!(
        table.columns()[3],
        ReportColumn::quantity(
            "max_common_thermal_transmittance",
            QuantityDimension::Other {
                exponents: [0, 1, -3, 0, -1, 0, 0]
            }
        )
        .with_exactness(ColumnExactness::Exact)
    );
    let maxima: Vec<ReportValue> = rows(&report, "set-takeoff")
        .into_iter()
        .map(|(_, values)| values[3].clone())
        .collect();
    assert_eq!(maxima, [exact(0.28), ReportValue::Unknown]);
    assert_eq!(
        open(&report),
        [(
            Scope::Object(id("c3")),
            NotEvaluatedReason::IncompleteEvidence
        )]
    );
}

#[test]
fn a_door_takeoff_shows_its_storey_through_a_related_column() {
    let model = Model::default()
        .object("l1", "storey")
        .object("l2", "storey")
        .text("l1", "Pset", "Name", "Level 1")
        .text("l2", "Pset", "Name", "Level 2")
        .object("d1", "door")
        .object("d2", "door")
        .object("d3", "door")
        .text("d1", "Pset", "TypeName", "T90")
        .text("d2", "Pset", "TypeName", "T90")
        .text("d3", "Pset", "TypeName", "T30")
        .edge("contains", "l1", "d1")
        .edge("contains", "l2", "d2")
        .edge("contains", "l1", "d3");
    let takeoff = rule(
        "door-takeoff",
        TAKEOFF,
        "info",
        entity("door"),
        json!({
            "group_1": reference(None, "t.TypeName"),
            "group_1_name": text("type"),
            "measure_1_kind": text("related"),
            "measure_1": reference(None, "t.Name"),
            "measure_1_path": strings(&["contains:backward"]),
            "measure_1_name": text("storey"),
        }),
        json!({}),
    );
    let report = check(&ruleset(vec![takeoff]), &session(model));
    let table = report
        .table(&RuleId::new("door-takeoff").unwrap(), "takeoff")
        .unwrap();
    assert_eq!(
        table.columns()[1],
        ReportColumn::text("values_storey").with_exactness(ColumnExactness::Exact)
    );
    assert_eq!(
        rows(&report, "door-takeoff"),
        [
            row(&["T30"], &[exact(1.0), listed("Level 1")]),
            row(&["T90"], &[exact(2.0), listed("Level 1, Level 2")]),
        ]
    );
    assert!(report.not_evaluated.is_empty());
}

#[test]
fn a_related_number_is_summed_over_the_objects_reached() {
    // Wall `w1` holds openings of 2 and 1.5 m²; `w4` holds none, so it has
    // no opening area to add, never a zero.
    let model = walls()
        .object("o1", "space")
        .object("o2", "space")
        .value("o1", "Qto", "NetSideArea", area(2.0))
        .value("o2", "Qto", "NetSideArea", area(1.5))
        .edge("voids", "w1", "o1")
        .edge("voids", "w1", "o2");
    let takeoff = rule(
        "openings",
        TAKEOFF,
        "info",
        entity("wall"),
        json!({
            "group_1": reference(None, "t.TypeName"),
            "measure_1_kind": text("related"),
            "measure_1": reference(None, "t.NetSideArea"),
            "measure_1_path": strings(&["voids:forward"]),
            "measure_1_name": text("openings"),
            "measure_1_aggregates": strings(&["max", "values"]),
        }),
        json!({}),
    );
    let report = check(&ruleset(vec![takeoff]), &session(model));
    assert_eq!(
        column_ids(&report, "openings"),
        ["count", "max_openings", "values_openings"]
    );
    let values: Vec<Vec<ReportValue>> = rows(&report, "openings")
        .into_iter()
        .map(|(_, values)| values)
        .collect();
    // Type A: `w1` reaches 3.5 m², `w2` and `w3` reach nothing.
    assert_eq!(
        values[0],
        [exact(3.0), ReportValue::Unknown, listed("3.5 m²")]
    );
    assert_eq!(values[1], [exact(1.0), ReportValue::Unknown, listed("-")]);
    assert!(
        report
            .not_evaluated
            .iter()
            .all(|outcome| outcome.message.contains("has no value"))
    );
    assert_eq!(report.not_evaluated.len(), 3);
}

/// A one-item extrusion of `family`, named `name`, with `dimensions` in
/// metres.
fn profiled(
    model: Model,
    local: &str,
    family: &str,
    name: Option<&str>,
    dimensions: &[(&str, f64)],
) -> Model {
    let mut model = model
        .object(local, "column")
        .value(local, BODY_SET, "Count", PropertyValue::Integer(1))
        .text(local, BODY_SET, "Kind", "extrusion")
        .text(local, BODY_SET, "Profile.Type", family);
    if let Some(name) = name {
        model = model.text(local, BODY_SET, "Profile.Name", name);
    }
    for (parameter, value) in dimensions {
        model = model.value(
            local,
            BODY_SET,
            &format!("Profile.{parameter}"),
            PropertyValue::Quantity {
                value: *value,
                dimension: QuantityDimension::Length,
            },
        );
    }
    model
}

#[test]
fn a_profile_column_gives_the_type_name_and_dimensions() {
    let model = Model::default();
    let model = profiled(
        model,
        "k1",
        "i-shape",
        Some("HEA300"),
        &[("OverallWidth", 0.3), ("OverallDepth", 0.29)],
    );
    let model = profiled(
        model,
        "k2",
        "i-shape",
        Some("HEA300"),
        &[("OverallWidth", 0.3), ("OverallDepth", 0.295)],
    );
    let model = profiled(
        model,
        "k3",
        "rectangle",
        None,
        &[("XDim", 0.4), ("YDim", 0.4)],
    )
    .text("k1", "Pset", "TypeName", "steel")
    .text("k2", "Pset", "TypeName", "steel")
    .text("k3", "Pset", "TypeName", "concrete");
    let takeoff = rule(
        "profiles",
        TAKEOFF,
        "info",
        entity("column"),
        json!({
            "group_1": reference(None, "t.TypeName"),
            "measure_1_kind": text("profile"),
            "measure_1_aggregates": strings(&["min", "max"]),
        }),
        json!({}),
    );
    let report = check(&ruleset(vec![takeoff]), &session(model));
    let table = report
        .table(&RuleId::new("profiles").unwrap(), "takeoff")
        .unwrap();
    assert_eq!(
        column_ids(&report, "profiles"),
        [
            "count",
            "values_profile_type",
            "values_profile_name",
            "min_profile_width",
            "max_profile_width",
            "min_profile_depth",
            "max_profile_depth",
        ]
    );
    assert_eq!(
        table.columns()[3],
        ReportColumn::quantity("min_profile_width", QuantityDimension::Length)
            .with_exactness(ColumnExactness::Exact)
    );
    assert_eq!(
        rows(&report, "profiles"),
        [
            row(
                &["concrete"],
                &[
                    exact(1.0),
                    listed("rectangle"),
                    listed("-"),
                    exact(0.4),
                    exact(0.4),
                    exact(0.4),
                    exact(0.4),
                ]
            ),
            row(
                &["steel"],
                &[
                    exact(2.0),
                    listed("i-shape"),
                    listed("HEA300"),
                    exact(0.3),
                    exact(0.3),
                    exact(0.29),
                    exact(0.295),
                ]
            ),
        ]
    );
    assert!(
        report.not_evaluated.is_empty(),
        "{:?}",
        report.not_evaluated
    );
}

#[test]
fn malformed_column_kinds_leave_the_rule_not_evaluated() {
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
    let message = declared(json!({ "measure_1_kind": text("volume") }));
    assert!(message.contains("`volume` is unsupported"), "{message}");
    let message = declared(json!({
        "measure_1_kind": text("related"),
        "measure_1": reference(None, "t.Name"),
    }));
    assert!(message.contains("requires `measure_1_path`"), "{message}");
    let message = declared(json!({ "measure_1_kind": text("boundary_area") }));
    assert!(
        message.contains("requires `measure_1_bounding`"),
        "{message}"
    );
    let message = declared(json!({
        "measure_1": reference(None, "t.NetSideArea"),
        "measure_1_path": strings(&["contains:backward"]),
    }));
    assert!(
        message.contains("does not apply to a `property`"),
        "{message}"
    );
    let message = declared(json!({
        "measure_1_kind": text("boundary_area"),
        "measure_1_bounding": selector(entity("wall")),
        "measure_2_kind": text("boundary_area"),
        "measure_2_bounding": selector(entity("door")),
    }));
    assert!(message.contains("twice"), "{message}");
}
