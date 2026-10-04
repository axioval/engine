//! Keyed limit tables: the applicable row is chosen by key values read from
//! the object or from related objects, and its limits bound a quantity.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axioval_engine::{
    CapabilityEvaluation, ElevationInterval, GeometryFidelity, ObjectBounds, PlanArea,
    PlanAreaError, PlanAreaService, PlanAreaServiceHandle, ProjectedDistanceEvidence,
    ProximityError, ProximityEvidence, ProximityProjection, ProximityRequest, ProximityService,
    ProximityServiceHandle, VerticalExtent, VerticalExtentError, VerticalExtentService,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, TableRow};
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId, PropertyValue, QuantityDimension};
use axioval_rules::KeyedLimit;
use common::{
    Model, assert_deviation, deviation_of, findings, flagged, id, kind, number, property, rule,
    source, string, strings, unevaluated,
};

const ID: &str = "axioval:capability.keyed-limit";

/// Plan areas per object, with an optional uncertainty around each.
#[derive(Default)]
struct Areas(BTreeMap<ObjectId, (f64, f64)>);

impl Areas {
    fn with(mut self, local: &str, area: f64, slack: f64) -> Self {
        self.0.insert(id(local), (area, slack));
        self
    }
}

impl PlanAreaService for Areas {
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        let (area, slack) = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| PlanAreaError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("footprint:{object}"));
        evidence.exact = slack == 0.0;
        PlanArea::try_new(area - slack, area + slack, evidence)
    }

    fn measure_plan_overlap(
        &self,
        _first: &ObjectId,
        _second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        Err(PlanAreaError::Unavailable("not measured here".into()))
    }
}

/// A row keyed by fire class, use class and sprinkler flag; `None` leaves
/// a key or bound blank.
fn row(keys: [Option<&str>; 3], minimum: Option<f64>, maximum: Option<f64>) -> TableRow {
    let mut cells = TableRow::new();
    for (column, key) in ["key_1", "key_2", "key_3"].into_iter().zip(keys) {
        if let Some(key) = key {
            cells.insert(column.into(), string(key));
        }
    }
    if let Some(minimum) = minimum {
        cells.insert("minimum".into(), number(minimum));
    }
    if let Some(maximum) = maximum {
        cells.insert("maximum".into(), number(maximum));
    }
    cells
}

fn table(rows: Vec<TableRow>) -> ParameterValue {
    ParameterValue::Table { value: rows }
}

/// Compartment area limits by building fire class and compartment use
/// class, with and without sprinklers on the compartment's storey.
fn fire_limits() -> Vec<TableRow> {
    vec![
        row([Some("1"), None, Some("false")], None, Some(400.0)),
        row([Some("1"), None, Some("true")], None, Some(800.0)),
        row(
            [Some("2"), Some("Office"), Some("false")],
            None,
            Some(1000.0),
        ),
        // Sprinklered offices in fire class 2 have no area limit.
        row([Some("2"), Some("Office"), Some("true")], None, None),
        row([Some("2"), Some("Retail*"), None], None, Some(600.0)),
    ]
}

/// The compartment's use class; the storey's sprinkler flag and the
/// building's fire class, reached by walking containment upwards.
fn fire_keys(limits: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("limits", table(limits)),
        ("quantity", string("plan-area")),
        ("key_1", property(Some("Pset"), "FireClass")),
        (
            "key_1_path",
            strings(&["contains:backward", "contains:backward"]),
        ),
        ("key_2", property(Some("Pset"), "UseClass")),
        ("key_3", property(Some("Pset"), "Sprinklered")),
        ("key_3_path", strings(&["contains:backward"])),
    ]
}

/// Building `b` of `fire_class` with an unsprinklered storey `dry` and a
/// sprinklered storey `wet`; each compartment `(name, storey, use class)`.
fn building(fire_class: Option<&str>, compartments: &[(&str, &str, &str)]) -> Model {
    let mut model = Model::default()
        .object("b", "building")
        .object("dry", "storey")
        .object("wet", "storey")
        .edge("contains", "b", "dry")
        .edge("contains", "b", "wet")
        .value("dry", "Pset", "Sprinklered", PropertyValue::Boolean(false))
        .value("wet", "Pset", "Sprinklered", PropertyValue::Boolean(true));
    if let Some(fire_class) = fire_class {
        model = model.text("b", "Pset", "FireClass", fire_class);
    }
    for (compartment, storey, use_class) in compartments {
        model = model
            .object(compartment, "compartment")
            .edge("contains", storey, compartment)
            .text(compartment, "Pset", "UseClass", use_class);
    }
    model
}

fn run(
    model: Model,
    areas: Areas,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("compartment"), parameters),
        |services| {
            services
                .register(PlanAreaServiceHandle::new(Arc::new(areas)))
                .unwrap();
        },
    )
}

