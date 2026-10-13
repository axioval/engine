//! Location-dependent design data as context (#324): a snow load zone
//! map supplied as context zones whose features state values, read by a
//! `zone` expression at each building's position and used as a `lookup`
//! key; and a design speed stated per station range of an alignment, read
//! by a `catalogue` expression at an object's station.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityRegistry, ContextZones, EngineError, ExecutionPlan, PlanArea, PlanAreaError,
    PlanAreaService, PlanAreaServiceHandle, SuppliedCatalogue, ZoneArea, ZoneAreaRequest, compile,
};
use axioval_ir::contract::{ParameterDefinition, PropertyValueKind};
use axioval_ir::{
    DefinitionPackage, Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension,
    Report,
};
use axioval_rules::register_builtins;
use common::runtime::{definition, definitions, entity, rule, ruleset, run, session, snapshot};
use common::{Model, id, source};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

/// Axis-aligned rectangular footprints `[x0, y0, x1, y1]`; zones are read
/// by their outer ring's bounding box, so the tests state rectangles only.
struct Rectangles(BTreeMap<ObjectId, [f64; 4]>);

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let width = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let depth = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    width * depth
}

fn size(r: [f64; 4]) -> f64 {
    (r[2] - r[0]) * (r[3] - r[1])
}

impl PlanAreaService for Rectangles {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let rectangle = self
            .0
            .get(object)
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let area = size(*rectangle);
        PlanArea::try_new(area, area, Evidence::exact(source(), "footprint"))
    }

    fn measure_plan_overlap(&self, _: &ObjectId, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not measured here".into()))
    }

    fn measure_zone_area(&self, request: &ZoneAreaRequest) -> Result<ZoneArea, PlanAreaError> {
        let outer = request.zone()[0].outer();
        let fold = |pick: fn(f64, f64) -> f64, axis: usize, start: f64| {
            outer.iter().map(|point| point[axis]).fold(start, pick)
        };
        let zone = [
            fold(f64::min, 0, f64::INFINITY),
            fold(f64::min, 1, f64::INFINITY),
            fold(f64::max, 0, f64::NEG_INFINITY),
            fold(f64::max, 1, f64::NEG_INFINITY),
        ];
        let (mut whole, mut inside) = (0.0, 0.0);
        for object in request.objects() {
            let rectangle = self
                .0
                .get(object)
                .copied()
                .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
            whole += size(rectangle);
            inside += overlap(rectangle, zone);
        }
        ZoneArea::try_new(
            request.objects().to_vec(),
            (whole, whole),
            (inside, inside),
            (whole - inside, whole - inside),
            Evidence::exact(source(), "zone-area"),
        )
    }
}

/// A synthetic snow load zone map in the model's coordinates: three bands
/// along x, each stating its zone and characteristic load on the ground.
const SNOW_MAP: &str = r#"{"type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "axioval:model"}},
  "accuracy": 0,
  "features": [
    {"type": "Feature", "id": "SL-1", "properties": {"kind": "snow-load-zone",
      "values": {"load_zone": "1", "characteristic_load": {"value": 0.65, "unit": "kN/m²"}}},
     "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [100, 0], [100, 100], [0, 100], [0, 0]]]}},
    {"type": "Feature", "id": "SL-2a", "properties": {"kind": "snow-load-zone",
      "values": {"load_zone": "2a", "characteristic_load": {"value": 1.06, "unit": "kN/m²"}}},
     "geometry": {"type": "Polygon", "coordinates": [[[100, 0], [200, 0], [200, 100], [100, 100], [100, 0]]]}},
    {"type": "Feature", "id": "SL-3", "properties": {"kind": "snow-load-zone",
      "values": {"load_zone": "3"}},
     "geometry": {"type": "Polygon", "coordinates": [[[200, 0], [300, 0], [300, 100], [200, 100], [200, 0]]]}}
  ]}"#;

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

