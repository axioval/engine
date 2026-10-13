//! `axioval check` of external data read as stated values, end to end
//! through the real binary: results of external calculations joined to
//! walls by their `GlobalId` (#323).
#![allow(missing_docs)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axioval::ifc::IFC4_TYPE_SYSTEM;
use serde_json::{Value, json};
use sha2::Digest as _;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../fixtures/schema-v0.1.0"
);

struct Case {
    dir: PathBuf,
}

/// One `expression` rule over the objects of an entity type.
struct Rule {
    /// The IFC entity the rule selects.
    entity: &'static str,
    /// The requirement.
    requirement: Value,
    /// Authored parameters the definition declares, with their values.
    parameters: Vec<(Value, Value)>,
    /// Fields of the ruleset (`catalogues`).
    ruleset: Value,
}

impl Case {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// `check --geometry` of `rule` over `model.ifc`, with `extra`
    /// arguments; the saved result, if any.
    fn check(&self, rule: &Rule, extra: &[&str]) -> (Output, Value) {
        let text = std::fs::read_to_string(format!("{FIXTURES}/definitions.json")).unwrap();
        let mut definitions: Value = serde_json::from_str(&text).unwrap();
        definitions["objectTypes"]["axioval:example.ifc.under-test"] = json!({
            "id": "axioval:example.ifc.under-test",
            "name": {"default": rule.entity, "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": rule.entity}],
            "citations": [],
        });
        for (id, name) in [("global-id", "GlobalId"), ("object-type", "ObjectType")] {
            definitions["properties"][format!("axioval:example.ifc.{id}")] = json!({
                "id": format!("axioval:example.ifc.{id}"),
                "name": {"default": name, "translations": {}},
                "valueKind": "string",
                "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
                "citations": [],
            });
        }
        let capability = "axioval:capability.expression";
        let mut signature = registry_signature(capability);
        for (declared, _) in &rule.parameters {
            signature[declared["id"].as_str().unwrap()] = declared.clone();
        }
        definitions["definitions"]["axioval:example.under-test"] = json!({
            "id": "axioval:example.under-test",
            "name": {"default": "Under test", "translations": {}},
            "description": {"default": "The capability under test.", "translations": {}},
            "capability": capability,
            "parameters": signature,
            "citations": [],
            "tags": [],
        });
        let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let under_test = &mut ruleset["root"]["rules"][0];
        under_test["id"] = json!("under-test");
        under_test["definitionId"] = json!("axioval:example.under-test");
        under_test["parameters"] = json!({
            "requirement": {"type": "expression", "value": rule.requirement},
        });
        for (declared, value) in &rule.parameters {
            under_test["parameters"][declared["id"].as_str().unwrap()] = value.clone();
        }
        under_test["applicability"]["groups"]["walls"]["selector"] = json!({
            "kind": "entityType", "objectType": "axioval:example.ifc.under-test",
            "includeSubtypes": true});
        for (field, value) in rule.ruleset.as_object().into_iter().flatten() {
            ruleset[field] = value.clone();
        }
        self.write("definitions.json", &definitions.to_string());
        self.write("ruleset.json", &ruleset.to_string());
        let saved = self.path("result.json");
        let _ = std::fs::remove_file(&saved);
        let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(&self.dir)
            .arg("check")
            .args(["--model", "model.ifc"])
            .args(["--definitions", "definitions.json"])
            .args(["--ruleset", "ruleset.json"])
            .args(["--geometry", "--report", saved.to_str().unwrap()])
            .args(extra)
            .output()
            .unwrap();
        let result = std::fs::read_to_string(&saved)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        (output, result)
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn text(value: &str) -> Value {
    json!({"default": value, "translations": {}})
}

/// A capability's full parameter signature as the registry declares it.
fn registry_signature(capability: &str) -> Value {
    let registry = axioval::default_registry().unwrap();
    registry
        .get(capability)
        .unwrap()
        .parameters()
        .into_iter()
        .map(|parameter| {
            let id = parameter.name.clone();
            let declared = json!({"id": id, "name": {"default": id, "translations": {}},
                       "kind": parameter.parameter_type.package_kind(),
                       "required": parameter.required, "allowedValues": [], "citations": []});
            (id, declared)
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// A box `size` (x, y, height) centred at `centre` in plan, standing on
/// the ground, as product `#first + 9` of entity text `product`.
fn placed_box(
    first: u32,
    [x, y]: [f64; 2],
    [length, width, height]: [f64; 3],
    product: &str,
) -> String {
    let [
        origin,
        frame,
        placement,
        p,
        pos,
        profile,
        solid,
        shape,
        definition,
        object,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map(|offset| first + offset);
    format!(
        "#{origin}=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
         #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
         #{p}=IFCCARTESIANPOINT(({x:.2},{y:.2}));\n\
         #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
         #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length:.2},{width:.2});\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{height:.2});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        product
            .replace("GID", &format!("{object:022}"))
            .replace("PL", &format!("#{placement}"))
            .replace("REP", &format!("#{definition}"))
    )
}

/// An IFC4 model in metres holding `products`, its context stating the
/// map conversion `georeference` (entity text after the units), if any.
fn model(products: &str, georeference: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,#8);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCUNITASSIGNMENT((#6));\n\
         #8=IFCDIRECTION((0.,1.));\n\
         #9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,(#5),#7);\n\
         {georeference}{products}\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Four partitions, 4 m by 0.2 m by 3 m: #19 and #29 between apartments,
/// #39 enclosing a stair, #49 between apartments.
fn partitions() -> String {
    let wall = |used: &str| format!("IFCWALL('GID',$,'W',$,'{used}',PL,REP,$,.STANDARD.)");
    [
        placed_box(
            10,
            [2.0, 0.0],
            [4.0, 0.2, 3.0],
            &wall("apartment-separating"),
        ),
        placed_box(
            20,
            [2.0, 5.0],
            [4.0, 0.2, 3.0],
            &wall("apartment-separating"),
        ),
        placed_box(30, [2.0, 10.0], [4.0, 0.2, 3.0], &wall("stair-enclosure")),
        placed_box(
            40,
            [2.0, 15.0],
            [4.0, 0.2, 3.0],
            &wall("apartment-separating"),
        ),
    ]
    .concat()
}

/// R'w of each partition at least the requirement table's for its use,
/// the results joined to the walls by their `GlobalId`.
fn sound_rule() -> Rule {
    let attribute = |id: &str| {
        json!({"kind": "property", "propertySet": "axioval:attributes",
               "property": format!("axioval:example.ifc.{id}")})
    };
    let required = json!({
    "id": "required", "name": text("required R'w"), "kind": "table", "required": true,
    "allowedValues": [], "citations": [],
    "columns": [
        {"id": "use", "name": text("use"), "kind": "textPattern", "required": false},
        {"id": "rw", "name": text("R'w"), "kind": "number", "required": false},
    ]});
    let row = |pattern: &str, rw: f64| json!({"use": {"type": "string", "value": pattern}, "rw": {"type": "number", "value": rw}});
    Rule {
        entity: "IfcWall",
        requirement: json!({"kind": "compare", "operator": "greaterThanOrEquals",
            "left": {"kind": "catalogue", "catalogue": "sound", "key": attribute("global-id"),
                     "column": "rw", "label": "R'w"},
            "right": {"kind": "lookup", "table": "required", "column": "rw",
                      "keys": {"use": attribute("object-type")}}}),
        parameters: vec![(
            required,
            json!({"type": "table",
                   "value": [row("apartment-separating", 53.0), row("*", 47.0)]}),
        )],
        ruleset: json!({"catalogues": {"sound": {
            "id": "sound",
            "name": text("Sound insulation results"),
            "key": "wall",
            "columns": [
                {"id": "wall", "header": "GlobalId", "kind": "string"},
                {"id": "rw", "header": "R'w (dB)", "kind": "number"},
                {"id": "method", "kind": "string"},
                {"id": "report", "kind": "string"},
                {"id": "date", "kind": "date"},
                {"id": "revision", "header": "model revision", "kind": "string"},
            ],
            "results": {"method": "method", "report": "report", "date": "date",
                        "revision": "revision"},
        }}}),
    }
}

/// engine#323 end to end: a results file of an invented calculation tool
/// states R'w per wall, keyed by `GlobalId`, each with its method, report,
/// date and the model revision it was computed on. Joined to the walls
/// and compared with a requirement table by use, #29 (51 dB between
/// apartments) is found; #19 and #39 meet theirs; #49's result was
/// computed on an earlier revision and is reported stale, never read.
#[test]
fn calculation_results_joined_to_walls_are_compared_and_a_stale_one_is_reported() {
    let case = Case::new("external-calculation-results");
    let model = model(&partitions(), "");
    case.write("model.ifc", &model);
    let digest = sha256_hex(model.as_bytes());
    case.write(
        "rw.csv",
        &format!(
            "GlobalId,R'w (dB),method,report,date,model revision\n\
             0000000000000000000019,54,ex-sound-method 2018,ex-acoustics report 12,2026-09-30,sha256:{digest}\n\
             0000000000000000000029,51,ex-sound-method 2018,ex-acoustics report 12,2026-09-30,sha256:{digest}\n\
             0000000000000000000039,48,ex-sound-method 2018,ex-acoustics report 12,2026-09-30,rev-12\n\
             0000000000000000000049,58,ex-sound-method 2018,ex-acoustics report 11,2026-08-01,rev-11\n"
        ),
    );
    let rule = sound_rule();
    let (output, result) = case.check(
        &rule,
        &[
            "--catalogue",
            "sound=rw.csv",
            "--model-revision",
            "model.ifc=rev-12",
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#29", "{result:#}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(
        message.contains("catalogue sound[0000000000000000000029].rw = 51"),
        "{message}"
    );
    let cited = findings[0]["evidence"].to_string();
    assert!(
        cited.contains(&format!(
            "#entry=0000000000000000000029;method=ex-sound-method 2018;\
             report=ex-acoustics report 12;date=2026-09-30;revision=sha256:{digest}"
        )),
        "{cited}"
    );
    // #49's result was computed on revision rev-11: stale, not evaluated.
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(open.len(), 1, "{result:#}");
    assert_eq!(open[0]["object_id"]["local_id"], "#49", "{result:#}");
    let reason = open[0]["message"].as_str().unwrap();
    assert!(
        reason.contains("is stale") && reason.contains("`rev-11`"),
        "{reason}"
    );
    let integrity = result["integrity"].as_array().unwrap();
    let stale: Vec<&Value> = integrity
        .iter()
        .filter(|record| record["code"] == "calculation-result-stale")
        .collect();
    assert_eq!(stale.len(), 1, "{integrity:#?}");
    assert_eq!(
        stale[0]["locator"],
        "axioval:catalogue;id=sound#entry=0000000000000000000049"
    );
    assert!(
        stale[0]["message"]
            .as_str()
            .unwrap()
            .contains(&format!("sha256:{digest} (revision `rev-12`)")),
        "{:#}",
        stale[0]
    );

    // Without the stated revision, #39's result (rev-12) is stale too.
    let (output, result) = case.check(&rule, &["--catalogue", "sound=rw.csv"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let open: Vec<&Value> = result["report"]["not_evaluated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| &outcome["object_id"]["local_id"])
        .collect();
    assert_eq!(open, [&json!("#39"), &json!("#49")], "{result:#}");

    // A revision naming no checked model stops the check.
    let (output, _) = case.check(
        &rule,
        &[
            "--catalogue",
            "sound=rw.csv",
            "--model-revision",
            "a=1",
            "--model-revision",
            "b=2",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
}

/// A synthetic snow load zone map in EPSG:25832, two bands along the
/// easting, each stating its zone and characteristic load on the ground.
const SNOW_MAP: &str = r#"{"type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "EPSG:25832"}},
  "accuracy": 0.5,
  "features": [
    {"type": "Feature", "id": "SL-1", "properties": {"kind": "snow-load-zone",
      "values": {"load_zone": "1", "characteristic_load": {"value": 0.65, "unit": "kN/m²"}}},
     "geometry": {"type": "Polygon", "coordinates": [[[500000, 5600000], [500100, 5600000], [500100, 5600100], [500000, 5600100], [500000, 5600000]]]}},
    {"type": "Feature", "id": "SL-2a", "properties": {"kind": "snow-load-zone",
      "values": {"load_zone": "2a", "characteristic_load": {"value": 1.06, "unit": "kN/m²"}}},
     "geometry": {"type": "Polygon", "coordinates": [[[500100, 5600000], [500200, 5600000], [500200, 5600100], [500100, 5600100], [500100, 5600000]]]}}
  ]}"#;

/// Buildings as proxies 3 m high: #19 at easting 20 (zone 1), #29 at
/// easting 150 (zone 2a) and #39 across the zones' boundary at 100.
fn buildings() -> String {
    let proxy = "IFCBUILDINGELEMENTPROXY('GID',$,'B',$,$,PL,REP,$,$)";
    [
        placed_box(10, [20.0, 20.0], [12.0, 10.0, 3.0], proxy),
        placed_box(20, [150.0, 20.0], [12.0, 10.0, 3.0], proxy),
        placed_box(30, [100.0, 60.0], [12.0, 10.0, 3.0], proxy),
    ]
    .concat()
}

/// The design load for the zone each building stands in, keyed by the
/// zone the map states there, at most 1.0 kN/m².
fn snow_rule() -> Rule {
    let loads = json!({
    "id": "loads", "name": text("snow loads"), "kind": "table", "required": true,
    "allowedValues": [], "citations": [],
    "columns": [
        {"id": "zone", "name": text("zone"), "kind": "textPattern", "required": false},
        {"id": "load", "name": text("load"), "kind": "number", "required": false},
    ]});
    let row = |zone: &str, load: f64| json!({"zone": {"type": "string", "value": zone}, "load": {"type": "number", "value": load}});
    Rule {
        entity: "IfcBuildingElementProxy",
        requirement: json!({"kind": "compare", "operator": "lessThanOrEquals",
            "left": {"kind": "lookup", "table": "loads", "column": "load",
                     "keys": {"zone": {"kind": "zone", "standsIn": "snow-load-zone",
                                       "value": "load_zone", "label": "snow load zone"}}},
            "right": {"kind": "literal", "value": {"type": "number", "value": 1.0}}}),
        parameters: vec![(
            loads,
            json!({"type": "table", "value": [row("1", 0.65), row("2a", 1.06), row("3", 1.10)]}),
        )],
        ruleset: json!({}),
    }
}

/// engine#324 end to end: a synthetic snow load zone map in EPSG:25832,
/// supplied with `--zones`, gives the zone at each building's position in
/// the georeferenced model, used as a `lookup` key: #29 stands in zone 2a,
/// whose 1.06 kN/m² exceed the rule's 1.0, #19 in zone 1, and #39, across
/// the zones' boundary, in neither surely: not evaluated, never given one.
#[test]
fn a_snow_load_zone_map_gives_the_zone_at_each_buildings_position() {
    let case = Case::new("external-snow-load-zones");
    let georeference = "#90=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#6);\n\
                        #91=IFCMAPCONVERSION(#5,#90,500000.,5600000.,0.,$,$,$);\n";
    case.write("model.ifc", &model(&buildings(), georeference));
    case.write("snow.geojson", SNOW_MAP);
    let rule = snow_rule();
    let (output, result) = case.check(&rule, &["--zones", "snow.geojson"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#29", "{result:#}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(message.contains("zone SL-2a.load_zone = `2a`"), "{message}");
    let cited = findings[0]["evidence"].to_string();
    assert!(
        cited.contains("axioval:zone;stands_in=snow-load-zone;id=SL-2a")
            && cited.contains(";value=load_zone;stated=properties.values.load_zone"),
        "{cited}"
    );
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(open.len(), 1, "{result:#}");
    assert_eq!(open[0]["object_id"]["local_id"], "#39", "{result:#}");
    assert!(
        open[0]["message"].as_str().unwrap().contains("undecided"),
        "{result:#}"
    );

    // Without the map every building is open, and the result says why.
    let (output, result) = case.check(&rule, &[]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        result["integrity"]
            .to_string()
            .contains("zones-not-supplied"),
        "{result:#}"
    );
}