#[test]
fn a_compartment_is_limited_by_its_building_fire_class_and_sprinklers() {
    let model = building(
        Some("1"),
        &[
            ("c1", "dry", "Office"),
            ("c2", "wet", "Office"),
            ("c3", "wet", "Office"),
            ("c4", "dry", "Office"),
        ],
    );
    let areas = Areas::default()
        .with("c1", 500.0, 0.0)
        .with("c2", 500.0, 0.0)
        .with("c3", 900.0, 0.0)
        .with("c4", 400.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    // 500 m² against 400 m², 900 m² against 800 m².
    assert_deviation(deviation_of(&evaluation, "plan area is 500"), (0.25, 0.25));
    assert_deviation(
        deviation_of(&evaluation, "plan area is 900"),
        (0.125, 0.125),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "c1".into(),
                "plan area is 500 m²; required at most 400 m² (limit row 0: Pset.FireClass \
                 (via contains then contains) `1`, Pset.UseClass `Office`, Pset.Sprinklered \
                 (via contains) `false`)"
                    .into()
            ),
            (
                "c3".into(),
                "plan area is 900 m²; required at most 800 m² (limit row 1: Pset.FireClass \
                 (via contains then contains) `1`, Pset.UseClass `Office`, Pset.Sprinklered \
                 (via contains) `true`)"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
    let related: Vec<_> = evaluation.findings()[0]
        .related
        .iter()
        .map(|object| object.local_id.as_str())
        .collect();
    assert_eq!(related, ["b", "dry"]);
    let cited: Vec<_> = evaluation.findings()[0]
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert!(
        cited.iter().any(|l| l.ends_with("b:Pset.FireClass")),
        "{cited:?}"
    );
    assert!(
        cited.iter().any(|l| l.starts_with("footprint:")),
        "{cited:?}"
    );
}

#[test]
fn a_row_without_bounds_sets_no_limit_and_patterns_select_rows() {
    let model = building(
        Some("2"),
        &[
            ("c1", "wet", "Office"),
            ("c2", "dry", "Office"),
            ("c3", "wet", "Retail food"),
        ],
    );
    let areas = Areas::default()
        .with("c1", 5000.0, 0.0)
        .with("c2", 1000.0, 0.0)
        .with("c3", 650.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert_eq!(
        common::flagged(&evaluation),
        ["c3"],
        "{:?}",
        findings(&evaluation)
    );
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("required at most 600 m² (limit row 4")
    );
}

#[test]
fn a_missing_key_is_not_evaluated() {
    let model = building(None, &[("c1", "dry", "Office")]);
    let areas = Areas::default().with("c1", 500.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message().to_owned();
    assert!(message.contains("Pset.FireClass of"), "{message}");
    assert!(message.ends_with("is absent"), "{message}");
}

#[test]
fn a_key_reached_on_no_object_is_not_evaluated() {
    // The compartment is on no storey: its sprinkler flag is unknown.
    let model = building(Some("1"), &[])
        .object("c1", "compartment")
        .text("c1", "Pset", "UseClass", "Office");
    let areas = Areas::default().with("c1", 500.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("reaches no object")
    );
}

#[test]
fn an_unknown_key_no_row_tests_does_not_matter() {
    // No row of fire class 3 exists, so the unknown use class cannot
    // change the outcome: no limit is defined.
    let model = building(Some("3"), &[])
        .object("c1", "compartment")
        .edge("contains", "dry", "c1");
    let areas = Areas::default().with("c1", 500.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert_eq!(
        findings(&evaluation),
        [(
            "c1".into(),
            "no limit defined for Pset.FireClass (via contains then contains) `3`, \
             Pset.UseClass unknown, Pset.Sprinklered (via contains) `false`"
                .into()
        )]
    );
}

#[test]
fn no_applicable_row_is_a_finding() {
    let model = building(Some("2"), &[("c1", "dry", "Storage")]);
    let areas = Areas::default().with("c1", 50.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert_eq!(
        findings(&evaluation),
        [(
            "c1".into(),
            "no limit defined for Pset.FireClass (via contains then contains) `2`, \
             Pset.UseClass `Storage`, Pset.Sprinklered (via contains) `false`"
                .into()
        )]
    );
}

#[test]
fn rows_tied_for_most_specific_are_not_evaluated() {
    let limits = vec![
        row([Some("1"), Some("Off*"), None], None, Some(400.0)),
        row([Some("1"), Some("*ice"), None], None, Some(800.0)),
    ];
    let model = building(Some("1"), &[("c1", "dry", "Office")]);
    let areas = Areas::default().with("c1", 500.0, 0.0);
    let evaluation = run(model, areas, fire_keys(limits));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("limit rows 0, 1 apply equally")
    );
}

#[test]
fn a_more_specific_row_wins_over_a_general_one() {
    let limits = vec![
        row([Some("1"), None, None], None, Some(400.0)),
        row([Some("1"), Some("Office"), None], None, Some(600.0)),
    ];
    let model = building(
        Some("1"),
        &[("c1", "dry", "Office"), ("c2", "dry", "Retail")],
    );
    let areas = Areas::default()
        .with("c1", 500.0, 0.0)
        .with("c2", 500.0, 0.0);
    let evaluation = run(model, areas, fire_keys(limits));
    assert_eq!(common::flagged(&evaluation), ["c2"]);
}

#[test]
fn an_area_straddling_the_limit_is_not_evaluated() {
    let model = building(Some("1"), &[("c1", "dry", "Office")]);
    let areas = Areas::default().with("c1", 400.0, 1.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn a_stated_quantity_is_checked_against_the_row() {
    // Occupant load per use class, stated on each room.
    let model = Model::default()
        .object("r1", "room")
        .object("r2", "room")
        .text("r1", "Pset", "UseClass", "Office")
        .text("r2", "Pset", "UseClass", "Assembly")
        .value("r1", "Pset", "Occupants", PropertyValue::Integer(30))
        .value(
            "r2",
            "Pset",
            "Area",
            PropertyValue::Quantity {
                value: 20.0,
                dimension: QuantityDimension::Area,
            },
        )
        .value("r2", "Pset", "Occupants", PropertyValue::Integer(10));
    let limits = vec![
        row([Some("Office"), None, None], Some(1.0), Some(20.0)),
        row([Some("Assembly"), None, None], Some(20.0), None),
    ];
    let evaluation = model.evaluate(
        &KeyedLimit,
        &rule(
            ID,
            kind("room"),
            vec![
                ("limits", table(limits)),
                ("quantity", string("property")),
                ("quantity_property", property(Some("Pset"), "Occupants")),
                ("key_1", property(Some("Pset"), "UseClass")),
            ],
        ),
    );
    assert_eq!(
        findings(&evaluation),
        [
            (
                "r1".into(),
                "Pset.Occupants is 30; required at most 20 (limit row 0: Pset.UseClass `Office`)"
                    .into()
            ),
            (
                "r2".into(),
                "Pset.Occupants is 10; required at least 20 (limit row 1: Pset.UseClass \
                 `Assembly`)"
                    .into()
            ),
        ]
    );
}

#[test]
fn a_key_that_differs_between_reached_objects_is_not_evaluated() {
    // A compartment spanning both storeys has no single sprinkler flag.
    let model = building(Some("1"), &[("c1", "dry", "Office")]).edge("contains", "wet", "c1");
    let areas = Areas::default().with("c1", 500.0, 0.0);
    let evaluation = run(model, areas, fire_keys(fire_limits()));
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("differs between")
    );
}

#[test]
fn declarations_are_checked() {
    let model = || building(Some("1"), &[("c1", "dry", "Office")]);
    let areas = || Areas::default().with("c1", 500.0, 0.0);
    let mut stray = fire_keys(vec![{
        let mut cells = row([Some("1"), None, None], None, Some(1.0));
        cells.insert("key_4".into(), string("x"));
        cells
    }]);
    stray.retain(|(name, _)| *name != "key_4");
    for parameters in [
        stray,
        {
            let mut parameters = fire_keys(fire_limits());
            parameters.push(("quantity_property", property(None, "Area")));
            parameters
        },
        {
            let mut parameters = fire_keys(fire_limits());
            parameters.retain(|(name, _)| *name != "key_1");
            parameters
        },
        fire_keys(vec![row([Some("1"), None, None], Some(2.0), Some(1.0))]),
        {
            let mut parameters = fire_keys(fire_limits());
            parameters.push(("key_4_path", strings(&["contains"])));
            parameters
        },
        {
            let mut parameters = fire_keys(fire_limits());
            parameters[1] = ("quantity", string("volume"));
            parameters
        },
    ] {
        let evaluation = run(model(), areas(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

#[test]
fn without_geometry_nothing_is_judged() {
    let model = building(Some("1"), &[("c1", "dry", "Office")]);
    let evaluation = model.evaluate(
        &KeyedLimit,
        &rule(ID, kind("compartment"), fire_keys(fire_limits())),
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("c1".to_owned(), NotEvaluatedReason::MissingService)]
    );
}

/// A package binds the capability's limit table: its declared columns must
/// be the capability's, and every row must fit them.
#[test]
#[allow(clippy::too_many_lines)] // the whole declared signature
fn a_package_binds_the_limit_table() {
    use axioval_engine::{CapabilityRegistry, EngineError, compile};
    use axioval_ir::{DefinitionPackage, RuleSetPackage};
    use serde_json::{Value, json};

    let text = |value: &str| json!({"default": value, "translations": {}});
    let parameter = |kind: &str, required: bool| {
        json!({"id": "", "name": text("p"), "kind": kind, "required": required,
               "allowedValues": [], "citations": []})
    };
    let column =
        |id: &str, kind: &str| json!({"id": id, "name": text(id), "kind": kind, "required": false});
    let mut parameters = serde_json::Map::new();
    let mut limits = parameter("table", true);
    limits["columns"] = json!([
        column("key_1", "textPattern"),
        column("key_2", "textPattern"),
        column("key_3", "textPattern"),
        column("key_4", "textPattern"),
        column("other_side", "textPattern"),
        column("minimum", "number"),
        column("maximum", "number"),
    ]);
    parameters.insert("limits".into(), limits);
    parameters.insert("quantity".into(), parameter("string", true));
    parameters.insert(
        "quantity_property".into(),
        parameter("propertyReference", false),
    );
    parameters.insert("case_sensitive".into(), parameter("boolean", false));
    parameters.insert("floor_path".into(), parameter("stringList", false));
    parameters.insert(
        "overall_width".into(),
        parameter("propertyReference", false),
    );
    parameters.insert("width_deduction".into(), parameter("quantity", false));
    parameters.insert("clear_width_from_leaves".into(), parameter("string", false));
    for name in ["overall_height", "lining_thickness", "threshold_thickness"] {
        parameters.insert(name.into(), parameter("propertyReference", false));
    }
    parameters.insert("ramp_selector".into(), parameter("selector", false));
    parameters.insert("ramp_reach".into(), parameter("quantity", false));
    parameters.insert("member_selector".into(), parameter("selector", false));
    parameters.insert("pair_key".into(), parameter("string", false));
    parameters.insert("measured_value".into(), parameter("string", false));
    let mut defaults = parameter("table", false);
    defaults["columns"] = json!([
        column("operation", "textPattern"),
        column("applies_to", "selector"),
        column("width_deduction", "quantity"),
        column("height_deduction", "quantity"),
        column("threshold_height", "quantity"),
        column("glazing_ratio", "number"),
    ]);
    parameters.insert("door_type_defaults".into(), defaults);
    for (name, kind) in [
        ("relationship", "string"),
        ("direction", "string"),
        ("follow_chain", "boolean"),
        ("path", "stringList"),
        ("skip_absent_relationship_ends", "boolean"),
    ] {
        parameters.insert(name.into(), parameter(kind, false));
    }
    for index in 1..=4 {
        parameters.insert(
            format!("key_{index}"),
            parameter("propertyReference", index == 1),
        );
        parameters.insert(format!("key_{index}_path"), parameter("stringList", false));
    }
    for (name, value) in &mut parameters {
        value["id"] = json!(name);
    }
    let mut definitions: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap();
    let definition = &mut definitions["definitions"]["axioval:example.property-exists"];
    definition["capability"] = json!(ID);
    definition["parameters"] = Value::Object(parameters);
    let definitions: DefinitionPackage = serde_json::from_value(definitions).unwrap();

    let ruleset = |rows: Value| {
        let mut ruleset: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/schema-v0.1.0/ruleset.json"
        ))
        .unwrap();
        let key = ruleset["root"]["rules"][0]["parameters"]["property"].clone();
        ruleset["root"]["rules"][0]["parameters"] = json!({
            "limits": {"type": "table", "value": rows},
            "quantity": {"type": "string", "value": "plan-area"},
            "key_1": key,
        });
        serde_json::from_value::<RuleSetPackage>(ruleset).unwrap()
    };
    let registry = CapabilityRegistry::new().register(KeyedLimit).unwrap();
    let limit = json!({"key_1": {"type": "string", "value": "A*"},
                       "maximum": {"type": "number", "value": 400.0}});
    compile(
        &registry,
        std::slice::from_ref(&definitions),
        &ruleset(json!([limit])),
    )
    .unwrap();
    let wrong = json!({"key_1": {"type": "string", "value": "A"},
                       "maximum": {"type": "quantity", "value": 400.0, "unit": "m2"}});
    assert!(matches!(
        compile(&registry, &[definitions], &ruleset(json!([wrong]))),
        Err(EngineError::InvalidTableRow { .. })
    ));
}

/// Bottom elevations per object, with an optional uncertainty around each;
/// every body is 1 m tall.
#[derive(Default)]
struct Bottoms(BTreeMap<ObjectId, (f64, f64)>);

impl Bottoms {
    fn with(mut self, local: &str, bottom: f64, slack: f64) -> Self {
        self.0.insert(id(local), (bottom, slack));
        self
    }
}

impl VerticalExtentService for Bottoms {
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let (bottom, slack) = self
            .0
            .get(object)
            .copied()
            .ok_or_else(|| VerticalExtentError::UnknownObject(object.clone()))?;
        let mut evidence = Evidence::exact(source(), format!("extent:{}", object.local_id));
        evidence.exact = slack == 0.0;
        VerticalExtent::try_new(
            object.clone(),
            ElevationInterval::try_new(bottom - slack, bottom + slack)?,
            ElevationInterval::try_new(bottom + 1.0 - slack, bottom + 1.0 + slack)?,
            evidence,
        )
    }
}

/// Offices allow a sill of at most 1 m, corridors any sill.
fn sill_limits() -> Vec<TableRow> {
    vec![
        row([Some("Office"), None, None], None, Some(1.0)),
        row([Some("Corridor"), None, None], None, None),
    ]
}

/// Keyed on the use of the spaces each window adjoins; sill heights from
/// those spaces' floors.
fn sill_keys(limits: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![
        ("limits", table(limits)),
        ("quantity", string("sill-height")),
        ("floor_path", strings(&["adjacent"])),
        ("key_1", property(Some("Pset"), "Use")),
        ("key_1_path", strings(&["adjacent"])),
    ]
}

/// Offices `o1` (floor 0 m) and `o2` (floor 0.5 m), corridor `k` (0 m) and
/// office `o3`, which cannot be measured; each window `(name, spaces)`.
fn rooms(windows: &[(&str, &[&str])]) -> Model {
    let mut model = Model::default();
    for (space, use_class) in [
        ("o1", "Office"),
        ("o2", "Office"),
        ("k", "Corridor"),
        ("o3", "Office"),
    ] {
        model = model
            .object(space, "space")
            .text(space, "Pset", "Use", use_class);
    }
    for (window, spaces) in windows {
        model = model.object(window, "window");
        for space in *spaces {
            model = model.edge("adjacent", window, space);
        }
    }
    model
}

fn floors() -> Bottoms {
    Bottoms::default()
        .with("o1", 0.0, 0.0)
        .with("o2", 0.5, 0.0)
        .with("k", 0.0, 0.0)
}

fn sill(
    model: Model,
    bottoms: Bottoms,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("window"), parameters),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(bottoms)))
                .unwrap();
        },
    )
}