/// Buildings designed for a snow load, signs at a station, and the
/// expression definition taking the table `loads` (load zone → least
/// design load).
fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION],
        &["building", "sign"],
        &["DesignSnowLoad", "Station"],
        &["Pset"],
    );
    let station = package.properties.get_mut("t.Station").unwrap();
    station.value_kind = PropertyValueKind::Quantity;
    station.unit_dimension = Some("length".into());
    package
        .properties
        .get_mut("t.DesignSnowLoad")
        .unwrap()
        .value_kind = PropertyValueKind::Number;
    let loads: ParameterDefinition = serde_json::from_value(json!({
    "id": "loads", "name": text("design snow loads"), "kind": "table", "required": false,
    "columns": [
        {"id": "zone", "name": text("zone"), "kind": "textPattern", "required": false},
        {"id": "load", "name": text("load"), "kind": "number", "required": false},
    ]}))
    .unwrap();
    package
        .definitions
        .get_mut(&definition(EXPRESSION))
        .unwrap()
        .parameters
        .insert(loads.id.clone(), loads);
    package
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": format!("t.{name}")})
}

fn zone(value: Option<&str>) -> Value {
    let mut node = json!({"kind": "zone", "standsIn": "snow-load-zone", "label": "snow"});
    if let Some(value) = value {
        node["value"] = json!(value);
    }
    node
}

fn expression_rule(id: &str, requirement: &Value, parameters: &Value) -> Value {
    let mut parameters = parameters.clone();
    parameters["requirement"] = json!({"type": "expression", "value": requirement});
    rule(
        id,
        EXPRESSION,
        "error",
        entity("building"),
        parameters,
        json!({}),
    )
}

/// The design snow load is at least the table's for the zone the building
/// stands in, keyed by the zone the map states there.
fn designed_rule() -> Value {
    let row = |zone: &str, load: f64| json!({"zone": {"type": "string", "value": zone}, "load": {"type": "number", "value": load}});
    expression_rule(
        "designed",
        &json!({"kind": "compare", "operator": "greaterThanOrEquals",
                "left": property("DesignSnowLoad"),
                "right": {"kind": "lookup", "table": "loads", "column": "load",
                          "keys": {"zone": zone(Some("load_zone"))}}}),
        &json!({"loads": {"type": "table", "value": [
            row("1", 0.65), row("2a", 1.06), row("3", 1.10)]}}),
    )
}

fn compiled(rules: Vec<Value>) -> Result<ExecutionPlan, EngineError> {
    let registry = registry();
    compile(&registry, &[vocabulary(&registry)], &ruleset(rules))
}

/// Buildings by local id, footprint and design snow load.
fn buildings() -> (Model, Rectangles) {
    let mut model = Model::default();
    let mut rectangles = BTreeMap::new();
    for (local, rectangle, load) in [
        ("barn", [10.0, 10.0, 30.0, 25.0], 0.70),
        ("house", [140.0, 20.0, 152.0, 30.0], 0.90),
        ("hall", [230.0, 40.0, 260.0, 70.0], 1.20),
        ("shed", [95.0, 50.0, 105.0, 56.0], 1.20),
        ("far", [400.0, 0.0, 410.0, 10.0], 1.20),
    ] {
        model = model.object(local, "building").value(
            local,
            "Pset",
            "DesignSnowLoad",
            PropertyValue::Decimal(load),
        );
        rectangles.insert(id(local), rectangle);
    }
    (model, Rectangles(rectangles))
}

fn check(mut plan: ExecutionPlan, map: Option<&str>) -> Report {
    if let Some(map) = map {
        let zones = ContextZones::read_geojson(map.as_bytes(), "snow.geojson").unwrap();
        plan.supply_context_zones(zones).unwrap();
    }
    let (model, rectangles) = buildings();
    let session = session(model)
        .with_host_service(
            PlanAreaServiceHandle::new(Arc::new(rectangles)),
            &[snapshot()],
        )
        .unwrap();
    run(registry(), plan, &session, |runtime| runtime).unwrap()
}

fn findings(report: &Report) -> Vec<String> {
    let mut subjects: Vec<String> = report.findings().iter().map(common::subject).collect();
    subjects.sort();
    subjects
}

fn outcomes_open(report: &Report) -> BTreeMap<String, (NotEvaluatedReason, String)> {
    report
        .not_evaluated
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map(|object| object.local_id.clone())
                    .unwrap_or_default(),
                (outcome.reason.clone(), outcome.message.clone()),
            )
        })
        .collect()
}

