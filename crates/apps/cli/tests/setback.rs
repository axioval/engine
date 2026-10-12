//! `axioval check` of setback areas (#326) end to end through the real
//! binary: synthetic buildings of four walls around a room, a parcel
//! supplied with `--zones` and a level terrain with `--terrain`.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axioval::ifc::IFC4_TYPE_SYSTEM;
use serde_json::{Value, json};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../fixtures/schema-v0.1.0"
);

struct Case {
    dir: PathBuf,
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

    /// `check --geometry` of one `expression` rule over every wall of
    /// `model`, with `extra` arguments; the saved result, if any.
    fn wall_rule(&self, model: &str, requirement: &Value, extra: &[&str]) -> (Output, Value) {
        let text = std::fs::read_to_string(format!("{FIXTURES}/definitions.json")).unwrap();
        let mut definitions: Value = serde_json::from_str(&text).unwrap();
        definitions["objectTypes"]["axioval:example.ifc.wall-under-test"] = json!({
            "id": "axioval:example.ifc.wall-under-test",
            "name": {"default": "IfcWall", "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcWall"}],
            "citations": [],
        });
        let capability = "axioval:capability.expression";
        definitions["definitions"]["axioval:example.under-test"] = json!({
            "id": "axioval:example.under-test",
            "name": {"default": "Under test", "translations": {}},
            "description": {"default": "The capability under test.", "translations": {}},
            "capability": capability,
            "parameters": registry_signature(capability),
            "citations": [],
            "tags": [],
        });
        let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["id"] = json!("under-test");
        rule["definitionId"] = json!("axioval:example.under-test");
        rule["parameters"] = json!({
            "requirement": {"type": "expression", "value": requirement},
            "message": {"type": "string", "value": "{value}"},
        });
        rule["applicability"]["groups"]["walls"]["selector"] = json!({
            "kind": "entityType", "objectType": "axioval:example.ifc.wall-under-test",
            "includeSubtypes": true});
        self.write("definitions.json", &definitions.to_string());
        self.write("ruleset.json", &ruleset.to_string());
        self.write("model.ifc", model);
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

/// A building of four 6 m walls, 0.3 m thick, around a room, its outline
/// `[x0, x0 + 10]` by `[y0, y0 + 10]`, numbered from `first`: the south
/// wall `#first + 9`, then north, west and east, then the room.
fn building(first: u32, [x0, y0]: [f64; 2]) -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let room = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    [
        placed_box(first, [x0 + 5.0, y0 + 0.15], [10.0, 0.3, 6.0], wall),
        placed_box(first + 10, [x0 + 5.0, y0 + 9.85], [10.0, 0.3, 6.0], wall),
        placed_box(first + 20, [x0 + 0.15, y0 + 5.0], [0.3, 9.4, 6.0], wall),
        placed_box(first + 30, [x0 + 9.85, y0 + 5.0], [0.3, 9.4, 6.0], wall),
        placed_box(first + 40, [x0 + 5.0, y0 + 5.0], [9.4, 9.4, 6.0], room),
    ]
    .concat()
}

/// A model of the buildings whose outlines start at `corners`.
fn model(corners: &[[f64; 2]]) -> String {
    let buildings: String = corners
        .iter()
        .zip([10, 60])
        .map(|(corner, first)| building(first, *corner))
        .collect();
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
         {buildings}\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

/// A level `LandXML` TIN at height 0 over x and y -50..50, in metres.
const GROUND: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
    <LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" version=\"1.2\">\n\
    <Units><Metric linearUnit=\"meter\" areaUnit=\"squareMeter\" volumeUnit=\"cubicMeter\"/></Units>\n\
    <Surfaces><Surface name=\"Ground\"><Definition surfType=\"TIN\">\n\
    <Pnts>\n<P id=\"1\">-50 -50 0</P>\n<P id=\"2\">-50 50 0</P>\n<P id=\"3\">50 50 0</P>\n<P id=\"4\">50 -50 0</P>\n</Pnts>\n\
    <Faces>\n<F>1 2 3</F>\n<F>1 3 4</F>\n</Faces>\n\
    </Definition></Surface></Surfaces>\n</LandXML>\n";

/// Parcel `P1`, x -10..20 and y -5..20: 5 m in front of the south wall
/// of the building at the origin, farther from its other walls.
const PARCEL: &str = r#"{"type": "FeatureCollection",
  "crs": {"type": "name", "properties": {"name": "axioval:model"}},
  "accuracy": 0,
  "features": [
    {"type": "Feature", "id": "P1", "properties": {"kind": "parcel"},
     "geometry": {"type": "Polygon", "coordinates": [[[-10, -5], [20, -5], [20, 20], [-10, 20], [-10, -5]]]}}
  ]}"#;