#[test]
fn a_window_too_high_above_one_of_its_spaces_floors_is_found() {
    // w1 lies between two offices whose floors differ: 1.2 m above o1's,
    // 0.7 m above o2's. w2 sits 0.9 m above o1, w3 2 m above a corridor.
    let model = rooms(&[("w1", &["o1", "o2"]), ("w2", &["o1"]), ("w3", &["k"])]);
    let bottoms = floors()
        .with("w1", 1.2, 0.0)
        .with("w2", 0.9, 0.0)
        .with("w3", 2.0, 0.0);
    let evaluation = sill(model, bottoms, sill_keys(sill_limits()));
    // 1.2 m against at most 1 m: 20 % too high.
    let high = deviation_of(&evaluation, "sill height");
    assert!(high.0 > 0.19 && high.1 < 0.21, "{high:?}");
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            format!(
                "sill height above the floor of {} is 1.2 m; required at most 1 m \
                 (limit row 0: Pset.Use (via adjacent) `Office`)",
                id("o1")
            )
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
    let finding = &evaluation.findings()[0];
    let related: Vec<_> = finding
        .related
        .iter()
        .map(|object| object.local_id.as_str())
        .collect();
    assert_eq!(related, ["o1", "o2"]);
    let cited: Vec<_> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    assert!(
        cited.contains(&"extent:w1") && cited.contains(&"extent:o1"),
        "{cited:?}"
    );
    assert!(!cited.contains(&"extent:o2"), "{cited:?}");
}

