//! `axioval check --terrain`: terrain surfaces read as context beside the
//! models, end to end through the real binary.
#![allow(missing_docs)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axioval::ifc::IFC4_TYPE_SYSTEM;
use serde_json::{Value, json};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../fixtures/schema-v0.1.0"
);

/// One wall, no geometry: enough for a check to run.
const ONE_WALL: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
    #1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
    ENDSEC;\nEND-ISO-10303-21;\n";

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

    /// `check` of the fixture packages over one wall, with `extra`
    /// arguments, run in the case directory; the saved result, if any.
    fn check(&self, extra: &[&str]) -> (Output, Value) {
        self.write("model.ifc", ONE_WALL);
        let saved = self.path("result.json");
        let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(&self.dir)
            .arg("check")
            .args(["--model", "model.ifc"])
            .arg("--definitions")
            .arg(format!("{FIXTURES}/definitions.json"))
            .arg("--ruleset")
            .arg(format!("{FIXTURES}/ruleset.json"))
            .args(["--report", saved.to_str().unwrap()])
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

/// A `LandXML` TIN of `n` by `n` squares over `[x0, x0 + size]` by
/// `[y0, y0 + size]` in metres, rising `slope` per metre eastwards from
/// `base` at x 0. Points are written northing first, as `LandXML` states
/// them.
fn sloped_landxml(
    name: &str,
    (x0, y0, size): (f64, f64, f64),
    n: u32,
    (base, slope): (f64, f64),
) -> String {
    let mut points = String::new();
    for row in 0..=n {
        for column in 0..=n {
            let x = x0 + size * f64::from(column) / f64::from(n);
            let y = y0 + size * f64::from(row) / f64::from(n);
            let z = slope.mul_add(x, base);
            let _ = writeln!(
                points,
                "<P id=\"{}\">{y} {x} {z}</P>",
                row * (n + 1) + column
            );
        }
    }
    let mut faces = String::new();
    for row in 0..n {
        for column in 0..n {
            let at = |r: u32, c: u32| r * (n + 1) + c;
            let _ = writeln!(
                faces,
                "<F>{} {} {}</F>\n<F>{} {} {}</F>",
                at(row, column),
                at(row, column + 1),
                at(row + 1, column + 1),
                at(row, column),
                at(row + 1, column + 1),
                at(row + 1, column)
            );
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" version=\"1.2\">\n\
         <Units><Metric linearUnit=\"meter\" areaUnit=\"squareMeter\" volumeUnit=\"cubicMeter\"/></Units>\n\
         <Surfaces><Surface name=\"{name}\"><Definition surfType=\"TIN\">\n\
         <Pnts>\n{points}</Pnts>\n<Faces>\n{faces}</Faces>\n\
         </Definition></Surface></Surfaces>\n</LandXML>\n"
    )
}

/// engine#320: a synthetic TIN is read beside the model and recorded with
/// its digest, ground, accuracy and frame; the check itself is unchanged.
#[test]
fn a_terrain_file_is_read_and_recorded_with_its_accuracy() {
    let case = Case::new("terrain-read");
    let text = sloped_landxml("Ground", (-50.0, -50.0, 100.0), 4, (100.0, 0.1));
    case.write("ground.xml", &text);
    let (output, result) = case.check(&[
        "--terrain",
        "original@0.05=ground.xml",
        "--terrain-in",
        "model",
    ]);
    assert!(
        matches!(output.status.code(), Some(0 | 3 | 4)),
        "{}",
        stderr(&output)
    );
    let files = result["terrain_files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{result:#}");
    let file = &files[0];
    assert_eq!(file["terrain"], "original");
    assert_eq!(file["file"], "ground.xml");
    assert_eq!(file["surface"], "Ground");
    assert_eq!(file["frame"], "model");
    assert_eq!(file["accuracy_metres"], 0.05);
    assert_eq!(file["points"], 25);
    assert_eq!(file["triangles"], 32);
    assert_eq!(file["sha256"].as_str().unwrap().len(), 64);

    // Without a terrain the result has no record.
    let (_, result) = case.check(&[]);
    assert!(result.get("terrain_files").is_none(), "{result:#}");
}

/// engine#320: a terrain that cannot be read exactly stops the check
/// before anything runs, naming the argument; an accuracy and the frame
/// are required, never assumed.
#[test]
fn a_terrain_that_cannot_be_read_stops_the_check() {
    let case = Case::new("terrain-refused");
    let text = sloped_landxml("Ground", (0.0, 0.0, 10.0), 1, (0.0, 0.0));
    case.write("ground.xml", &text);
    case.write("feet.xml", &text.replace("\"meter\"", "\"furlong\""));

    let (output, result) = case.check(&[
        "--terrain",
        "original@0.05=feet.xml",
        "--terrain-in",
        "model",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("--terrain original=feet.xml"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("linear unit `furlong`"),
        "{}",
        stderr(&output)
    );
    assert!(result.is_null(), "nothing is written");

    let (output, _) = case.check(&[
        "--terrain",
        "original@0.05=ground.xml#Design",
        "--terrain-in",
        "model",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no surface `Design`"),
        "{}",
        stderr(&output)
    );

    // No accuracy, or no frame, is a usage error.
    for args in [
        &["--terrain", "original=ground.xml", "--terrain-in", "model"][..],
        &["--terrain", "original@0.05=ground.xml"][..],
    ] {
        let (output, _) = case.check(args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
    }
    let (output, _) = case.check(&[
        "--terrain",
        "original@0.05=ground.xml",
        "--terrain",
        "original@0.1=ground.xml",
        "--terrain-in",
        "model",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("a second original terrain"),
        "{}",
        stderr(&output)
    );
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

impl Case {
    /// `check --geometry` of one `expression` rule over the storeys of
    /// `models` (file name and IFC text), with `extra` arguments.
    fn storey_rule(
        &self,
        models: &[(&str, String)],
        requirement: Value,
        extra: &[&str],
    ) -> (Output, Value) {
        let text = std::fs::read_to_string(format!("{FIXTURES}/definitions.json")).unwrap();
        let mut definitions: Value = serde_json::from_str(&text).unwrap();
        for (id, name) in [("storey", "IfcBuildingStorey"), ("slab", "IfcSlab")] {
            definitions["objectTypes"][format!("axioval:example.ifc.{id}")] = json!({
                "id": format!("axioval:example.ifc.{id}"),
                "name": {"default": name, "translations": {}},
                "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
                "citations": [],
            });
        }
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
        rule["parameters"] = requirement;
        rule["applicability"]["groups"]["walls"]["selector"] = json!({
            "kind": "entityType", "objectType": "axioval:example.ifc.storey",
            "includeSubtypes": true});
        self.write("definitions.json", &definitions.to_string());
        self.write("ruleset.json", &ruleset.to_string());
        let saved = self.path("result.json");
        let _ = std::fs::remove_file(&saved);
        let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
        command.current_dir(&self.dir).arg("check");
        for (name, text) in models {
            self.write(name, text);
            command.arg("--model").arg(name);
        }
        let output = command
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

/// A building whose ground slab #100, 0.3 m thick, covers `[x0, x0 + 10]`
/// by `[0, 10]` in plan with its top at 100.5 m, and storeys #20, #30 and
/// #40 at 100.5, 103.5 and 106.5 m.
fn building(x0: f64) -> String {
    let storey = |id: u32, name: &str, z: f64| {
        format!(
            "#{a}=IFCCARTESIANPOINT((0.,0.,{z:?}));\n\
             #{b}=IFCAXIS2PLACEMENT3D(#{a},$,$);\n\
             #{c}=IFCLOCALPLACEMENT($,#{b});\n\
             #{id}=IFCBUILDINGSTOREY('{id:022}',$,'{name}',$,$,#{c},$,$,.ELEMENT.,{z:?});\n",
            a = id + 1,
            b = id + 2,
            c = id + 3,
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCUNITASSIGNMENT((#6));\n\
         #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
         {}{}{}\
         #101=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,10.,10.);\n\
         #102=IFCCARTESIANPOINT(({:?},5.,100.2));\n\
         #103=IFCAXIS2PLACEMENT3D(#102,$,$);\n\
         #104=IFCEXTRUDEDAREASOLID(#101,#103,#4,0.3);\n\
         #105=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#104));\n\
         #106=IFCPRODUCTDEFINITIONSHAPE($,$,(#105));\n\
         #100=IFCSLAB('{:022}',$,$,$,$,#3,#106,$,.FLOOR.);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        storey(20, "Ground", 100.5),
        storey(30, "First", 103.5),
        storey(40, "Second", 106.5),
        x0 + 5.0,
        100,
    )
}

/// The `[lower, upper]` of a finding message `height L..U m` or `height V m`.
fn bounds_of(message: &str) -> (f64, f64) {
    let text = message.trim_start_matches("height ").trim_end_matches(" m");
    let bounds: Vec<f64> = text
        .split("..")
        .map(|bound| bound.trim().parse::<f64>().unwrap())
        .collect();
    (bounds[0], *bounds.last().unwrap())
}

fn below(left: &Value) -> Value {
    json!({"requirement": {"type": "expression", "value": {
            "kind": "compare", "operator": "lessThanOrEquals", "left": left,
            "right": {"kind": "literal",
                      "value": {"type": "quantity", "value": -1.0, "unit": "m"}}}},
        "message": {"type": "string", "value": "height {height}"}})
}

/// engine#325: on a terrain rising 10 % eastwards, stated to 0.05 m, the
/// highest storey of a building whose 10 m slab runs from x 0 to x 10
/// stands 6 m above the mean terrain along the slab's outline (100.5 m),
/// within the accuracy; the storeys of a building beyond the terrain are
/// not evaluated, naming the point outside it.
#[test]
fn the_highest_floor_level_is_measured_above_the_mean_terrain() {
    let case = Case::new("terrain-above");
    let text = sloped_landxml("Ground", (-50.0, -50.0, 100.0), 5, (100.0, 0.1));
    case.write("ground.xml", &text);
    let height = json!({"kind": "property", "propertySet": "axioval:measured",
                        "property": "above_terrain;terrain=original;of=level;outline=IfcSlab",
                        "label": "height"});
    let models = [("near.ifc", building(0.0)), ("far.ifc", building(1000.0))];
    let terrain = [
        "--terrain",
        "original@0.05=ground.xml",
        "--terrain-in",
        "model",
    ];
    // Every level stands above -1 m, so each is reported with its height.
    let (output, result) = case.storey_rule(&models, below(&height), &terrain);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    let mut heights: Vec<(String, (f64, f64))> = findings
        .iter()
        .map(|finding| {
            assert_eq!(
                finding["object_id"]["source"]["document"], "near.ifc",
                "{finding:#}"
            );
            (
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                bounds_of(finding["message"].as_str().unwrap()),
            )
        })
        .collect();
    heights.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(heights.len(), 3, "{result:#}");
    for ((storey, (lower, upper)), expected) in heights.iter().zip([0.0, 3.0, 6.0]) {
        assert!(
            *lower <= expected - 0.05 && *upper >= expected + 0.05,
            "{storey}: {lower}..{upper} holds {expected} ± 0.05"
        );
        assert!(
            upper - lower < 0.1 + 1e-6,
            "{storey}: {lower}..{upper} is the accuracy wide"
        );
    }
    // The highest floor level, #40, is 6 m above the mean terrain.
    assert_eq!(heights[2].0, "#40");

    // The far building's storeys lie beyond the terrain.
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(open.len(), 3, "{result:#}");
    for outcome in open {
        assert_eq!(
            outcome["object_id"]["source"]["document"], "far.ifc",
            "{outcome:#}"
        );
        let why = outcome["message"].as_str().unwrap();
        assert!(
            why.contains("lies outside the original terrain surface `Ground`"),
            "{why}"
        );
    }

    // Without a finished terrain, a rule reading it is not evaluated and
    // the check warns that none was supplied.
    let finished = json!({"kind": "property", "propertySet": "axioval:measured",
                          "property": "above_terrain;terrain=finished;of=level;outline=IfcSlab",
                          "label": "height"});
    let (output, result) = case.storey_rule(&models[..1], below(&finished), &terrain);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let warned: Vec<&Value> = result["integrity"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|record| record["code"] == "terrain-not-supplied")
        .collect();
    assert_eq!(warned.len(), 1, "{result:#}");
    assert_eq!(warned[0]["locator"], "axioval:terrain;kind=finished");
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(!open.is_empty(), "{result:#}");
    assert!(
        open.iter().all(|outcome| outcome["message"]
            .as_str()
            .unwrap()
            .contains("no finished terrain is supplied")),
        "{result:#}"
    );
}

/// [`building`] placed far out in its own model coordinates: the slab's
/// and every storey's placement moved by `(east, north)`.
fn building_far_out(x0: f64, (east, north): (f64, f64)) -> String {
    let mut text = building(x0).replace(
        "#3=IFCLOCALPLACEMENT($,#2);",
        &format!(
            "#10=IFCCARTESIANPOINT(({east:?},{north:?},0.));\n\
             #9=IFCAXIS2PLACEMENT3D(#10,$,$);\n\
             #3=IFCLOCALPLACEMENT($,#9);"
        ),
    );
    for z in ["100.5", "103.5", "106.5"] {
        text = text.replace(
            &format!("IFCCARTESIANPOINT((0.,0.,{z}))"),
            &format!("IFCCARTESIANPOINT(({east:?},{north:?},{z}))"),
        );
    }
    text
}

/// engine#372: the building and the terrain moved together 5600 km out in
/// the model's coordinates stand as at the origin: the bodies are measured
/// about a local origin and the slab's outline is placed back on the
/// terrain, so each storey stands as high above it (0, 3 and 6 m within
/// the accuracy) and the far building's storeys are not evaluated alike.
#[test]
fn a_georeferenced_building_stands_above_the_terrain_as_at_its_origin() {
    let case = Case::new("terrain-georeferenced");
    let (east, north) = (600_000.0, 5_600_000.0);
    let height = json!({"kind": "property", "propertySet": "axioval:measured",
                        "property": "above_terrain;terrain=original;of=level;outline=IfcSlab",
                        "label": "height"});
    let terrain = [
        "--terrain",
        "original@0.05=ground.xml",
        "--terrain-in",
        "model",
    ];
    let run = |moved: bool| {
        let (dx, dy) = if moved { (east, north) } else { (0.0, 0.0) };
        // Rising 0.1 per metre eastwards, 100 m high at the building's x 0.
        let text = sloped_landxml(
            "Ground",
            (dx - 50.0, dy - 50.0, 100.0),
            5,
            ((-0.1_f64).mul_add(dx, 100.0), 0.1),
        );
        case.write("ground.xml", &text);
        let models = [
            ("near.ifc", building_far_out(0.0, (dx, dy))),
            ("far.ifc", building_far_out(1000.0, (dx, dy))),
        ];
        case.storey_rule(&models, below(&height), &terrain)
    };
    let (at_origin, expected) = run(false);
    let (output, result) = run(true);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(output.status.code(), at_origin.status.code());
    assert_eq!(
        result["geometry"]["local_origin"],
        json!([east, north]),
        "{:#}",
        result["geometry"]
    );
    let heights = |result: &Value| {
        let mut found: Vec<(String, (f64, f64))> = result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| {
                (
                    finding["object_id"]["local_id"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    bounds_of(finding["message"].as_str().unwrap()),
                )
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    };
    let (found, before) = (heights(&result), heights(&expected));
    assert_eq!(found.len(), 3, "{result:#}");
    for (((storey, (lower, upper)), (_, (low, high))), level) in
        found.iter().zip(&before).zip([0.0, 3.0, 6.0])
    {
        assert!(
            *lower <= level - 0.05 && *upper >= level + 0.05,
            "{storey}: {lower}..{upper} holds {level} ± 0.05"
        );
        assert!(
            (lower - low).abs() < 1e-5 && (upper - high).abs() < 1e-5,
            "{storey}: {lower}..{upper} as at the origin, {low}..{high}"
        );
    }
    let open = |result: &Value| result["report"]["not_evaluated"].as_array().unwrap().len();
    assert_eq!(open(&result), 3, "{result:#}");
    assert_eq!(open(&result), open(&expected));
}

/// [`building`] georeferenced: its x 0, y 0, z 0 at easting 500 000 m,
/// northing 5 600 000 m and height 50 m of EPSG:25832, axes unturned.
fn building_on_map(x0: f64) -> String {
    building(x0).replace(
        "#8=IFCPROJECT(",
        "#90=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#6);\n\
         #91=IFCMAPCONVERSION(#5,#90,500000.,5600000.,50.,$,$,$);\n\
         #8=IFCPROJECT(",
    )
}

/// engine#325: a terrain in map coordinates is placed through each
/// model's map conversion: the same ground as before, given in EPSG:25832
/// with heights 50 m higher, measures the same heights, and the ground
/// beneath each storey's origin (the plan origin, at 100 m) too. A model
/// stating no map conversion, or a terrain naming another reference
/// system, is not evaluated, never placed by a guess.
#[test]
fn a_terrain_on_the_map_is_placed_through_the_map_conversion() {
    let case = Case::new("terrain-map");
    // z = 150 + 0.1 (E - 500 000) over E 499 950..500 050, N 5 599 950..
    let text = sloped_landxml(
        "Ground",
        (499_950.0, 5_599_950.0, 100.0),
        5,
        (-49_850.0, 0.1),
    )
    .replace(
        "<Surfaces>",
        "<CoordinateSystem epsgCode=\"25832\"/><Surfaces>",
    );
    case.write("map.xml", &text);
    case.write("other.xml", &text.replace("25832", "25833"));
    let terrain = |file: &str| {
        [
            "--terrain".to_owned(),
            format!("original@0.05={file}"),
            "--terrain-in".to_owned(),
            "map".to_owned(),
        ]
    };
    let read = |property: &str| {
        json!({"kind": "property", "propertySet": "axioval:measured",
               "property": property, "label": "height"})
    };
    let heights = |result: &Value| {
        let mut found: Vec<(String, (f64, f64))> = result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| {
                (
                    finding["object_id"]["local_id"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    bounds_of(finding["message"].as_str().unwrap()),
                )
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    };
    let args = terrain("map.xml");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let models = [("near.ifc", building_on_map(0.0))];

    let above_mean = read("above_terrain;terrain=original;of=level;outline=IfcSlab");
    let (output, result) = case.storey_rule(&models, below(&above_mean), &args);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = heights(&result);
    assert_eq!(found.len(), 3, "{result:#}");
    for ((storey, (lower, upper)), expected) in found.iter().zip([0.0, 3.0, 6.0]) {
        assert!(
            *lower <= expected - 0.05 && *upper >= expected + 0.05 && upper - lower < 0.1 + 1e-5,
            "{storey}: {lower}..{upper} holds {expected} ± 0.05"
        );
    }

    let above_beneath = read("above_terrain;terrain=original;of=point;reference=beneath");
    let (output, result) = case.storey_rule(&models, below(&above_beneath), &args);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    for ((storey, (lower, upper)), expected) in heights(&result).iter().zip([0.5, 3.5, 6.5]) {
        assert!(
            *lower <= expected - 0.05 && *upper >= expected + 0.05 && upper - lower < 0.1 + 1e-5,
            "{storey}: {lower}..{upper} holds {expected} ± 0.05"
        );
    }

    // A model stating no map conversion cannot be placed on the map.
    let plain = [("plain.ifc", building(0.0))];
    let (output, result) = case.storey_rule(&plain, below(&above_mean), &args);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(open.len(), 3, "{result:#}");
    let why = open[0]["message"].as_str().unwrap();
    assert!(why.contains("states no map conversion"), "{why}");

    // Nor a terrain naming another reference system than the model's.
    let args = terrain("other.xml");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (output, result) = case.storey_rule(&models, below(&above_mean), &args);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    let why = open[0]["message"].as_str().unwrap();
    assert!(
        why.contains("names `EPSG:25833`, and the model's map conversion is to `EPSG:25832`"),
        "{why}"
    );
}

/// engine#325: the mean terrain height along the slab's outline is
/// 100.5 m on the terrain rising 10 % eastwards, within its accuracy.
#[test]
fn the_mean_terrain_height_follows_the_outline() {
    let case = Case::new("terrain-mean");
    let text = sloped_landxml("Ground", (-50.0, -50.0, 100.0), 5, (100.0, 0.1));
    case.write("ground.xml", &text);
    let mean = json!({"kind": "property", "propertySet": "axioval:measured",
                      "property": "mean_terrain_height;terrain=original;outline=IfcSlab",
                      "label": "height"});
    let (output, result) = case.storey_rule(
        &[("near.ifc", building(0.0))],
        below(&mean),
        &[
            "--terrain",
            "original@0.05=ground.xml",
            "--terrain-in",
            "model",
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 3, "{result:#}");
    for finding in findings {
        let (lower, upper) = bounds_of(finding["message"].as_str().unwrap());
        assert!(
            lower <= 100.45 && upper >= 100.55 && upper - lower < 0.1 + 1e-6,
            "{lower}..{upper}"
        );
    }
}