impl Case {
    /// The ground and the parcel as `check` arguments.
    fn context(&self) -> Vec<String> {
        let ground = self.write("ground.xml", GROUND);
        let zones = self.write("zones.geojson", PARCEL);
        vec![
            "--terrain".to_owned(),
            format!("original@0.05={}", ground.display()),
            "--terrain-in".to_owned(),
            "model".to_owned(),
            "--zones".to_owned(),
            zones.display().to_string(),
        ]
    }
}

fn measured(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "axioval:measured", "property": name,
           "label": "value"})
}

fn at_most(name: &str, square_metres: f64) -> Value {
    json!({"kind": "compare", "operator": "lessThanOrEquals", "left": measured(name),
           "right": {"kind": "literal",
                     "value": {"type": "quantity", "value": square_metres, "unit": "m2"}}})
}

/// The `[lower, upper]` of a finding message `L..U <unit>` or `V <unit>`.
fn bounds_of(message: &str) -> (f64, f64) {
    let text = message.split_whitespace().next().unwrap_or_default();
    let bounds: Vec<f64> = text
        .split("..")
        .map(|bound| bound.trim().parse::<f64>().unwrap())
        .collect();
    (bounds[0], *bounds.last().unwrap())
}

fn findings(result: &Value) -> Vec<(String, (f64, f64), Vec<String>)> {
    let mut found: Vec<_> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            // What the finding cites: its related objects and evidence.
            let mut related: Vec<String> = finding["related"]
                .as_array()
                .map(|related| {
                    related
                        .iter()
                        .filter_map(|object| object["local_id"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            related.extend(
                finding["evidence"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|evidence| evidence["locator"].as_str().map(str::to_owned)),
            );
            (
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                bounds_of(finding["message"].as_str().unwrap()),
                related,
            )
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// engine#326: every wall of a building 6 m high above a level terrain
/// known to 5 cm has its setback measured, 6 ± 0.05 m deep: the south and
/// north walls' across their 10 m faces and their 0.3 m ends at the
/// corners (63.6 m²), the west and east walls' across the 9.4 m between
/// them (56.4 m²). A rule allowing 70 m² passes them all; one allowing
/// 50 m² finds each with its area.
#[test]
fn every_wall_of_a_building_has_its_setback_measured() {
    let case = Case::new("setback-measured");
    let context = case.context();
    let args: Vec<&str> = context.iter().map(String::as_str).collect();
    let rule = at_most("setback_area;terrain=original;factor=1", 70.0);
    let (output, result) = case.wall_rule(&model(&[[0.0, 0.0]]), &rule, &args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}: {result:#}",
        stderr(&output)
    );
    let rule = at_most("setback_area;terrain=original;factor=1", 50.0);
    let (output, result) = case.wall_rule(&model(&[[0.0, 0.0]]), &rule, &args);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = findings(&result);
    let walls: Vec<&str> = found.iter().map(|(wall, _, _)| wall.as_str()).collect();
    assert_eq!(walls, ["#19", "#29", "#39", "#49"], "{result:#}");
    for ((wall, (lower, upper), _), (expected, along)) in
        found
            .iter()
            .zip([(63.6, 10.6), (63.6, 10.6), (56.4, 9.4), (56.4, 9.4)])
    {
        assert!(
            *lower <= expected && *upper >= expected && upper - lower <= 0.1 * along + 1e-6,
            "{wall}: {lower}..{upper} holds {expected}"
        );
    }
}

/// engine#326: the parcel's edge lies 5 m in front of the south wall #19,
/// whose setback reaches 6 ± 0.05 m: a finding names the 10 ± 0.5 m²
/// beyond it. The other walls' setbacks lie on the parcel. Without the
/// terrain nothing is evaluated and the check warns.
#[test]
fn a_setback_crossing_the_parcel_is_found_with_the_area_beyond_it() {
    let case = Case::new("setback-parcel");
    let context = case.context();
    let args: Vec<&str> = context.iter().map(String::as_str).collect();
    let rule = at_most(
        "setback_outside_area;terrain=original;factor=1;zone=P1",
        0.0,
    );
    let (output, result) = case.wall_rule(&model(&[[0.0, 0.0]]), &rule, &args);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}: {result:#}",
        stderr(&output)
    );
    let found = findings(&result);
    assert_eq!(found.len(), 1, "{result:#}");
    let (wall, (lower, upper), _) = &found[0];
    assert_eq!(wall, "#19");
    assert!(
        *lower <= 10.0 && *upper >= 10.0 && *lower >= 9.5 - 1e-6 && *upper <= 10.5 + 1e-6,
        "{lower}..{upper}"
    );
    // 0.4 H, at least 3 m, keeps to the parcel.
    let rule = at_most(
        "setback_outside_area;terrain=original;factor=0.4;minimum=3;zone=P1",
        0.0,
    );
    let (output, result) = case.wall_rule(&model(&[[0.0, 0.0]]), &rule, &args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}: {result:#}",
        stderr(&output)
    );

    // Without the terrain the height is unknown: not evaluated, warned.
    let zones = &args[4..];
    let rule = at_most(
        "setback_outside_area;terrain=original;factor=1;zone=P1",
        0.0,
    );
    let (output, result) = case.wall_rule(&model(&[[0.0, 0.0]]), &rule, zones);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let integrity = result["integrity"].to_string();
    assert!(integrity.contains("terrain-not-supplied"), "{integrity}");
}

/// engine#326: a second building 8 m south of the first faces it; at
/// 1 H (6 ± 0.05 m) their setbacks overlap 4 m deep over x 4..10, so the
/// first's south wall #19 and the second's north wall #79 are each found
/// with 24 m², relating the other. At 0.4 H they keep apart.
#[test]
fn overlapping_setbacks_of_two_buildings_are_reported() {
    let case = Case::new("setback-overlap");
    let context = case.context();
    let args: Vec<&str> = context.iter().map(String::as_str).collect();
    let two = model(&[[0.0, 0.0], [4.0, -18.0]]);
    let rule = at_most(
        "setback_overlap_area;terrain=original;factor=1;others=IfcWall",
        0.0,
    );
    let (output, result) = case.wall_rule(&two, &rule, &args);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}: {result:#}",
        stderr(&output)
    );
    let found = findings(&result);
    let walls: Vec<&str> = found.iter().map(|(wall, _, _)| wall.as_str()).collect();
    assert_eq!(walls, ["#19", "#79"], "{result:#}");
    for ((wall, (lower, upper), related), other) in found.iter().zip(["#79", "#19"]) {
        assert!(
            *lower <= 24.0 && *upper >= 24.0 && upper - lower <= 1.2 + 1e-6,
            "{wall}: {lower}..{upper}"
        );
        // The finding names the wall whose setback overlaps.
        assert!(
            related
                .iter()
                .any(|cited| cited.contains(&format!("overlapped-by=ifc-step:model.ifc/{other}"))),
            "{wall}: {related:?}"
        );
    }
    let rule = at_most(
        "setback_overlap_area;terrain=original;factor=0.4;others=IfcWall",
        0.0,
    );
    let (output, result) = case.wall_rule(&two, &rule, &args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}: {result:#}",
        stderr(&output)
    );
}