#[test]
fn a_sill_height_straddling_the_limit_is_not_evaluated() {
    // A tessellated window whose bottom lies within 1 cm of 1 m.
    let model = rooms(&[("w1", &["o1"]), ("w2", &["o1"])]);
    let bottoms = floors().with("w1", 1.0, 0.01).with("w2", 1.1, 0.01);
    let evaluation = sill(model, bottoms, sill_keys(sill_limits()));
    assert_eq!(flagged(&evaluation), ["w2"]);
    assert_eq!(
        unevaluated(&evaluation),
        [("w1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("straddles the bound at most 1 m")
    );
}

#[test]
fn an_unmeasured_floor_is_not_evaluated_unless_another_floor_fails() {
    let model = rooms(&[("w1", &["o1", "o3"]), ("w2", &["o1", "o3"])]);
    let bottoms = floors().with("w1", 0.5, 0.0).with("w2", 1.5, 0.0);
    let evaluation = sill(model, bottoms, sill_keys(sill_limits()));
    assert_eq!(flagged(&evaluation), ["w2"]);
    assert_eq!(
        unevaluated(&evaluation),
        [("w1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.not_evaluated_outcomes()[0]
            .message()
            .contains("cannot be measured")
    );
}

#[test]
fn a_window_between_spaces_of_different_uses_or_none_is_not_evaluated() {
    // w1 adjoins an office and a corridor, whose limits differ; w2 adjoins
    // nothing; w3 cannot itself be measured.
    let model = rooms(&[("w1", &["o1", "k"]), ("w2", &[]), ("w3", &["o1"])]);
    let bottoms = floors().with("w1", 3.0, 0.0).with("w2", 3.0, 0.0);
    let evaluation = sill(model, bottoms, sill_keys(sill_limits()));
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("w1".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("w2".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("w3".to_owned(), NotEvaluatedReason::BackendUnavailable),
        ]
    );
}

#[test]
fn a_sill_below_a_minimum_is_found_and_without_geometry_nothing_is_judged() {
    let model = || rooms(&[("w1", &["o2"])]);
    let limits = vec![row([Some("Office"), None, None], Some(0.8), None)];
    let evaluation = sill(
        model(),
        floors().with("w1", 1.0, 0.0),
        sill_keys(limits.clone()),
    );
    assert_eq!(
        findings(&evaluation),
        [(
            "w1".into(),
            format!(
                "sill height above the floor of {} is 0.5 m; required at least 0.8 m \
                 (limit row 0: Pset.Use (via adjacent) `Office`)",
                id("o2")
            )
        )]
    );
    let evaluation = model().evaluate(&KeyedLimit, &rule(ID, kind("window"), sill_keys(limits)));
    assert_eq!(
        unevaluated(&evaluation),
        [("w1".to_owned(), NotEvaluatedReason::MissingService)]
    );
}

#[test]
fn a_sill_height_needs_its_floor_path_and_nothing_else_takes_one() {
    let model = || rooms(&[("w1", &["o1"])]);
    let mut missing = sill_keys(sill_limits());
    missing.retain(|(name, _)| *name != "floor_path");
    let mut stray = fire_keys(fire_limits());
    stray.push(("floor_path", strings(&["adjacent"])));
    let mut both = sill_keys(sill_limits());
    both.push(("quantity_property", property(None, "Sill")));
    for parameters in [missing, stray, both] {
        let evaluation = sill(model(), floors(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

fn metres(value: f64) -> ParameterValue {
    ParameterValue::Quantity {
        value,
        unit: "m".into(),
    }
}

fn length(value: f64) -> PropertyValue {
    PropertyValue::Quantity {
        value,
        dimension: QuantityDimension::Length,
    }
}

/// Single-swing doors need 0.9 m clear, double doors 1.2 m.
fn door_limits() -> Vec<TableRow> {
    vec![
        row([Some("SINGLE_SWING_*"), None, None], Some(0.9), None),
        row([Some("DOUBLE_DOOR_*"), None, None], Some(1.2), None),
    ]
}

/// A clear width stated in `Pset.ClearWidth`, else `OverallWidth` less
/// `deduction`, keyed by the door's `OperationType`.
fn door_keys(deduction: Option<f64>) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        ("limits", table(door_limits())),
        ("quantity", string("clear-width")),
        ("quantity_property", property(Some("Pset"), "ClearWidth")),
        ("key_1", property(Some("Attributes"), "OperationType")),
    ];
    if let Some(deduction) = deduction {
        parameters.push((
            "overall_width",
            property(Some("Attributes"), "OverallWidth"),
        ));
        parameters.push(("width_deduction", metres(deduction)));
    }
    parameters
}

/// Doors `(name, operation type, overall width, stated clear width)`.
type Door<'a> = (&'a str, &'a str, Option<f64>, Option<PropertyValue>);

fn doors(doors: &[Door<'_>]) -> Model {
    let mut model = Model::default();
    for (door, operation, overall, stated) in doors {
        model = model
            .object(door, "door")
            .text(door, "Attributes", "OperationType", operation);
        if let Some(overall) = overall {
            model = model.value(door, "Attributes", "OverallWidth", length(*overall));
        }
        if let Some(stated) = stated {
            model = model.value(door, "Pset", "ClearWidth", stated.clone());
        }
    }
    model
}

fn clear(model: Model, parameters: Vec<(&str, ParameterValue)>) -> CapabilityEvaluation {
    model.evaluate(&KeyedLimit, &rule(ID, kind("door"), parameters))
}

#[test]
fn a_door_too_narrow_after_the_deduction_is_found() {
    let model = doors(&[
        // 1 m less 0.1 m meets 0.9 m exactly, although the binary
        // difference falls a rounding step short of it.
        ("d1", "SINGLE_SWING_LEFT", Some(1.0), None),
        ("d2", "SINGLE_SWING_RIGHT", Some(0.9), None),
        // A stated clear width is used before any deduction.
        ("d3", "SINGLE_SWING_LEFT", Some(0.9), Some(length(0.95))),
        ("d4", "DOUBLE_DOOR_SINGLE_SWING", Some(1.25), None),
        ("d5", "SINGLE_SWING_LEFT", Some(2.0), Some(length(0.8))),
    ]);
    let evaluation = clear(model, door_keys(Some(0.1)));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "d2".into(),
                "clear width (Attributes.OverallWidth 0.9 m less the rule's deduction 0.1 m, \
                 an approximation) is 0.8 m; required at least 0.9 m (limit row 0: \
                 Attributes.OperationType `SINGLE_SWING_RIGHT`)"
                    .into()
            ),
            (
                "d4".into(),
                "clear width (Attributes.OverallWidth 1.25 m less the rule's deduction 0.1 m, \
                 an approximation) is 1.15 m; required at least 1.2 m (limit row 1: \
                 Attributes.OperationType `DOUBLE_DOOR_SINGLE_SWING`)"
                    .into()
            ),
            (
                "d5".into(),
                "clear width (Pset.ClearWidth) is 0.8 m; required at least 0.9 m (limit row 0: \
                 Attributes.OperationType `SINGLE_SWING_LEFT`)"
                    .into()
            ),
        ]
    );
    assert!(unevaluated(&evaluation).is_empty());
    // The derivation is recorded as the rule author's approximation.
    let step = |index: usize| {
        evaluation.findings()[index]
            .evidence
            .iter()
            .find(|evidence| evidence.locator.starts_with("axioval:derived.clear-width:"))
            .cloned()
            .unwrap()
    };
    let derived = step(0);
    assert!(
        derived
            .locator
            .ends_with("d2:step=overall-width-less-deduction;deduction=0.1"),
        "{}",
        derived.locator
    );
    assert!(!derived.exact);
    let stated = step(2);
    assert!(
        stated.locator.ends_with("d5:step=stated"),
        "{}",
        stated.locator
    );
    assert!(stated.exact);
}

#[test]
fn only_an_exactly_absent_clear_width_falls_back_to_the_deduction() {
    let model = doors(&[
        (
            "d1",
            "SINGLE_SWING_LEFT",
            Some(1.0),
            Some(PropertyValue::Null),
        ),
        (
            "d2",
            "SINGLE_SWING_LEFT",
            Some(1.0),
            Some(PropertyValue::String("wide".into())),
        ),
        ("d3", "SINGLE_SWING_LEFT", None, None),
        ("d4", "SINGLE_SWING_LEFT", Some(0.05), None),
        (
            "d5",
            "SINGLE_SWING_LEFT",
            Some(1.0),
            Some(PropertyValue::Decimal(0.8)),
        ),
    ]);
    let evaluation = clear(model, door_keys(Some(0.1)));
    assert!(findings(&evaluation).is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        ["d1", "d2", "d3", "d4", "d5"]
            .map(|door| (door.to_owned(), NotEvaluatedReason::IncompleteEvidence))
    );
    let message = |index: usize| evaluation.not_evaluated_outcomes()[index].message();
    for index in [0, 1, 4] {
        assert!(
            message(index).contains("not a positive length"),
            "{}",
            message(index)
        );
    }
    assert!(message(2).contains("are absent"), "{}", message(2));
    assert!(
        message(3).contains("leaves no clear width"),
        "{}",
        message(3)
    );
}

#[test]
fn without_a_deduction_only_a_stated_clear_width_is_judged() {
    let model = doors(&[
        ("d1", "SINGLE_SWING_LEFT", Some(0.8), None),
        ("d2", "SINGLE_SWING_LEFT", Some(0.8), Some(length(0.85))),
    ]);
    let evaluation = clear(model, door_keys(None));
    assert_eq!(
        findings(&evaluation),
        [(
            "d2".into(),
            "clear width (Pset.ClearWidth) is 0.85 m; required at least 0.9 m (limit row 0: \
             Attributes.OperationType `SINGLE_SWING_LEFT`)"
                .into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("d1".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // With a deduction and no stated property, the overall width alone counts.
    let mut parameters = door_keys(Some(0.1));
    parameters.retain(|(name, _)| *name != "quantity_property");
    let evaluation = clear(
        doors(&[("d1", "SINGLE_SWING_LEFT", Some(0.95), None)]),
        parameters,
    );
    assert_eq!(findings(&evaluation).len(), 1);
}

#[test]
fn a_clear_width_declaration_is_checked() {
    let model = || doors(&[("d1", "SINGLE_SWING_LEFT", Some(1.0), None)]);
    let without = |names: &[&str]| {
        let mut parameters = door_keys(Some(0.1));
        parameters.retain(|(name, _)| !names.contains(name));
        parameters
    };
    let with = |extra: (&'static str, ParameterValue)| {
        let mut parameters = door_keys(Some(0.1));
        parameters.retain(|(name, _)| *name != extra.0);
        parameters.push(extra);
        parameters
    };
    let mut plan_area = fire_keys(fire_limits());
    plan_area.push(("width_deduction", metres(0.1)));
    for parameters in [
        // Neither step.
        without(&["quantity_property", "overall_width", "width_deduction"]),
        // A deduction without the width it is taken from, and the reverse.
        without(&["overall_width"]),
        without(&["width_deduction"]),
        with(("width_deduction", metres(-0.1))),
        with((
            "width_deduction",
            ParameterValue::Quantity {
                value: 0.1,
                unit: "m2".into(),
            },
        )),
        with(("floor_path", strings(&["adjacent"]))),
        plan_area,
    ] {
        let evaluation = clear(model(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// With `clear_width_from_leaves`, a door's clear width is its overall width
/// less its lining on both jambs and its open leaves, as the leaves state
/// them; a door whose leaves slide or state no thickness moves on to the
/// deduction, and one whose leaves cannot be read is not evaluated.
#[test]
fn a_clear_width_is_derived_from_the_lining_and_leaves() {
    use common::doors::{Doors, hinged, sliding};
    let leaf = |width: f64| hinged([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], width, false);
    let model = doors(&[
        // 1 m less 2 × 0.05 m lining and a 0.04 m leaf: 0.86 m.
        ("d1", "SINGLE_SWING_LEFT", Some(1.0), None),
        // 1.1 m less 0.1 m and 0.04 m: 0.96 m.
        ("d2", "SINGLE_SWING_LEFT", Some(1.1), None),
        // Two leaves open: 1.3 m less 0.1 m and 2 × 0.04 m: 1.12 m.
        ("d3", "DOUBLE_DOOR_SINGLE_SWING", Some(1.3), None),
        // Sliding: the deduction applies, 1 m less 0.2 m.
        ("d4", "SINGLE_SWING_LEFT", Some(1.0), None),
        // No lining thickness: the deduction applies, 1.2 m less 0.2 m.
        ("d5", "SINGLE_SWING_LEFT", Some(1.2), None),
        ("d6", "SINGLE_SWING_LEFT", Some(1.2), None),
    ]);
    let frames = Doors::default()
        .door("d1", vec![leaf(0.9)], 1.0, Some(0.05))
        .door("d2", vec![leaf(1.0)], 1.1, Some(0.05))
        .door("d3", vec![leaf(0.6), leaf(0.6)], 1.3, Some(0.05))
        .door(
            "d4",
            vec![sliding([0.0; 3], [1.0, 0.0, 0.0], 1.0)],
            1.0,
            Some(0.05),
        )
        .door("d5", vec![leaf(1.1)], 1.2, None)
        .unknown(
            "d6",
            axioval_engine::DoorLeavesError::Refused("folding".into()),
        )
        .handle();
    let mut parameters = door_keys(Some(0.2));
    parameters.push(("clear_width_from_leaves", string("passage")));
    let evaluation = model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("door"), parameters),
        |services| {
            services.register(frames).unwrap();
        },
    );
    let found = findings(&evaluation);
    assert_eq!(
        found
            .iter()
            .map(|(door, _)| door.as_str())
            .collect::<Vec<_>>(),
        ["d1", "d3", "d4"],
        "{found:?}"
    );
    assert_eq!(
        found[0].1,
        "clear width (overall width 1 m less 2 × 0.05 m lining and 0.04 m of open leaf, as the \
         door states them) is 0.86 m; required at least 0.9 m (limit row 0: \
         Attributes.OperationType `SINGLE_SWING_LEFT`)"
    );
    assert!(
        found[1].1.contains("is 1.12 m; required at least 1.2 m"),
        "{}",
        found[1].1
    );
    assert!(
        found[2].1.contains("the rule's deduction 0.2 m"),
        "{}",
        found[2].1
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("d6".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let derived = evaluation.findings()[0]
        .evidence
        .iter()
        .find(|evidence| evidence.locator.starts_with("axioval:derived.clear-width:"))
        .unwrap();
    assert!(derived.locator.ends_with("d1:step=lining-and-leaves"));
    assert!(!derived.exact);
}

/// With `clear_width_from_leaves` `widest-leaf`, a door's clear width is its
/// widest leaf's: the leaf less the lining at the jamb it meets and its own
/// thickness, however wide the whole passage is.
#[test]
fn a_widest_leaf_too_narrow_is_found_although_the_passage_is_wide() {
    use common::doors::{Doors, hinged};
    // A 1.4 m double door: a 0.9 m leaf hinged at x 0 and a 0.5 m leaf
    // hinged at x 1.4, 0.05 m lining and 0.04 m leaves. Its widest leaf
    // gives 0.9 - 0.05 - 0.04 = 0.81 m; the passage 1.22 m.
    let model = || doors(&[("d1", "DOUBLE_DOOR_SINGLE_SWING", Some(1.4), None)]);
    let frames = || {
        Doors::default()
            .door(
                "d1",
                vec![
                    hinged([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.9, false),
                    hinged(
                        [1.4, 0.0, 0.0],
                        [-1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        0.5,
                        false,
                    ),
                ],
                1.4,
                Some(0.05),
            )
            .handle()
    };
    let limits = vec![row([Some("DOUBLE_DOOR_*"), None, None], Some(1.0), None)];
    let evaluate = |mode: &str| {
        let parameters = vec![
            ("limits", table(limits.clone())),
            ("quantity", string("clear-width")),
            ("key_1", property(Some("Attributes"), "OperationType")),
            ("clear_width_from_leaves", string(mode)),
        ];
        model().evaluate_with(
            &KeyedLimit,
            &rule(ID, kind("door"), parameters),
            |services| {
                services.register(frames()).unwrap();
            },
        )
    };
    let evaluation = evaluate("widest-leaf");
    assert_eq!(
        findings(&evaluation),
        [(
            "d1".into(),
            "clear width of the widest leaf (leaf 0.9 m less 0.05 m lining at one jamb and 0.04 \
             m of open leaf, as the door states them) is 0.81 m; required at least 1 m (limit \
             row 0: Attributes.OperationType `DOUBLE_DOOR_SINGLE_SWING`)"
                .into()
        )]
    );
    let derived = evaluation.findings()[0]
        .evidence
        .iter()
        .find(|evidence| evidence.locator.starts_with("axioval:derived.clear-width:"))
        .unwrap();
    assert!(derived.locator.ends_with("d1:step=widest-leaf"));
    assert!(!derived.exact);
    let evaluation = evaluate("passage");
    assert!(findings(&evaluation).is_empty());
    assert!(unevaluated(&evaluation).is_empty());
    let evaluation = evaluate("every-leaf");
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// Doors `(name, overall height, lining, threshold, stated clear height)`.
type TallDoor<'a> = (&'a str, f64, Option<f64>, Option<f64>, Option<f64>);

fn tall_doors(doors: &[TallDoor<'_>]) -> Model {
    let mut model = Model::default();
    for (door, overall, lining, threshold, stated) in doors {
        model = model
            .object(door, "door")
            .text(door, "Attributes", "OperationType", "SINGLE_SWING_LEFT")
            .value(door, "Attributes", "OverallHeight", length(*overall));
        for (name, value) in [
            ("LiningThickness", lining),
            ("ThresholdThickness", threshold),
            ("ClearHeight", stated),
        ] {
            if let Some(value) = value {
                model = model.value(door, "Lining", name, length(*value));
            }
        }
    }
    model
}

fn height_keys(stated: bool) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        (
            "limits",
            table(vec![row([Some("*"), None, None], Some(2.05), None)]),
        ),
        ("quantity", string("clear-height")),
        ("key_1", property(Some("Attributes"), "OperationType")),
        (
            "overall_height",
            property(Some("Attributes"), "OverallHeight"),
        ),
        (
            "lining_thickness",
            property(Some("Lining"), "LiningThickness"),
        ),
        (
            "threshold_thickness",
            property(Some("Lining"), "ThresholdThickness"),
        ),
    ];
    if stated {
        parameters.push(("quantity_property", property(Some("Lining"), "ClearHeight")));
    }
    parameters
}

/// A door's clear height is its overall height less its head lining and
/// threshold; an unstated thickness bounds it only from above, so only a
/// door too low even without it is found.
#[test]
fn a_door_too_low_after_its_lining_and_threshold_is_found() {
    let model = || {
        tall_doors(&[
            // 2.1 m less 0.05 m and 0.02 m: 2.03 m.
            ("d1", 2.1, Some(0.05), Some(0.02), None),
            // 2.2 m less 0.05 m and 0.02 m: 2.13 m.
            ("d2", 2.2, Some(0.05), Some(0.02), None),
            // No threshold stated: at most 2.15 m, at least nothing.
            ("d3", 2.2, Some(0.05), None, None),
            // At most 2.0 m even without a threshold.
            ("d4", 2.05, Some(0.05), None, None),
            // A stated clear height governs.
            ("d5", 2.1, Some(0.05), Some(0.02), Some(2.06)),
        ])
    };
    let evaluation = model().evaluate(&KeyedLimit, &rule(ID, kind("door"), height_keys(true)));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "d1".into(),
                "clear height (Attributes.OverallHeight 2.1 m less the lining 0.05 m less the \
                 threshold 0.02 m) is 2.03 m; required at least 2.05 m (limit row 0: \
                 Attributes.OperationType `SINGLE_SWING_LEFT`)"
                    .into()
            ),
            (
                "d4".into(),
                "clear height (Attributes.OverallHeight 2.05 m less the lining 0.05 m less a \
                 threshold Lining.ThresholdThickness does not state) is between 0 and 2 m; \
                 required at least 2.05 m (limit row 0: Attributes.OperationType \
                 `SINGLE_SWING_LEFT`)"
                    .into()
            ),
        ]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("d3".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let derived = evaluation.findings()[0]
        .evidence
        .iter()
        .find(|evidence| {
            evidence
                .locator
                .starts_with("axioval:derived.clear-height:")
        })
        .unwrap();
    assert!(!derived.exact);
    // Thicknesses need the overall height, and a width deduction applies to
    // a clear width only.
    let without_overall = {
        let mut parameters = height_keys(true);
        parameters.retain(|(name, _)| *name != "overall_height");
        parameters
    };
    let with_deduction = {
        let mut parameters = height_keys(false);
        parameters.push(("width_deduction", metres(0.1)));
        parameters
    };
    for parameters in [without_overall, with_deduction] {
        let evaluation = model().evaluate(&KeyedLimit, &rule(ID, kind("door"), parameters));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// Horizontal distances from doors to ramps and which ramps overlap which
/// spaces in plan; nothing has a box.
#[derive(Default)]
struct Ramps {
    distances: BTreeMap<(String, String), f64>,
    over: Vec<(String, String)>,
}

impl ProximityService for Ramps {
    fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_proximity(&self, _: &ProximityRequest) -> Result<ProximityEvidence, ProximityError> {
        Err(ProximityError::Unavailable)
    }

    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let pair = (
            request.subject().local_id.clone(),
            request.counterpart().local_id.clone(),
        );
        let distance = match request.projection() {
            ProximityProjection::Horizontal => self
                .distances
                .get(&pair)
                .copied()
                .ok_or(ProximityError::Unavailable)?,
            ProximityProjection::PlanOverlap => {
                if self.over.contains(&pair) {
                    0.0
                } else {
                    f64::INFINITY
                }
            }
            _ => return Err(ProximityError::UnsupportedProjection),
        };
        ProjectedDistanceEvidence::try_new(
            request.clone(),
            distance,
            distance,
            GeometryFidelity::Exact,
            Evidence::exact(source(), format!("distance:{}:{}", pair.0, pair.1)),
        )
    }
}

fn step_keys(ramps: bool) -> Vec<(&'static str, ParameterValue)> {
    let mut parameters = vec![
        (
            "limits",
            table(vec![row([Some("*"), None, None], None, Some(0.02))]),
        ),
        ("quantity", string("threshold-step")),
        ("floor_path", strings(&["adjacent"])),
        ("key_1", property(Some("Attributes"), "OperationType")),
    ];
    if ramps {
        parameters.push(("ramp_selector", common::selector(kind("ramp"))));
        parameters.push(("ramp_reach", metres(0.4)));
    }
    parameters
}

/// Doors between spaces, each `(door, spaces)`, and ramps.
fn stepped(doors: &[(&str, &[&str])], ramps: &[&str]) -> Model {
    let mut model = Model::default();
    for space in ["k", "o", "low"] {
        model = model.object(space, "space");
    }
    for (door, spaces) in doors {
        model = model.object(door, "door").text(
            door,
            "Attributes",
            "OperationType",
            "SINGLE_SWING_LEFT",
        );
        for space in *spaces {
            model = model.edge("adjacent", door, space);
        }
    }
    for ramp in ramps {
        model = model.object(ramp, "ramp");
    }
    model
}

fn step(
    model: Model,
    bottoms: Bottoms,
    ramps: Ramps,
    parameters: Vec<(&str, ParameterValue)>,
) -> CapabilityEvaluation {
    model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("door"), parameters),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(bottoms)))
                .unwrap();
            services
                .register(ProximityServiceHandle::new(Arc::new(ramps)))
                .unwrap();
        },
    )
}

/// A threshold step is measured from geometry: a sill 4 cm above the
/// corridor's floor fails a 2 cm maximum with no property stated.
#[test]
fn a_sill_above_the_corridor_floor_is_found_from_geometry() {
    // Corridor k and office o have their floors at 0 m. d1's bottom lies
    // 4 cm above both, d2's on them.
    let model = stepped(&[("d1", &["k", "o"]), ("d2", &["k", "o"])], &[]);
    let bottoms = Bottoms::default()
        .with("k", 0.0, 0.0)
        .with("o", 0.0, 0.0)
        .with("d1", 0.04, 0.0)
        .with("d2", 0.0, 0.0);
    let evaluation = step(model, bottoms, Ramps::default(), step_keys(false));
    assert_eq!(flagged(&evaluation), ["d1"]);
    assert!(unevaluated(&evaluation).is_empty());
    let message = &findings(&evaluation)[0].1;
    assert!(
        message.starts_with(&format!(
            "the step from the floor of {} to the door's bottom is 0.04 m; required at most \
             0.02 m",
            id("k")
        )),
        "{message}"
    );
    assert_eq!(
        evaluation.findings()[0]
            .related
            .iter()
            .map(|object| object.local_id.as_str())
            .collect::<Vec<_>>(),
        ["k", "o"]
    );
}

/// A stated threshold adds to the door's bottom; a declared one the door
/// does not state leaves only a step already too high decided.
#[test]
fn a_stated_threshold_adds_to_the_step_and_an_unstated_one_is_unknown() {
    let model = stepped(&[("d1", &["k"]), ("d2", &["k"]), ("d3", &["k"])], &[])
        .value("d1", "Lining", "ThresholdThickness", length(0.03))
        .value("d2", "Lining", "ThresholdThickness", length(0.01));
    let bottoms = Bottoms::default()
        .with("k", 0.0, 0.0)
        .with("d1", 0.0, 0.0)
        .with("d2", 0.0, 0.0)
        .with("d3", 0.0, 0.0);
    let mut parameters = step_keys(false);
    parameters.push((
        "threshold_thickness",
        property(Some("Lining"), "ThresholdThickness"),
    ));
    let evaluation = step(model, bottoms, Ramps::default(), parameters);
    assert_eq!(flagged(&evaluation), ["d1"]);
    assert!(
        findings(&evaluation)[0]
            .1
            .contains("to the door's bottom with its 0.03 m threshold is 0.03 m"),
        "{:?}",
        findings(&evaluation)
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("d3".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

/// A ramp within reach of the door and over a space is that side's floor,
/// measured at its top.
#[test]
fn a_ramp_near_the_door_is_the_floor_on_its_side() {
    // Space `low` has its floor at 0 m; d1 at 0.5 m opens onto it at the
    // top of ramp r1, 0.1 m away and over `low`; d2 does the same with no
    // ramp near it; d3's ramp r3 is 1 m away.
    let model = stepped(
        &[("d1", &["low"]), ("d2", &["low"]), ("d3", &["low"])],
        &["r1", "r3"],
    );
    let bottoms = Bottoms::default()
        .with("low", 0.0, 0.0)
        .with("d1", 0.5, 0.0)
        .with("d2", 0.5, 0.0)
        .with("d3", 0.5, 0.0)
        // Every body is 1 m tall, so the ramps' tops lie at 0.5 m.
        .with("r1", -0.5, 0.0)
        .with("r3", -0.5, 0.0);
    let mut ramps = Ramps::default();
    for (door, ramp, distance) in [
        ("d1", "r1", 0.1),
        ("d1", "r3", 5.0),
        ("d2", "r1", 5.0),
        ("d2", "r3", 5.0),
        ("d3", "r1", 5.0),
        ("d3", "r3", 1.0),
    ] {
        ramps.distances.insert((door.into(), ramp.into()), distance);
    }
    ramps.over = vec![("r1".into(), "low".into()), ("r3".into(), "low".into())];
    let evaluation = step(model, bottoms, ramps, step_keys(true));
    assert_eq!(flagged(&evaluation), ["d2", "d3"]);
    assert!(unevaluated(&evaluation).is_empty());
    // The rule declares its ramps and their reach together.
    let mut parameters = step_keys(true);
    parameters.retain(|(name, _)| *name != "ramp_reach");
    let evaluation = step(
        stepped(&[("d1", &["low"])], &[]),
        Bottoms::default(),
        Ramps::default(),
        parameters,
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
    );
}

/// Storeys `eg`, `og` and `dg`, named after themselves in capitals, each
/// containing the spaces listed.
fn named_storeys(storeys: &[(&str, &str, &[&str])]) -> Model {
    let mut model = Model::default();
    for (storey, name, spaces) in storeys {
        model = model
            .object(storey, "storey")
            .text(storey, "Pset", "Name", name);
        for space in *spaces {
            model = model.object(space, "space").edge("contains", storey, space);
        }
    }
    model
}

fn storey_area_limits() -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "limits",
            table(vec![
                row([Some("EG*"), None, None], Some(300.0), Some(400.0)),
                row([Some("OG*"), None, None], Some(250.0), Some(350.0)),
            ]),
        ),
        ("quantity", string("member-plan-area")),
        ("key_1", property(Some("Pset"), "Name")),
        ("member_selector", common::selector(kind("space"))),
        ("relationship", string("contains")),
    ]
}

fn per_storey(model: Model, areas: Areas) -> CapabilityEvaluation {
    model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("storey"), storey_area_limits()),
        |services| {
            services
                .register(PlanAreaServiceHandle::new(Arc::new(areas)))
                .unwrap();
        },
    )
}

#[test]
fn each_storey_sums_its_spaces_against_its_own_row() {
    let model = named_storeys(&[
        ("eg", "EG", &["e1", "e2"]),
        ("og", "OG1", &["o1", "o2"]),
        ("dg", "DG", &["d1"]),
    ]);
    let areas = Areas::default()
        .with("e1", 200.0, 0.0)
        .with("e2", 150.0, 0.0)
        .with("o1", 100.0, 0.0)
        .with("o2", 100.0, 0.0)
        .with("d1", 80.0, 0.0);
    let evaluation = per_storey(model, areas);
    assert_eq!(
        findings(&evaluation),
        [
            ("dg".into(), "no limit defined for Pset.Name `DG`".into()),
            (
                "og".into(),
                "summed plan area of the members via contains is 200 m²; required at least \
                 250 m² (limit row 1: Pset.Name `OG1`)"
                    .into()
            ),
        ]
    );
    assert_eq!(evaluation.findings()[1].related, [id("o1"), id("o2")]);
    assert!(evaluation.not_evaluated_outcomes().is_empty());
}

#[test]
fn a_storey_sum_straddling_its_row_is_not_evaluated() {
    let model = named_storeys(&[("eg", "EG", &["e1", "e2"])]);
    let areas = Areas::default()
        .with("e1", 200.0, 1.0)
        .with("e2", 100.0, 1.0);
    let evaluation = per_storey(model, areas);
    assert!(evaluation.findings().is_empty());
    assert_eq!(
        unevaluated(&evaluation),
        [("eg".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
}

#[test]
fn member_areas_need_a_member_selector_and_nothing_else_takes_one() {
    let mut without = storey_area_limits();
    without.retain(|(name, _)| *name != "member_selector");
    let mut other = storey_area_limits();
    other[1] = ("quantity", string("plan-area"));
    for parameters in [without, other] {
        let evaluation = named_storeys(&[("eg", "EG", &["e1"])])
            .evaluate(&KeyedLimit, &rule(ID, kind("storey"), parameters));
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    }
}

/// A selector picking doors whose `OperationType` is `operation`.
fn operated_as(operation: &str) -> ParameterValue {
    common::selector(axioval_ir::contract::Selector::property(
        Some("Attributes".into()),
        "OperationType",
        axioval_ir::contract::ComparisonOperator::Equals,
        Some(string(operation)),
    ))
}

/// One row of `door_type_defaults`, its cells as given.
fn defaults_row(cells: &[(&str, ParameterValue)]) -> TableRow {
    let mut row = TableRow::new();
    for (column, value) in cells {
        row.insert((*column).into(), value.clone());
    }
    row
}

/// Double-swing doors need 1.2 m clear, single-swing doors 0.9 m; the
/// overall width less a deduction where no clear width is stated.
fn typed_widths(defaults: Vec<TableRow>) -> Vec<(&'static str, ParameterValue)> {
    vec![
        (
            "limits",
            table(vec![
                row([Some("DOUBLE_SWING_*"), None, None], Some(1.2), None),
                row([Some("SINGLE_SWING_*"), None, None], Some(0.9), None),
            ]),
        ),
        ("quantity", string("clear-width")),
        ("quantity_property", property(Some("Pset"), "ClearWidth")),
        ("key_1", property(Some("Attributes"), "OperationType")),
        (
            "overall_width",
            property(Some("Attributes"), "OverallWidth"),
        ),
        ("width_deduction", metres(0.05)),
        ("door_type_defaults", table(defaults)),
    ]
}

/// The evidence entry recording the first default finding `index` used.
fn default_entry(evaluation: &CapabilityEvaluation, index: usize) -> Evidence {
    evaluation.findings()[index]
        .evidence
        .iter()
        .find(|evidence| evidence.locator.starts_with("axioval:default.door-type:"))
        .cloned()
        .unwrap()
}

/// A double-swing door without a stated clear width is judged by its overall
/// width less its type's deduction; a type row without one falls back to the
/// rule's deduction, a stated width wins and an unreadable one is never
/// replaced by a default.
#[test]
fn a_door_without_a_stated_clear_width_takes_its_types_deduction() {
    let model = doors(&[
        // 1.35 m less the type's 0.2 m: 1.15 m, short of 1.2 m.
        ("d1", "DOUBLE_SWING_LEFT", Some(1.35), None),
        // 1.45 m less 0.2 m: 1.25 m.
        ("d2", "DOUBLE_SWING_LEFT", Some(1.45), None),
        // The single-swing row gives no width deduction: 0.9 m less the
        // rule's 0.05 m is 0.85 m.
        ("d3", "SINGLE_SWING_LEFT", Some(0.9), None),
        // A stated clear width wins over every default.
        ("d4", "DOUBLE_SWING_LEFT", Some(1.0), Some(length(1.3))),
        // A stated width that is no length is not replaced by a default.
        (
            "d5",
            "DOUBLE_SWING_LEFT",
            Some(1.45),
            Some(PropertyValue::String("wide".into())),
        ),
    ]);
    let defaults = vec![
        defaults_row(&[
            ("applies_to", operated_as("DOUBLE_SWING_LEFT")),
            ("width_deduction", metres(0.2)),
        ]),
        defaults_row(&[
            ("applies_to", operated_as("SINGLE_SWING_LEFT")),
            ("threshold_height", metres(0.02)),
        ]),
    ];
    let evaluation = clear(model, typed_widths(defaults));
    assert_eq!(
        findings(&evaluation),
        [
            (
                "d1".into(),
                "clear width (Attributes.OverallWidth 1.35 m less the door type's default width \
                 deduction 0.2 m (door_type_defaults row 0), an approximation) is 1.15 m; \
                 required at least 1.2 m (limit row 0: Attributes.OperationType \
                 `DOUBLE_SWING_LEFT`)"
                    .into()
            ),
            (
                "d3".into(),
                "clear width (Attributes.OverallWidth 0.9 m less the rule's deduction 0.05 m, \
                 an approximation) is 0.85 m; required at least 0.9 m (limit row 1: \
                 Attributes.OperationType `SINGLE_SWING_LEFT`)"
                    .into()
            ),
        ]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("d5".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    // The default is cited as a rule parameter, never as a measurement.
    let used = default_entry(&evaluation, 0);
    assert!(
        used.locator.ends_with("d1:row=0;width_deduction=0.2"),
        "{}",
        used.locator
    );
    assert!(!used.exact);
    assert!(evaluation.findings()[0].evidence.iter().any(|evidence| {
        evidence
            .locator
            .ends_with("d1:step=overall-width-less-type-deduction;deduction=0.2")
            && !evidence.exact
    }));
    assert!(
        !evaluation.findings()[1]
            .evidence
            .iter()
            .any(|evidence| evidence.locator.starts_with("axioval:default.door-type:"))
    );
}

/// A row keyed on the operation the door's leaves state applies to doors
/// whose leaves say so; a door whose operation cannot be read has no
/// decidable type and is not evaluated, never given a later row's default.
#[test]
fn a_type_row_keyed_on_the_operation_needs_the_doors_leaves() {
    use common::doors::Doors;
    let model = doors(&[
        ("d1", "DOUBLE_SWING_LEFT", Some(1.35), None),
        ("d2", "DOUBLE_SWING_LEFT", Some(1.35), None),
    ]);
    let frames = Doors::default()
        .operated("d1", "DOUBLE_SWING_LEFT", 1.35)
        .unknown(
            "d2",
            axioval_engine::DoorLeavesError::NotStated("no panels".into()),
        )
        .handle();
    let defaults = vec![
        defaults_row(&[
            ("operation", string("DOUBLE_SWING_*")),
            ("width_deduction", metres(0.2)),
        ]),
        defaults_row(&[("width_deduction", metres(0.01))]),
    ];
    let evaluation = model.evaluate_with(
        &KeyedLimit,
        &rule(ID, kind("door"), typed_widths(defaults)),
        |services| {
            services.register(frames).unwrap();
        },
    );
    assert_eq!(flagged(&evaluation), ["d1"]);
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| evidence.locator == "leaves:d1"),
        "the operation is cited"
    );
    assert_eq!(
        unevaluated(&evaluation),
        [("d2".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    let message = evaluation.not_evaluated_outcomes()[0].message();
    assert!(
        message.contains("its type's default cannot be decided"),
        "{message}"
    );
}

/// A door that states no head lining or threshold takes its type's
/// defaults in its clear height; a stated one wins.
#[test]
fn a_clear_height_takes_the_types_default_lining_and_threshold() {
    let model = tall_doors(&[
        // 2.1 m less the defaults 0.05 m and 0.02 m: 2.03 m.
        ("d1", 2.1, None, None, None),
        // 2.2 m less the stated 0.05 m and the default 0.02 m: 2.13 m.
        ("d2", 2.2, Some(0.05), None, None),
        // 2.1 m less the stated 0.01 m and 0.01 m: 2.08 m.
        ("d3", 2.1, Some(0.01), Some(0.01), None),
    ]);
    let mut parameters = height_keys(false);
    parameters.push((
        "door_type_defaults",
        table(vec![defaults_row(&[
            ("applies_to", common::selector(kind("door"))),
            ("height_deduction", metres(0.05)),
            ("threshold_height", metres(0.02)),
        ])]),
    ));
    let evaluation = model.evaluate(&KeyedLimit, &rule(ID, kind("door"), parameters));
    assert_eq!(
        findings(&evaluation),
        [(
            "d1".into(),
            "clear height (Attributes.OverallHeight 2.1 m less the door type's default height \
             deduction 0.05 m (door_type_defaults row 0) less the door type's default threshold \
             0.02 m (door_type_defaults row 0)) is 2.03 m; required at least 2.05 m (limit row \
             0: Attributes.OperationType `SINGLE_SWING_LEFT`)"
                .into()
        )]
    );
    assert!(unevaluated(&evaluation).is_empty());
    let cited = evaluation.findings()[0]
        .evidence
        .iter()
        .filter(|evidence| evidence.locator.starts_with("axioval:default.door-type:"))
        .count();
    assert_eq!(cited, 2);
}

/// A door with no stated threshold takes its type's default in the
/// threshold step; a stated one wins.
#[test]
fn a_threshold_step_takes_the_types_default_threshold() {
    let model = stepped(&[("d1", &["k"]), ("d2", &["k"])], &[]).value(
        "d2",
        "Lining",
        "ThresholdThickness",
        length(0.01),
    );
    let bottoms = Bottoms::default()
        .with("k", 0.0, 0.0)
        .with("d1", 0.0, 0.0)
        .with("d2", 0.0, 0.0);
    let mut parameters = step_keys(false);
    parameters.push((
        "threshold_thickness",
        property(Some("Lining"), "ThresholdThickness"),
    ));
    parameters.push((
        "door_type_defaults",
        table(vec![defaults_row(&[("threshold_height", metres(0.03))])]),
    ));
    let evaluation = step(model, bottoms, Ramps::default(), parameters);
    assert_eq!(flagged(&evaluation), ["d1"]);
    assert!(unevaluated(&evaluation).is_empty());
    let message = &findings(&evaluation)[0].1;
    assert!(
        message.contains(
            "to the door's bottom with the door type's default threshold 0.03 m \
             (door_type_defaults row 0) is 0.03 m; required at most 0.02 m"
        ),
        "{message}"
    );
    assert!(!default_entry(&evaluation, 0).exact);
}

/// A glazing ratio is stated, else its type's default; a stated value that
/// is no ratio, or a door whose type gives none, is not evaluated.
#[test]
fn a_glazing_ratio_is_stated_or_its_types_default() {
    let mut model = Model::default()
        .value(
            "d1",
            "Pset",
            "GlazingAreaFraction",
            PropertyValue::Decimal(0.2),
        )
        .text("d3", "Pset", "GlazingAreaFraction", "half");
    for (door, operation) in [
        ("d1", "SINGLE_SWING_LEFT"),
        ("d2", "SINGLE_SWING_LEFT"),
        ("d3", "SINGLE_SWING_LEFT"),
        ("d4", "REVOLVING"),
    ] {
        model = model
            .object(door, "door")
            .text(door, "Attributes", "OperationType", operation);
    }
    let parameters = vec![
        (
            "limits",
            table(vec![row([Some("*"), None, None], Some(0.3), None)]),
        ),
        ("quantity", string("glazing-ratio")),
        (
            "quantity_property",
            property(Some("Pset"), "GlazingAreaFraction"),
        ),
        ("key_1", property(Some("Attributes"), "OperationType")),
        (
            "door_type_defaults",
            table(vec![defaults_row(&[
                ("applies_to", operated_as("SINGLE_SWING_LEFT")),
                ("glazing_ratio", number(0.5)),
            ])]),
        ),
    ];
    let evaluation = clear(model, parameters);
    assert_eq!(
        findings(&evaluation),
        [(
            "d1".into(),
            "glazing ratio (Pset.GlazingAreaFraction) is 0.2; required at least 0.3 (limit row \
             0: Attributes.OperationType `SINGLE_SWING_LEFT`)"
                .into()
        )]
    );
    assert_eq!(
        unevaluated(&evaluation),
        [
            ("d3".to_owned(), NotEvaluatedReason::IncompleteEvidence),
            ("d4".to_owned(), NotEvaluatedReason::IncompleteEvidence),
        ]
    );
}

#[test]
fn door_type_defaults_are_checked() {
    let model = || doors(&[("d1", "SINGLE_SWING_LEFT", Some(1.0), None)]);
    let invalid = |parameters: Vec<(&'static str, ParameterValue)>| {
        let evaluation = clear(model(), parameters);
        assert_eq!(
            unevaluated(&evaluation),
            [("-".to_owned(), NotEvaluatedReason::InvalidDeclaration)]
        );
    };
    // Defaults apply to door quantities only.
    let mut plan_area = fire_keys(fire_limits());
    plan_area.push(("door_type_defaults", table(Vec::new())));
    invalid(plan_area);
    // A ratio above one, and a deduction that is not a length.
    invalid(typed_widths(vec![defaults_row(&[(
        "glazing_ratio",
        number(1.5),
    )])]));
    invalid(typed_widths(vec![defaults_row(&[(
        "width_deduction",
        ParameterValue::Quantity {
            value: 1.0,
            unit: "m2".into(),
        },
    )])]));
    // The overall width needs a deduction: the rule's or the door type's.
    let mut bare = typed_widths(Vec::new());
    bare.retain(|(name, _)| !matches!(*name, "width_deduction" | "door_type_defaults"));
    invalid(bare);
    let mut typed_only = typed_widths(vec![defaults_row(&[("width_deduction", metres(0.1))])]);
    typed_only.retain(|(name, _)| *name != "width_deduction");
    let evaluation = clear(model(), typed_only);
    assert!(unevaluated(&evaluation).is_empty());
    assert!(findings(&evaluation).is_empty());
}

/// A clear width stated only as a measured interval is judged where every
/// width it may be agrees, and cited as inexact.
#[test]
fn a_clear_width_known_as_an_interval_decides_only_where_it_agrees() {
    let between = |lower: f64, upper: f64| PropertyValue::Measured {
        lower,
        upper,
        dimension: Some(QuantityDimension::Length),
    };
    let model = doors(&[
        ("d1", "SINGLE_SWING_LEFT", None, Some(between(0.7, 0.8))),
        ("d2", "SINGLE_SWING_LEFT", None, Some(between(0.85, 0.95))),
        ("d3", "SINGLE_SWING_LEFT", None, Some(between(0.95, 1.0))),
    ]);
    let evaluation = clear(model, door_keys(None));
    assert_eq!(flagged(&evaluation), ["d1"]);
    assert_eq!(
        unevaluated(&evaluation),
        [("d2".to_owned(), NotEvaluatedReason::IncompleteEvidence)]
    );
    assert!(
        evaluation.findings()[0]
            .evidence
            .iter()
            .any(|evidence| evidence.locator.ends_with("d1:step=stated") && !evidence.exact)
    );
}

#[test]
fn a_computed_limit_cell_judges_as_the_literal_it_computes() {
    let model = || {
        building(
            Some("1"),
            &[
                ("c1", "dry", "Office"),
                ("c2", "wet", "Office"),
                ("c3", "wet", "Office"),
                ("c4", "dry", "Office"),
            ],
        )
    };
    let areas = || {
        Areas::default()
            .with("c1", 500.0, 0.0)
            .with("c2", 500.0, 0.0)
            .with("c3", 900.0, 0.0)
            .with("c4", 400.0, 0.0)
    };
    let mut computed = fire_limits();
    computed[0].insert(
        "maximum".into(),
        common::expression(serde_json::json!({"kind": "multiply",
            "left": {"kind": "literal", "value": {"type": "number", "value": 200.0}},
            "right": {"kind": "literal", "value": {"type": "integer", "value": 2}}})),
    );
    let expected = run(model(), areas(), fire_keys(fire_limits()));
    let found = run(model(), areas(), fire_keys(computed));
    assert_eq!(found.findings(), expected.findings());
    assert!(!found.findings().is_empty());
    assert_eq!(
        found.not_evaluated_outcomes().len(),
        expected.not_evaluated_outcomes().len()
    );
}

/// `parameters` with the built-in quantity replaced by the measured value
/// `name`, the quantity's own parameters dropped.
fn as_measured(
    parameters: &[(&'static str, ParameterValue)],
    name: &str,
) -> Vec<(&'static str, ParameterValue)> {
    let own = [
        "quantity",
        "quantity_property",
        "overall_width",
        "width_deduction",
        "clear_width_from_leaves",
        "overall_height",
        "lining_thickness",
        "threshold_thickness",
        "floor_path",
    ];
    let mut measured: Vec<_> = parameters
        .iter()
        .filter(|(key, _)| !own.contains(key))
        .cloned()
        .collect();
    measured.push(("quantity", string("measured")));
    measured.push(("measured_value", string(name)));
    measured
}

/// Which objects an evaluation finds and leaves open.
fn outcome(evaluation: &CapabilityEvaluation) -> (Vec<String>, Vec<(String, NotEvaluatedReason)>) {
    (flagged(evaluation), unevaluated(evaluation))
}

/// Every built-in door and window quantity, read instead as its registered
/// measured value, judges every fixture alike.
#[test]
#[allow(clippy::too_many_lines)]
fn the_built_in_quantities_read_from_the_registry_judge_alike() {
    // Clear widths: stated, else the overall width less the deduction.
    let widths = || {
        doors(&[
            ("d1", "SINGLE_SWING_LEFT", Some(1.0), None),
            ("d2", "SINGLE_SWING_RIGHT", Some(0.9), None),
            ("d3", "SINGLE_SWING_LEFT", Some(0.9), Some(length(0.95))),
            ("d4", "DOUBLE_DOOR_SINGLE_SWING", Some(1.25), None),
            ("d5", "SINGLE_SWING_LEFT", Some(2.0), Some(length(0.8))),
            (
                "d6",
                "SINGLE_SWING_LEFT",
                Some(1.0),
                Some(PropertyValue::Null),
            ),
            ("d7", "SINGLE_SWING_LEFT", None, None),
        ])
    };
    let keys = door_keys(Some(0.1));
    let built_in = clear(widths(), keys.clone());
    let measured = widths().evaluate_measured(
        &KeyedLimit,
        &rule(
            ID,
            kind("door"),
            as_measured(
                &keys,
                "door_clear_width;stated=Pset/ClearWidth;overall=Attributes/OverallWidth;\
                 deduction=0.1",
            ),
        ),
        |_| {},
    );
    assert_eq!(outcome(&measured), outcome(&built_in), "clear width");
    // Clear heights: stated, else overall less lining and threshold.
    let heights = || {
        tall_doors(&[
            ("d1", 2.1, Some(0.05), Some(0.02), None),
            ("d2", 2.2, Some(0.05), Some(0.02), None),
            ("d3", 2.2, Some(0.05), None, None),
            ("d4", 2.05, Some(0.05), None, None),
            ("d5", 2.1, Some(0.05), Some(0.02), Some(2.06)),
        ])
    };
    let keys = height_keys(true);
    let built_in = heights().evaluate(&KeyedLimit, &rule(ID, kind("door"), keys.clone()));
    let measured = heights().evaluate_measured(
        &KeyedLimit,
        &rule(
            ID,
            kind("door"),
            as_measured(
                &keys,
                "door_clear_height;stated=Lining/ClearHeight;overall=Attributes/OverallHeight;\
                 lining=Lining/LiningThickness;threshold=Lining/ThresholdThickness",
            ),
        ),
        |_| {},
    );
    assert_eq!(outcome(&measured), outcome(&built_in), "clear height");
    // Sill heights above every reached floor, one row for all of them.
    let windows = || rooms(&[("w1", &["o1", "o2"]), ("w2", &["o1"]), ("w3", &["k"])]);
    let bottoms = || {
        floors()
            .with("w1", 1.2, 0.0)
            .with("w2", 0.9, 0.0)
            .with("w3", 2.0, 0.0)
            .with("w4", 1.0, 0.01)
    };
    let keys = sill_keys(sill_limits());
    let built_in = sill(windows(), bottoms(), keys.clone());
    let measured = windows().evaluate_measured(
        &KeyedLimit,
        &rule(
            ID,
            kind("window"),
            as_measured(&keys, "sill_height;floor_path=adjacent"),
        ),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(bottoms())))
                .unwrap();
        },
    );
    assert_eq!(outcome(&measured), outcome(&built_in), "sill height");
    // Threshold steps, stated or unknown thresholds, without ramps.
    let thresholds = || {
        stepped(&[("d1", &["k"]), ("d2", &["k"]), ("d3", &["k"])], &[])
            .value("d1", "Lining", "ThresholdThickness", length(0.03))
            .value("d2", "Lining", "ThresholdThickness", length(0.01))
    };
    let levels = || {
        Bottoms::default()
            .with("k", 0.0, 0.0)
            .with("d1", 0.0, 0.0)
            .with("d2", 0.0, 0.0)
            .with("d3", 0.0, 0.0)
    };
    let mut keys = step_keys(false);
    keys.push((
        "threshold_thickness",
        property(Some("Lining"), "ThresholdThickness"),
    ));
    let built_in = step(thresholds(), levels(), Ramps::default(), keys.clone());
    let measured = thresholds().evaluate_measured(
        &KeyedLimit,
        &rule(
            ID,
            kind("door"),
            as_measured(
                &keys,
                "threshold_step;floor_path=adjacent;threshold=Lining/ThresholdThickness",
            ),
        ),
        |services| {
            services
                .register(VerticalExtentServiceHandle::new(Arc::new(levels())))
                .unwrap();
        },
    );
    assert_eq!(outcome(&measured), outcome(&built_in), "threshold step");
}