#[test]
fn a_snow_load_zone_map_gives_the_zone_at_each_buildings_position() {
    // The zone itself: the barn stands in zone 1, the house in 2a and the
    // hall in 3, by plan containment; read as a `lookup` key.
    let plan = compiled(vec![designed_rule()]).unwrap();
    let report = check(plan, Some(SNOW_MAP));
    // The house is designed for 0.90 kN/m², below 2a's 1.06.
    assert_eq!(findings(&report), ["house"]);
    let finding = &report.findings()[0];
    assert!(
        finding.message.contains("zone SL-2a.load_zone = `2a`"),
        "{}",
        finding.message
    );
    // Cited as the map's statement, with how the zone was chosen.
    let cited: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .filter(|locator| locator.starts_with("axioval:zone;"))
        .collect();
    assert!(
        cited.contains(&"axioval:zone;stands_in=snow-load-zone;id=SL-2a:test:model/house"),
        "{cited:?}"
    );
    assert!(
        cited.iter().any(
            |locator| locator.contains("id=SL-2a;kind=snow-load-zone;sha256=")
                && locator.ends_with(";value=load_zone;stated=properties.values.load_zone")
        ),
        "{cited:?}"
    );
    // A building straddling two zones, and one in none, are not
    // evaluated, never given a zone.
    let open = outcomes_open(&report);
    assert_eq!(open.len(), 2, "{open:?}");
    assert!(open["shed"].1.contains("undecided"), "{:?}", open["shed"]);
    assert!(
        open["far"]
            .1
            .contains("stands in no supplied context zone of kind `snow-load-zone`"),
        "{:?}",
        open["far"]
    );
    assert_eq!(open["far"].0, NotEvaluatedReason::IncompleteEvidence);
}

#[test]
fn a_zone_states_quantities_its_id_and_null_for_what_it_leaves_unstated() {
    // The characteristic load the map states, a quantity in kN/m².
    let at_most = expression_rule(
        "light",
        &json!({"kind": "compare", "operator": "lessThanOrEquals",
                "left": zone(Some("characteristic_load")),
                "right": {"kind": "literal",
                          "value": {"type": "quantity", "value": 1.0, "unit": "kN/m²"}}}),
        &json!({}),
    );
    // The zone's id, without a value.
    let named = expression_rule(
        "named",
        &json!({"kind": "compare", "operator": "equals", "left": zone(None),
                "right": {"kind": "literal", "value": {"type": "string", "value": "SL-1"}}}),
        &json!({}),
    );
    let plan = compiled(vec![at_most, named]).unwrap();
    let report = check(plan, Some(SNOW_MAP));
    let by_rule = |rule: &str| -> Vec<String> {
        let mut subjects: Vec<String> = report
            .findings()
            .iter()
            .filter(|finding| finding.rule_id.to_string() == rule)
            .map(common::subject)
            .collect();
        subjects.sort();
        subjects
    };
    // 2a states 1.06 kN/m²; zone 3 states none: `null`, which never passes.
    assert_eq!(by_rule("light"), ["hall", "house"]);
    assert_eq!(by_rule("named"), ["hall", "house"]);
}

#[test]
fn without_a_zone_map_every_reading_building_is_not_evaluated() {
    let plan = compiled(vec![designed_rule()]).unwrap();
    assert_eq!(
        plan.context_zone_reads()
            .kinds
            .into_iter()
            .collect::<Vec<_>>(),
        ["snow-load-zone"]
    );
    let report = check(plan, None);
    assert!(report.findings().is_empty());
    let open = outcomes_open(&report);
    assert_eq!(open.len(), 5, "{open:?}");
    assert!(
        open["house"].1.contains("no context zones were supplied"),
        "{:?}",
        open["house"]
    );
    // A zone named by neither an id nor a kind fails compilation.
    let neither = expression_rule(
        "neither",
        &json!({"kind": "isDefined", "operand": {"kind": "zone", "value": "load_zone"}}),
        &json!({}),
    );
    assert!(
        compiled(vec![neither])
            .unwrap_err()
            .to_string()
            .contains("exactly one of `zone` and `standsIn`")
    );
}

/// Design speeds per station range of two invented alignments.
fn speeds() -> Value {
    json!({ "speeds": {
        "id": "speeds",
        "name": text("Design speeds"),
        "key": "alignment",
        "columns": [
            {"id": "alignment", "kind": "string"},
            {"id": "from", "header": "from station", "kind": "quantity", "unit": "m"},
            {"id": "to", "header": "to station", "kind": "quantity", "unit": "m"},
            {"id": "speed", "header": "design speed", "kind": "quantity", "unit": "km/h"},
        ],
        "ranges": {"from": "from", "to": "to"},
    } })
}

const SPEEDS: &str = "alignment,from station,to station,design speed\n\
                      ex-axis-1,0,800,50\n\
                      ex-axis-1,800,2400,70\n\
                      ex-axis-2,0,5000,100\n";

/// The design speed at the sign's station.
fn speed_at(at: &Value) -> Value {
    json!({"kind": "catalogue", "catalogue": "speeds", "column": "speed", "label": "speed",
           "key": {"kind": "literal", "value": {"type": "string", "value": "ex-axis-1"}},
           "at": at})
}

fn signs_plan(requirement: &Value) -> Result<ExecutionPlan, EngineError> {
    let registry = registry();
    let mut set = ruleset(vec![rule(
        "signs",
        EXPRESSION,
        "error",
        entity("sign"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )]);
    set.catalogues = serde_json::from_value(speeds()).unwrap();
    compile(&registry, &[vocabulary(&registry)], &set)
}

fn signs(plan: &mut ExecutionPlan) -> Report {
    let definition = plan.catalogues()[0].clone();
    let catalogue =
        SuppliedCatalogue::read(&definition, SPEEDS.as_bytes(), "speeds.csv", None, None).unwrap();
    plan.supply_catalogue("speeds", catalogue).unwrap();
    let mut model = Model::default();
    for (local, station) in [
        ("s1", Some(120.0)),
        ("s2", Some(1500.0)),
        ("s3", Some(3000.0)),
        ("s4", None),
    ] {
        model = model.object(local, "sign").value(
            local,
            "Pset",
            "Station",
            station.map_or(PropertyValue::Null, |value| PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Length,
            }),
        );
    }
    run(registry(), plan.clone(), &session(model), |runtime| runtime).unwrap()
}

#[test]
fn a_design_speed_stated_per_station_range_is_read_at_a_station() {
    // A sign for a 60 km/h stretch stands where the design speed is at
    // most 60 km/h.
    let requirement = json!({"kind": "compare", "operator": "lessThanOrEquals",
        "left": speed_at(&property("Station")),
        "right": {"kind": "literal", "value": {"type": "quantity", "value": 60, "unit": "km/h"}}});
    let mut plan = signs_plan(&requirement).unwrap();
    let report = signs(&mut plan);
    // s1 at 120 m is in the 50 km/h range; s2 at 1500 m in the 70 km/h
    // one. s4 states no station, so no range is named: `null`, which
    // never passes.
    assert_eq!(findings(&report), ["s2", "s4"]);
    let finding = &report.findings()[0];
    assert!(
        finding
            .message
            .contains("catalogue speeds[ex-axis-1 at 1500 m].speed"),
        "{}",
        finding.message
    );
    assert!(
        finding.evidence.iter().any(|evidence| evidence
            .locator
            .ends_with("#entry=ex-axis-1;range=800 m..2400 m")),
        "{:?}",
        finding.evidence
    );
    // Beyond the stated ranges: not evaluated, naming them.
    let open = outcomes_open(&report);
    assert_eq!(open.len(), 1, "{open:?}");
    assert!(
        open["s3"]
            .1
            .contains("states no range of `ex-axis-1` holding 3000 m"),
        "{:?}",
        open["s3"]
    );
}

#[test]
fn a_station_range_read_is_checked_when_compiled() {
    let refused = |requirement: Value| {
        signs_plan(&json!({"kind": "isDefined", "operand": requirement}))
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    };
    // A ranged catalogue is read at a position, a length.
    let mut plain = speed_at(&property("Station"));
    plain.as_object_mut().unwrap().remove("at");
    assert!(refused(plain).contains("read `at` a position"));
    assert!(
        refused(speed_at(
            &json!({"kind": "literal", "value": {"type": "quantity", "value": 2, "unit": "s"}})
        ))
        .contains("compares")
    );
    assert!(
        refused(speed_at(
            &json!({"kind": "literal", "value": {"type": "string", "value": "km 1"}})
        ))
        .contains("number")
    );
    // A literal station reads too.
    assert!(
        signs_plan(&json!({"kind": "isDefined", "operand": speed_at(&
            json!({"kind": "literal", "value": {"type": "quantity", "value": 0.5, "unit": "km"}}))}))
        .is_ok()
    );
}
