//! `axioval check` end to end: the real binary over real packages and IFC.
//!
//! Exit status and output shape are automation contracts, so every case
//! asserts the status first.
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

/// Walls #1 and #2 (a subtype) and a slab; only #1 has `Reference`.
/// The `GlobalId` of `#2` is varied per test.
fn ifc(wall_2_global_id: &str, wall_2_has_reference: bool) -> String {
    let related = if wall_2_has_reference {
        "(#1,#2)"
    } else {
        "(#1)"
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
         #2=IFCWALLSTANDARDCASE('{wall_2_global_id}',$,$,$,$,$,$,$,$);\n\
         #3=IFCSLAB('0000000000000000000003',$,$,$,$,$,$,$,$);\n\
         #4=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W-1'),$);\n\
         #5=IFCPROPERTYSET('0000000000000000000005',$,'Pset_WallCommon',$,(#4));\n\
         #6=IFCRELDEFINESBYPROPERTIES('0000000000000000000006',$,$,$,{related},#5);\n\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

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

    /// The MCS minimal example, optionally bound to IFC4 names as a package
    /// author targeting IFC4 would.
    fn definitions(&self, bound: bool) -> PathBuf {
        let text = std::fs::read_to_string(format!("{FIXTURES}/definitions.json")).unwrap();
        let mut definitions: Value = serde_json::from_str(&text).unwrap();
        if bound {
            for (section, id, name) in [
                ("objectTypes", "axioval:example.ifc.wall", "IfcWall"),
                ("properties", "axioval:example.ifc.reference", "Reference"),
                (
                    "propertySets",
                    "axioval:example.ifc.pset-wall-common",
                    "Pset_WallCommon",
                ),
            ] {
                definitions[section][id]["externalNames"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"typeSystem": IFC4_TYPE_SYSTEM, "name": name}));
            }
        }
        self.write("definitions.json", &definitions.to_string())
    }

    fn check(&self, model: &str, bound: bool, extra: &[&str]) -> Output {
        let model = self.write("model.ifc", model);
        self.check_file(&model, bound, extra)
    }

    /// Checks the model file at `model` as it is.
    fn check_file(&self, model: &Path, bound: bool, extra: &[&str]) -> Output {
        let definitions = self.definitions(bound);
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .arg("check")
            .arg("--model")
            .arg(model)
            .arg("--definitions")
            .arg(definitions)
            .arg("--ruleset")
            .arg(format!("{FIXTURES}/ruleset.json"))
            .args(extra)
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap()
    }
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_finding_exits_3_and_is_written_as_json_and_bcf() {
    let case = Case::new("finding");
    let bcf = case.path("issues.bcfzip");
    let output = case.check(
        &ifc("0000000000000000000002", false),
        true,
        &["--bcf", bcf.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#2");
    assert_eq!(findings[0]["object_id"]["source"]["document"], "model.ifc");
    assert!(result["integrity"].as_array().unwrap().is_empty());

    let archive = openbim_bcf::read_path(&bcf).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );
    let topic = &archive.topics().next().unwrap().topic;
    assert_eq!(topic.creation_date.as_deref(), Some("2026-09-26T10:00:00Z"));
    assert_eq!(topic.creation_author.as_deref(), Some("axioval"));
    // The rule id, then the rule's tag from the ruleset.
    assert_eq!(topic.labels, ["wall-reference-required", "example"]);
    assert!(stderr(&output).contains("1 finding(s), 0 not evaluated"));
}

/// The HTML report: one self-contained file through the built-in template,
/// sections reordered by a custom one, and a template hiding the
/// not-evaluated outcomes refused before anything is written.
#[test]
fn a_finding_is_written_as_an_html_report_from_a_template() {
    let case = Case::new("html-report");
    let model = ifc("0000000000000000000002", false);
    let page = case.path("report.html");
    let output = case.check(
        &model,
        true,
        &[
            "--html",
            page.to_str().unwrap(),
            "--html-title",
            "Walls <A>",
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let html = std::fs::read_to_string(&page).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
    assert!(html.contains("<title>Walls &lt;A&gt;</title>"), "{html}");
    assert!(html.contains("2026-09-26T10:00:00Z"), "{html}");
    assert!(html.contains("Not passed: 1 finding(s)"), "{html}");
    assert!(
        html.contains("<code>wall-reference-required</code>"),
        "{html}"
    );
    assert!(
        html.contains("<code>0000000000000000000002</code>"),
        "{html}"
    );
    for external in ["http://", "https://", "<script", "<link"] {
        assert!(!html.contains(external), "{external}: {html}");
    }
    assert!(
        html.find("id=\"summary\"").unwrap() < html.find("id=\"tables\"").unwrap(),
        "{html}"
    );
    // Rendered again, the same bytes.
    case.check(
        &model,
        true,
        &[
            "--html",
            page.to_str().unwrap(),
            "--html-title",
            "Walls <A>",
        ],
    );
    assert_eq!(std::fs::read_to_string(&page).unwrap(), html);

    let template = case.write(
        "template.html",
        "<html><body>{{tables}}{{findings}}{{not-evaluated}}{{summary}}</body></html>",
    );
    let custom = case.path("custom.html");
    let output = case.check(
        &model,
        true,
        &[
            "--html",
            custom.to_str().unwrap(),
            "--html-template",
            template.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let html = std::fs::read_to_string(&custom).unwrap();
    assert!(
        html.find("id=\"findings\"").unwrap() < html.find("id=\"summary\"").unwrap(),
        "{html}"
    );
    assert!(!html.contains("id=\"cover\""), "{html}");

    let hiding = case.write("hiding.html", "<html><body>{{summary}}</body></html>");
    let refused = case.path("refused.html");
    let saved = case.path("refused.json");
    let output = case.check(
        &model,
        true,
        &[
            "--html",
            refused.to_str().unwrap(),
            "--html-template",
            hiding.to_str().unwrap(),
            "--report",
            saved.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("not-evaluated"),
        "{}",
        stderr(&output)
    );
    assert!(!refused.exists() && !saved.exists());
}

#[test]
fn a_complete_clean_check_exits_0() {
    let case = Case::new("clean");
    let output = case.check(&ifc("0000000000000000000002", true), true, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let result = json(&output);
    assert!(result["report"]["findings"].as_array().unwrap().is_empty());
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_incomplete_check_never_exits_0() {
    // The published example names its concepts in IFC4.3 only, so nothing
    // binds to an IFC4 model: no finding, and not a pass either.
    let case = Case::new("incomplete");
    let output = case.check(&ifc("0000000000000000000002", false), false, &[]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let result = json(&output);
    assert!(result["report"]["findings"].as_array().unwrap().is_empty());
    assert!(
        !result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// A model with no objects: a presentation layer holding one point.
const EMPTY_MODEL: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
     #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
     #2=IFCPRESENTATIONLAYERWITHSTYLE('Foo',$,(#1),$,.T.,.F.,.F.,());\n\
     ENDSEC;\nEND-ISO-10303-21;\n";

impl Case {
    /// The fixture packages with the rule swapped for "the model contains
    /// at least one wall".
    fn wall_count_packages(&self) -> (PathBuf, PathBuf) {
        let definitions = self.definitions(true);
        let mut definitions: Value =
            serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
        definitions["definitions"]["axioval:example.object-count"] = json!({
            "id": "axioval:example.object-count",
            "name": {"default": "Object count", "translations": {}},
            "description": {"default": "The model contains the selection.", "translations": {}},
            "capability": "axioval:capability.object-count",
            "parameters": registry_signature("axioval:capability.object-count"),
            "citations": [],
            "tags": [],
        });
        let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["id"] = json!("a-wall-exists");
        rule["definitionId"] = json!("axioval:example.object-count");
        rule["parameters"] = json!({});
        (
            self.write("definitions.json", &definitions.to_string()),
            self.write("ruleset.json", &ruleset.to_string()),
        )
    }
}

#[test]
fn a_model_without_objects_is_checked_and_an_existence_rule_fails_on_it() {
    let case = Case::new("empty-model");

    // An object rule has nothing to check in it: a complete, clean check.
    let output = case.check(EMPTY_MODEL, true, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let result = json(&output);
    assert!(result["report"]["findings"].as_array().unwrap().is_empty());
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // "The model contains a wall" is reported against the empty source,
    // never passed because no object was there to be counted.
    let model = case.write("model.ifc", EMPTY_MODEL);
    let (definitions, ruleset) = case.wall_count_packages();
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["rule_id"], "a-wall-exists", "{result:#}");
    assert_eq!(
        findings[0]["message"],
        "no object matches the selection in source `ifc-step:model.ifc`; required at least 1"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_type_object_is_checked_against_its_own_property_sets() {
    // The example's wall concept bound to IfcWallType: the rule checks the
    // type objects themselves, each against its own HasPropertySets. #10
    // states the reference, #11 does not; the occurrence #20 inherits #10's
    // set but is no IfcWallType, so it is not selected.
    let case = Case::new("type-objects");
    let model = case.write(
        "model.ifc",
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('WT-1'),$);\n\
         #2=IFCPROPERTYSET('0000000000000000000002',$,'Pset_WallCommon',$,(#1));\n\
         #10=IFCWALLTYPE('0000000000000000000010',$,'A',$,$,(#2),$,$,$,.STANDARD.);\n\
         #11=IFCWALLTYPE('0000000000000000000011',$,'B',$,$,$,$,$,$,.STANDARD.);\n\
         #20=IFCWALL('0000000000000000000020',$,$,$,$,$,$,$,$);\n\
         #21=IFCRELDEFINESBYTYPE('0000000000000000000021',$,$,$,(#20),#10);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
    );
    let text = std::fs::read_to_string(case.definitions(true)).unwrap();
    let mut definitions: Value = serde_json::from_str(&text).unwrap();
    definitions["objectTypes"]["axioval:example.ifc.wall"]["externalNames"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["name"] = json!("IfcWallType");
    let definitions = case.write("definitions.json", &definitions.to_string());
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(format!("{FIXTURES}/ruleset.json"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#11", "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{result:#}"
    );
}

/// A wall, two materials and an `IfcRelConnectsPathElements`, with the
/// example's wall concept bound to `IFCMATERIAL` when `materials` is set.
fn materials_check(case: &Case, materials: bool, extra: &str) -> (Output, PathBuf) {
    let model = case.write(
        "model.ifc",
        &format!("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
         #2=IFCWALL('0000000000000000000002',$,$,$,$,$,$,$,$);\n\
         #3=IFCRELCONNECTSPATHELEMENTS('0000000000000000000003',$,$,$,$,#1,#2,(),(),.ATSTART.,.ATEND.);\n\
         #5=IFCMATERIAL('Concrete',$,$);\n\
         #6=IFCMATERIAL('Steel',$,$);\n\
         {extra}ENDSEC;\nEND-ISO-10303-21;\n"
        ),
    );
    let text = std::fs::read_to_string(case.definitions(true)).unwrap();
    let mut definitions: Value = serde_json::from_str(&text).unwrap();
    if materials {
        definitions["objectTypes"]["axioval:example.ifc.wall"]["externalNames"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["name"] = json!("IFCMATERIAL");
    }
    let definitions = case.write("definitions.json", &definitions.to_string());
    let bcf = case.path("issues.bcfzip");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(format!("{FIXTURES}/ruleset.json"))
        .arg("--bcf")
        .arg(&bcf)
        .env("SOURCE_DATE_EPOCH", "1790416800")
        .output()
        .unwrap();
    (output, bcf)
}

#[test]
fn a_rule_naming_a_resource_class_checks_its_resource_objects() {
    // The rule requires Pset_WallCommon.Reference; the model holds no
    // material property set, so neither material carries it.
    let case = Case::new("resources");
    let (output, bcf) = materials_check(&case, true, "");
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    let subjects: Vec<&str> = findings
        .iter()
        .map(|finding| finding["object_id"]["local_id"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["#5", "#6"], "{result:#}");
    // The report carries the materials, and the result labels them.
    let carried: Vec<&str> = result["report"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["kind"].as_str().unwrap())
        .collect();
    assert_eq!(carried, ["IFCMATERIAL", "IFCMATERIAL"], "{result:#}");
    assert_eq!(
        result["objects"]["ifc-step:model.ifc/#5"]["kind"], "IFCMATERIAL",
        "{result:#}"
    );
    // Each keeps its topic; no viewpoint selects a material.
    let archive = openbim_bcf::read_path(&bcf).unwrap();
    assert_eq!(archive.topics().count(), 2);
    assert!(archive.topics().all(|topic| topic.viewpoints.is_empty()));
}

#[test]
fn a_material_carrying_the_property_in_its_own_set_meets_the_rule() {
    // Concrete (#5) carries Pset_WallCommon.Reference in a material
    // property set; Steel (#6) carries none.
    let case = Case::new("material-properties");
    let (output, _) = materials_check(
        &case,
        true,
        "#7=IFCPROPERTYSINGLEVALUE('Reference',$,IFCLABEL('C30/37'),$);\n\
         #8=IFCMATERIALPROPERTIES('Pset_WallCommon',$,(#7),#5);\n",
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#6"], "{result:#}");
    assert!(open.is_empty(), "{result:#}");
}

#[test]
fn a_wall_rule_on_a_model_with_resources_never_sees_them() {
    let case = Case::new("resources-walls");
    let (output, _) = materials_check(&case, false, "");
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let subjects: Vec<&str> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["object_id"]["local_id"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["#1", "#2"], "{result:#}");
    assert!(result["report"].get("resources").is_none(), "{result:#}");
}

/// Walls whose `Pset_WallCommon.Reference` holds `values`, one per wall,
/// in order: `#1`, `#2`, ...
fn walls_with_references(values: &[&str]) -> String {
    let mut data = String::new();
    for (index, value) in values.iter().enumerate() {
        let wall = 10 * index + 1;
        writeln!(
            data,
            "#{wall}=IFCWALL('{wall:022}',$,$,$,$,$,$,$,$);\n\
             #{}=IFCPROPERTYSINGLEVALUE('Reference',$,{value},$);\n\
             #{}=IFCPROPERTYSET('{:022}',$,'Pset_WallCommon',$,(#{}));\n\
             #{}=IFCRELDEFINESBYPROPERTIES('{:022}',$,$,$,(#{wall}),#{});",
            wall + 1,
            wall + 2,
            wall + 2,
            wall + 1,
            wall + 3,
            wall + 3,
            wall + 2,
        )
        .unwrap();
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

/// The subjects of a result's findings and not-evaluated outcomes.
fn subjects_of(result: &Value) -> (Vec<String>, Vec<String>) {
    let subjects = |outcomes: &str| {
        result["report"][outcomes]
            .as_array()
            .unwrap()
            .iter()
            .map(|outcome| {
                outcome["object_id"]["local_id"]
                    .as_str()
                    .unwrap_or("(no object)")
                    .to_owned()
            })
            .collect()
    };
    (subjects("findings"), subjects("not_evaluated"))
}

#[test]
fn a_date_stating_a_time_zone_is_not_the_unzoned_date() {
    // `IfcDate` is an `xs:date`: `2022-01-01+00:00` is read with its zone,
    // and XML Schema never finds it equal to `2022-01-01`.
    let case = Case::new("zoned-dates");
    let (output, result) = case.geometry_rule(
        &walls_with_references(&["IFCDATE('2022-01-01+00:00')", "IFCDATE('2022-01-01')"]),
        &[],
        "axioval:capability.property-value",
        &registry_signature("axioval:capability.property-value"),
        entity("wall"),
        json!({"property": {"type": "propertyReference",
                            "property": "axioval:example.ifc.reference",
                            "propertySet": "axioval:example.ifc.pset-wall-common"},
               "values": {"type": "stringList", "value": ["2022-01-01"]}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#1"], "{result:#}");
    assert!(open.is_empty(), "{result:#}");
}

/// Walls #1, #11 and #21 state 15 March, 1 July and a zoned first of
/// January: only #11 is outside the first half of 2026, and #21 lies within
/// 14 hours of the unzoned start, in no order.
#[test]
fn a_date_between_two_date_bounds_is_compared_as_xml_schema_orders_it() {
    let case = Case::new("date-window");
    let reference = json!({"type": "propertyReference",
                           "property": "axioval:example.ifc.reference",
                           "propertySet": "axioval:example.ifc.pset-wall-common"});
    let (output, result) = case.geometry_rule(
        &walls_with_references(&[
            "IFCDATE('2026-03-15')",
            "IFCDATE('2026-07-01')",
            "IFCDATE('2026-01-01Z')",
        ]),
        &[],
        "axioval:capability.property-comparison",
        &registry_signature("axioval:capability.property-comparison"),
        entity("wall"),
        json!({
            "compared_selector": {"type": "selector", "value": entity("wall")},
            "compared_property": reference,
            "operator": {"type": "string", "value": "between"},
            "minimum_date": {"type": "date", "value": "2026-01-01"},
            "maximum_date": {"type": "date", "value": "2026-06-30"},
            "factor": {"type": "number", "value": 1},
            "component_mode": {"type": "string", "value": "checked"},
            "quantifier": {"type": "string", "value": "each"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#11"], "{result:#}");
    assert_eq!(open, ["#21"], "{result:#}");
}

/// Walls #1, #11 and #21 of type `T1` state 240 mm, 240.5 mm and 260 mm:
/// within 1 mm only #21 is inconsistent, reported against the median.
#[test]
fn walls_of_one_type_agree_on_a_value_within_a_tolerance() {
    let case = Case::new("consistent-tolerance");
    let model = walls_with_references(&[
        "IFCLENGTHMEASURE(240.)",
        "IFCLENGTHMEASURE(240.5)",
        "IFCLENGTHMEASURE(260.)",
    ])
    .replace("',$,$,$,$,$,$,$,$);", "',$,$,$,'T1',$,$,$,$);")
    .replace(
        "ENDSEC;\nEND-ISO",
        "#90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n\
         #91=IFCUNITASSIGNMENT((#90));\n\
         #92=IFCPROJECT('0000000000000000000092',$,'P',$,$,$,$,$,#91);\n\
         ENDSEC;\nEND-ISO",
    );
    let (output, result) = case.geometry_rule(
        &model,
        &[],
        "axioval:capability.consistent-value",
        &registry_signature("axioval:capability.consistent-value"),
        entity("wall"),
        json!({
            "key": {"type": "propertyReference", "propertySet": "axioval:attributes",
                    "property": "axioval:example.ifc.object-type"},
            "value": {"type": "propertyReference",
                      "property": "axioval:example.ifc.reference",
                      "propertySet": "axioval:example.ifc.pset-wall-common"},
            "tolerance_quantity": {"type": "quantity", "value": 1, "unit": "mm"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#21"], "{result:#}");
    assert!(open.is_empty(), "{result:#}");
    assert!(
        result["report"]["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("from the median 0.2405 m"),
        "{result:#}"
    );
}

#[test]
fn a_measure_of_unknown_unit_is_judged_by_its_declared_type_alone() {
    // The file has no project, so no measure's unit resolves: the value of
    // either wall is unread, but #1 declares a mass, not a time.
    let case = Case::new("unreadable-value");
    let (output, result) = case.geometry_rule(
        &walls_with_references(&["IFCMASSMEASURE(2.)", "IFCTIMEMEASURE(2.)"]),
        &[],
        "axioval:capability.property-value",
        &registry_signature("axioval:capability.property-value"),
        entity("wall"),
        json!({"property": {"type": "propertyReference",
                            "property": "axioval:example.ifc.reference",
                            "propertySet": "axioval:example.ifc.pset-wall-common"},
               "data_type": {"type": "string", "value": "IFCTIMEMEASURE"},
               "values": {"type": "stringList", "value": ["2"]},
               "si_units": {"type": "boolean", "value": true}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#1"], "{result:#}");
    assert_eq!(open, ["#11"], "{result:#}");
    assert!(
        result["report"]["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("is IFCMASSMEASURE, not IFCTIMEMEASURE"),
        "{result:#}"
    );
}

#[test]
fn a_logical_unknown_holds_no_value() {
    // `.U.` states no truth value, so the property holds none, as `$`; a
    // logical true or false is a boolean of the declared type.
    let case = Case::new("logical-unknown");
    let (output, result) = case.geometry_rule(
        &walls_with_references(&["IFCLOGICAL(.U.)", "IFCLOGICAL(.T.)", "IFCLOGICAL(.F.)"]),
        &[],
        "axioval:capability.property-data-type",
        &registry_signature("axioval:capability.property-data-type"),
        entity("wall"),
        json!({"property": {"type": "propertyReference",
                            "property": "axioval:example.ifc.reference",
                            "propertySet": "axioval:example.ifc.pset-wall-common"},
               "data_type": {"type": "string", "value": "IFCLOGICAL"}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#1"], "{result:#}");
    assert!(open.is_empty(), "{result:#}");
}

#[test]
fn a_complex_property_is_present_but_of_no_data_type() {
    // Wall #1's `Reference` is a complex property grouping a label; wall
    // #11's a label. Both are present, only #11's is an `IFCLABEL`.
    let ifc = walls_with_references(&["IFCLABEL('inner')", "IFCLABEL('outer')"]).replace(
        "#2=IFCPROPERTYSINGLEVALUE('Reference',$,IFCLABEL('inner'),$);",
        "#2=IFCCOMPLEXPROPERTY('Reference',$,'group',(#5));\n\
         #5=IFCPROPERTYSINGLEVALUE('Inner',$,IFCLABEL('inner'),$);",
    );
    let property = json!({"type": "propertyReference",
                          "property": "axioval:example.ifc.reference",
                          "propertySet": "axioval:example.ifc.pset-wall-common"});
    let (output, result) = Case::new("complex-property").geometry_rule(
        &ifc,
        &[],
        "axioval:capability.property-data-type",
        &registry_signature("axioval:capability.property-data-type"),
        entity("wall"),
        json!({"property": property, "data_type": {"type": "string", "value": "IFCLABEL"}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert_eq!(findings, ["#1"], "{result:#}");
    assert!(open.is_empty(), "{result:#}");
    assert!(
        result["report"]["findings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("is a complex property, not IFCLABEL"),
        "{result:#}"
    );
    let (output, result) = Case::new("complex-property-required").geometry_rule(
        &ifc,
        &[],
        "axioval:capability.property-required",
        &registry_signature("axioval:capability.property-required"),
        entity("wall"),
        json!({"property": property}),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let (findings, open) = subjects_of(&result);
    assert!(findings.is_empty() && open.is_empty(), "{result:#}");
}

#[test]
fn integrity_issues_and_unselectable_objects_are_reported() {
    let case = Case::new("integrity");
    let bcf = case.path("issues.bcfzip");
    let report = case.path("result.json");
    let output = case.check(
        &ifc("bad", false),
        true,
        &[
            "--bcf",
            bcf.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        output.stdout.is_empty(),
        "--report moves the JSON off stdout"
    );

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    let integrity = result["integrity"].as_array().unwrap();
    assert_eq!(integrity.len(), 1, "{result:#}");
    assert_eq!(integrity[0]["code"], axioval::ifc::INVALID_GLOBAL_ID);
    assert_eq!(integrity[0]["severity"], "warning");

    let stderr = stderr(&output);
    assert!(stderr.contains(axioval::ifc::INVALID_GLOBAL_ID), "{stderr}");
    assert!(
        stderr.contains("model.ifc/#2 has no valid unique GlobalId"),
        "{stderr}"
    );
}

#[test]
fn source_date_epoch_makes_bcf_output_reproducible() {
    let case = Case::new("reproducible");
    let bytes = |name: &str| {
        let bcf = case.path(name);
        let output = case.check(
            &ifc("0000000000000000000002", false),
            true,
            &["--bcf", bcf.to_str().unwrap()],
        );
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        std::fs::read(bcf).unwrap()
    };
    assert_eq!(bytes("a.bcfzip"), bytes("b.bcfzip"));
}

#[test]
fn unusable_input_exits_1_naming_the_file() {
    let case = Case::new("errors");
    let output = case.check("not a STEP file", true, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("model.ifc"), "{}", stderr(&output));

    let bcf = case.path("issues.bcfzip");
    let output = case.check(
        &ifc("0000000000000000000002", false),
        true,
        &["--bcf", bcf.to_str().unwrap(), "--bcf-date", "yesterday"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("BCF writer refused"),
        "{}",
        stderr(&output)
    );
    assert!(!bcf.exists(), "nothing is written when the writer refuses");
    assert!(output.stdout.is_empty(), "nor is the JSON report");
}

/// An ifcZIP archive of `members` (path, content).
fn ifc_zip(members: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write as _;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in members {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn a_zipped_model_gives_the_plain_files_report_under_its_archive_name() {
    let case = Case::new("ifczip");
    let model = ifc("0000000000000000000002", false);
    let plain = case.check(&model, true, &[]);
    assert_eq!(plain.status.code(), Some(3), "{}", stderr(&plain));

    let archive = case.path("model.ifczip");
    std::fs::write(
        &archive,
        ifc_zip(&[("readme.txt", "notes"), ("model.ifc", &model)]),
    )
    .unwrap();
    let zipped = case.check_file(&archive, true, &[]);
    assert_eq!(zipped.status.code(), Some(3), "{}", stderr(&zipped));
    let text = String::from_utf8(zipped.stdout.clone()).unwrap();
    assert!(text.contains("\"model.ifczip/model.ifc\""), "{text}");
    // Apart from the source name, the same report.
    let renamed: Value =
        serde_json::from_str(&text.replace("model.ifczip/model.ifc", "model.ifc")).unwrap();
    assert_eq!(renamed, json(&plain));

    // Two models in one archive: refused, naming both, nothing written.
    std::fs::write(&archive, ifc_zip(&[("a.ifc", &model), ("b.ifc", &model)])).unwrap();
    let refused = case.check_file(&archive, true, &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("holds 2 models (a.ifc, b.ifc); exactly one is read"),
        "{}",
        stderr(&refused)
    );
    assert!(refused.stdout.is_empty());
}

/// `ifc("0000000000000000000002", false)` in the buildingSMART XSD
/// configuration of IFC4 ADD2 TC1, numbered in document order as the STEP
/// form numbers it.
const WALLS_XSD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ifcXML xmlns="https://standards.buildingsmart.org/IFC/RELEASE/IFC4/ADD2_TC1/XML" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <IfcWall id="w1" GlobalId="0000000000000000000001"/>
  <IfcWallStandardCase GlobalId="0000000000000000000002"/>
  <IfcSlab GlobalId="0000000000000000000003"/>
  <IfcPropertySingleValue id="reference" Name="Reference">
    <NominalValue><IfcIdentifier-wrapper>W-1</IfcIdentifier-wrapper></NominalValue>
  </IfcPropertySingleValue>
  <IfcPropertySet id="pset" GlobalId="0000000000000000000005" Name="Pset_WallCommon">
    <HasProperties><IfcPropertySingleValue ref="reference" xsi:nil="true"/></HasProperties>
  </IfcPropertySet>
  <IfcRelDefinesByProperties GlobalId="0000000000000000000006">
    <RelatedObjects><IfcWall ref="w1" xsi:nil="true"/></RelatedObjects>
    <RelatingPropertyDefinition><IfcPropertySet ref="pset" xsi:nil="true"/></RelatingPropertyDefinition>
  </IfcRelDefinesByProperties>
</ifcXML>
"#;

#[test]
fn an_xsd_configuration_ifcxml_model_gives_the_report_of_its_step_form() {
    let case = Case::new("ifcxml-xsd");
    let plain = case.check(&ifc("0000000000000000000002", false), true, &[]);
    assert_eq!(plain.status.code(), Some(3), "{}", stderr(&plain));
    let model = case.write("model.ifcxml", WALLS_XSD);
    let xml = case.check_file(&model, true, &[]);
    assert_eq!(xml.status.code(), Some(3), "{}", stderr(&xml));
    let text = String::from_utf8(xml.stdout.clone()).unwrap();
    assert!(text.contains("\"ifc-xml\""), "{text}");
    // Apart from the source, the same report; finding ids and evidence
    // locators carry the fingerprint of the file's own bytes.
    let fingerprint = |text: &str| {
        let start = text.find("ifc:sha256:").unwrap() + "ifc:sha256:".len();
        text[start..start + 64].to_owned()
    };
    let text = text.replace(&fingerprint(&text), &fingerprint(&stdout(&plain)));
    let without_ids = |mut report: Value| {
        if let Some(findings) = report["findings"].as_array_mut() {
            for finding in findings {
                finding.as_object_mut().unwrap().remove("id");
            }
        }
        report
    };
    let renamed: Value = serde_json::from_str(
        &text
            .replace("model.ifcxml", "model.ifc")
            .replace("\"ifc-xml\"", "\"ifc-step\""),
    )
    .unwrap();
    assert_eq!(
        without_ids(renamed["report"].clone()),
        without_ids(json(&plain)["report"].clone())
    );

    // A decimal comma, which the XSD's reals do not admit: refused, naming
    // the file, nothing written.
    let comma = WALLS_XSD.replace(
        "<IfcSlab GlobalId=\"0000000000000000000003\"/>",
        "<IfcSlab GlobalId=\"0000000000000000000003\" Tag=\"x\" PredefinedType=\"floor\"><ObjectPlacement xsi:type=\"IfcLocalPlacement\"><RelativePlacement><IfcAxis2Placement3D><Location Coordinates=\"0,5 0 0\"/></IfcAxis2Placement3D></RelativePlacement></ObjectPlacement></IfcSlab>",
    );
    assert_ne!(comma, WALLS_XSD);
    let model = case.write("model.ifcxml", &comma);
    let refused = case.check_file(&model, true, &[]);
    assert_eq!(refused.status.code(), Some(1), "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("model.ifcxml")
            && stderr(&refused).contains("@Coordinates: invalid REAL scalar \"0,5\""),
        "{}",
        stderr(&refused)
    );
    assert!(refused.stdout.is_empty());
}

#[test]
fn bcf_options_without_bcf_output_are_usage_errors() {
    let case = Case::new("usage");
    let output = case.check(
        &ifc("0000000000000000000002", false),
        true,
        &["--bcf-author", "x"],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

fn report(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("report")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Walls #1..#n, none with a reference: n findings of one rule.
fn many_walls(n: usize) -> String {
    let mut walls = String::new();
    for i in 1..=n {
        let _ = writeln!(walls, "#{i}=IFCWALL('{i:0>22}',$,$,$,$,$,$,$,$);");
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{walls}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn a_summary_stays_small_and_names_the_next_command() {
    let case = Case::new("summary");
    let saved = case.path("result.json");
    let output = case.check(
        &many_walls(200),
        true,
        &["--summary", "--report", saved.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let summary = stdout(&output);
    assert!(
        summary.starts_with("status: findings · 200 finding(s)"),
        "{summary}"
    );
    assert!(summary.contains("wall-reference-required"), "{summary}");
    assert!(
        summary.contains("e.g. #1 IFCWALL 0000000000000000000001;"),
        "{summary}"
    );
    assert!(summary.contains("+197 more"), "{summary}");
    let rule_step = format!(
        "axioval report {} --rule wall-reference-required",
        saved.display()
    );
    assert!(summary.contains(&rule_step), "{summary}");
    // Bounded by distinct rules, not by findings.
    assert!(summary.len() < 1_000, "{} bytes:\n{summary}", summary.len());
    assert!(
        std::fs::metadata(&saved).unwrap().len() > 20 * summary.len() as u64,
        "the full result is on disk, not on stdout"
    );
    // The stderr count line would repeat the summary's status line.
    assert!(
        !stderr(&output).contains("finding(s)"),
        "{}",
        stderr(&output)
    );
}

/// Ten walls, each but #9 and #10 stating its `Pset_WallCommon.Reference`.
fn ten_walls_two_without_reference() -> String {
    let mut data = String::new();
    for i in 1..=10 {
        let _ = writeln!(data, "#{i}=IFCWALL('{i:0>22}',$,$,$,$,$,$,$,$);");
    }
    for i in 1..=8 {
        let (value, set, rel) = (100 + 3 * i, 101 + 3 * i, 102 + 3 * i);
        let _ = writeln!(
            data,
            "#{value}=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W{i}'),$);\n\
             #{set}=IFCPROPERTYSET('{set:0>22}',$,'Pset_WallCommon',$,(#{value}));\n\
             #{rel}=IFCRELDEFINESBYPROPERTIES('{rel:0>22}',$,$,$,(#{i}),#{set});"
        );
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn rule_status_counts_the_checked_and_failed_objects_of_each_rule() {
    let case = Case::new("rule-status");
    let saved = case.path("result.json");
    let saved = saved.to_str().unwrap();
    let output = case.check(
        &ten_walls_two_without_reference(),
        true,
        &["--rule-status", "--summary", "--report", saved],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    assert_eq!(
        result["report"]["rules"],
        json!([{"rule_id": "wall-reference-required", "checked": 10, "failed": 2,
                "not_evaluated": 0, "status": "failed"}]),
        "{result:#}"
    );
    let summary = stdout(&output);
    assert!(summary.contains("rules: 1 failed"), "{summary}");
    assert!(
        summary.contains(
            "failed                10 checked · 2 failed · 0 not evaluated  wall-reference-required"
        ),
        "{summary}"
    );

    // Over a model without walls the rule selected nothing, which is not a
    // pass over walls.
    let output = case.check(&many_walls(0), true, &["--rule-status", "--report", saved]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    assert_eq!(
        result["report"]["rules"][0]["status"], "nothing_selected",
        "{result:#}"
    );
    assert_eq!(result["report"]["rules"][0]["checked"], 0, "{result:#}");

    // Without the flag the result is unchanged.
    case.check(&many_walls(0), true, &["--report", saved]);
    let plain = std::fs::read_to_string(saved).unwrap();
    assert!(!plain.contains("\"rules\""), "{plain}");
}

#[test]
fn a_gated_rule_checks_only_the_walls_its_parent_failed_or_is_skipped() {
    let case = Case::new("rule-gates");
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let parent = ruleset["root"]["rules"][0].clone();
    let mut failed = parent.clone();
    failed["id"] = json!("a-recheck-failed-walls");
    failed["gate"] = json!({"rule": "wall-reference-required", "condition": "failedObjects"});
    let mut if_passed = parent.clone();
    if_passed["id"] = json!("b-only-if-passed");
    if_passed["gate"] = json!({"rule": "wall-reference-required", "condition": "allIfPassed"});
    ruleset["root"]["rules"] = json!([parent, failed, if_passed]);
    let ruleset = case.write("gated.json", &ruleset.to_string());
    let model = case.write("model.ifc", &ten_walls_two_without_reference());
    let definitions = case.definitions(true);
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .arg("--rule-status")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let rules = &result["report"]["rules"];
    // The recheck selects only the two walls its parent failed.
    assert_eq!(
        rules[0],
        json!({"rule_id": "a-recheck-failed-walls", "checked": 2, "failed": 2,
                "not_evaluated": 0, "status": "failed"}),
        "{result:#}"
    );
    assert_eq!(rules[1]["status"], "skipped", "{result:#}");
}

#[test]
fn walls_a_derived_classification_leaves_unclassified_are_reported() {
    let case = Case::new("classifications");
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    definitions["definitions"]["axioval:example.unclassified"] = json!({
        "id": "axioval:example.unclassified",
        "name": {"default": "Unclassified objects", "translations": {}},
        "capability": "axioval:capability.unclassified-object",
        "parameters": registry_signature("axioval:capability.unclassified-object"),
        "citations": [],
        "tags": [],
    });
    let definitions = case.write("definitions.json", &definitions.to_string());
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("walls-classified");
    rule["definitionId"] = json!("axioval:example.unclassified");
    rule["parameters"] = json!({"classification": {"type": "string", "value": "wall-kind"}});
    ruleset["classifications"] = json!({"wall-kind": {
        "id": "wall-kind",
        "name": {"default": "Wall kind", "translations": {}},
        "rows": [{
            "selector": {"kind": "property",
                         "propertySet": "axioval:example.ifc.pset-wall-common",
                         "property": "axioval:example.ifc.reference", "operator": "exists"},
            "class": "referenced",
        }],
    }});
    let ruleset = case.write("classified.json", &ruleset.to_string());
    let model = case.write("model.ifc", &ten_walls_two_without_reference());
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2, "{result:#}");
    assert!(
        findings.iter().all(|finding| finding["message"]
            .as_str()
            .unwrap()
            .contains("unclassified")),
        "{result:#}"
    );
}

/// A three-level wall classification: referenced walls in the leaf
/// `kg-331`, the rest in `kg-332`, both below `kg-330` below `kg-300`.
/// The reference rule selects the inner class with its descendants, and a
/// takeoff counts the walls by level 1 and by leaf.
#[test]
fn a_class_tree_selects_descendants_and_groups_a_takeoff_by_level() {
    let case = Case::new("class-tree");
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    definitions["definitions"]["axioval:example.takeoff"] = json!({
        "id": "axioval:example.takeoff",
        "name": {"default": "Takeoff", "translations": {}},
        "capability": "axioval:capability.quantity-takeoff",
        "parameters": registry_signature("axioval:capability.quantity-takeoff"),
        "citations": [],
        "tags": [],
    });
    let definitions = case.write("definitions.json", &definitions.to_string());
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let mut reference = ruleset["root"]["rules"][0].clone();
    reference["applicability"]["groups"]["walls"]["selector"] = json!({
        "kind": "derivedClass", "classification": "wall-group", "class": "kg-330",
        "includeDescendants": true});
    let mut takeoff = reference.clone();
    takeoff["id"] = json!("wall-takeoff");
    takeoff["definitionId"] = json!("axioval:example.takeoff");
    takeoff["parameters"] = json!({
        "group_1": {"type": "propertyReference", "propertySet": "axioval:classification",
                    "property": "wall-group;level=1"},
        "group_1_name": {"type": "string", "value": "group"},
        "group_2": {"type": "propertyReference", "propertySet": "axioval:classification",
                    "property": "wall-group"},
        "group_2_name": {"type": "string", "value": "class"},
    });
    ruleset["root"]["rules"] = json!([reference, takeoff]);
    let class = |id: &str, parent: Option<&str>| {
        let mut class = json!({"id": format!("kg-{id}"), "code": id,
                               "name": {"default": format!("Group {id}"), "translations": {}}});
        if let Some(parent) = parent {
            class["parent"] = json!(format!("kg-{parent}"));
        }
        class
    };
    ruleset["classifications"] = json!({"wall-group": {
        "id": "wall-group",
        "name": {"default": "Wall group", "translations": {}},
        "classes": [class("300", None), class("330", Some("300")),
                    class("331", Some("330")), class("332", Some("330"))],
        "rows": [
            {"selector": {"kind": "property",
                          "propertySet": "axioval:example.ifc.pset-wall-common",
                          "property": "axioval:example.ifc.reference", "operator": "exists"},
             "class": "kg-331"},
            {"selector": {"kind": "entityType", "objectType": "axioval:example.ifc.wall"},
             "class": "kg-332"},
        ],
    }});
    let ruleset = case.write("classified.json", &ruleset.to_string());
    let model = case.write("model.ifc", &ten_walls_two_without_reference());
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2, "{result:#}");
    let table = &result["report"]["tables"][0];
    assert_eq!(table["group_by"], json!(["group", "class"]), "{result:#}");
    let rows: Vec<(&Value, &Value)> = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (&row["group"], &row["values"][0]))
        .collect();
    assert_eq!(
        rows,
        [
            (
                &json!(["kg-300", "kg-331"]),
                &json!({"type": "exact", "value": 8.0})
            ),
            (
                &json!(["kg-300", "kg-332"]),
                &json!({"type": "exact", "value": 2.0})
            ),
        ],
        "{result:#}"
    );
}

#[test]
fn a_summary_without_a_saved_result_says_how_to_get_one() {
    let case = Case::new("summary-unsaved");
    let output = case.check(&many_walls(2), true, &["--summary"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(
        stdout(&output).contains("rerun with --report <file>"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_saved_result_can_be_summarized_and_paged_without_rerunning() {
    let case = Case::new("drill");
    let saved = case.path("result.json");
    let output = case.check(&many_walls(5), true, &["--report", saved.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3));
    let saved = saved.to_str().unwrap();

    // The result names what each object is, not just its STEP number.
    let result: Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    assert_eq!(
        result["objects"]["ifc-step:model.ifc/#2"],
        json!({"kind": "IFCWALL", "global_id": "0000000000000000000002"})
    );

    let summary = report(&[saved, "--json"]);
    assert_eq!(summary.status.code(), Some(0), "{}", stderr(&summary));
    let summary: Value = serde_json::from_slice(&summary.stdout).unwrap();
    assert_eq!(summary["status"], "findings");
    assert_eq!(summary["groups"][0]["count"], 5);

    let first = report(&[saved, "--rule", "wall-reference-required", "--limit", "2"]);
    let text = stdout(&first);
    assert!(text.contains("showing 1–2 of 5"), "{text}");
    let next = text
        .lines()
        .find_map(|line| line.strip_prefix("next: axioval report "))
        .unwrap_or_else(|| panic!("no next-page hint in:\n{text}"));
    // The hint is a runnable command that continues where the page ended.
    let args: Vec<&str> = next.split(' ').collect();
    let second = stdout(&report(&args));
    assert!(second.contains("showing 3–4 of 5"), "{second}");
    assert!(second.contains("#3 IFCWALL"), "{second}");

    let by_global_id = report(&[saved, "--object", "0000000000000000000004", "--json"]);
    let listing: Value = serde_json::from_slice(&by_global_id.stdout).unwrap();
    assert_eq!(listing["total"], 1);
    assert_eq!(
        listing["entries"][0]["object"],
        "#4 IFCWALL 0000000000000000000004"
    );
    assert!(listing["entries"][0].get("evidence").is_none());

    let with_evidence = report(&[saved, "--object", "#4", "--evidence", "--json"]);
    let listing: Value = serde_json::from_slice(&with_evidence.stdout).unwrap();
    assert!(
        listing["entries"][0]["evidence"][0]
            .as_str()
            .unwrap()
            .contains("sha256:")
    );

    let none = stdout(&report(&[saved, "--code", "identity.invalid-global-id"]));
    assert_eq!(none, "no matching entries\n");
}

#[test]
fn reading_a_missing_result_exits_1() {
    let output = report(&["/nonexistent/result.json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("/nonexistent/result.json"));
}

#[test]
fn every_hint_runs_unchanged_in_a_shell() {
    // Ids start with `#`, which an unquoted shell word turns into a comment:
    // `--object #1` would silently become `--object` with no value.
    let case = Case::new("shell");
    let saved = case.path("result.json");
    let output = case.check(
        &many_walls(3),
        true,
        &["--summary", "--report", saved.to_str().unwrap()],
    );
    let summary = stdout(&output);
    let hint = summary
        .lines()
        .map(str::trim)
        .find(|line| line.contains("--object"))
        .unwrap_or_else(|| panic!("no --object hint in:\n{summary}"));
    assert!(hint.contains("'#1'"), "{hint}");
    let command = hint.replacen("axioval", env!("CARGO_BIN_EXE_axioval"), 1);
    let ran = Command::new("sh").arg("-c").arg(&command).output().unwrap();
    assert_eq!(ran.status.code(), Some(0), "{}", stderr(&ran));
    let listing = stdout(&ran);
    assert!(listing.contains("#1 IFCWALL"), "{listing}");
    assert!(listing.contains("evidence: "), "{listing}");
    assert!(listing.contains("showing 1–1 of 1"), "{listing}");
}

/// Two 4 m × 0.2 m × 3 m walls crossing at right angles, so each penetrates
/// the other by half its thickness (0.1 m), and a proxy with no body at all.
fn crossing_walls() -> String {
    let wall = |first: u32, x: f64, y: f64, length: f64, width: f64, global: &str| {
        let [p, pos, profile, solid, shape, product, wall] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        format!(
            "#{p}=IFCCARTESIANPOINT(({x},{y}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length},{width});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,3.);\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{product}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{wall}=IFCWALL('{global}',$,$,$,$,#3,#{product},$,$);\n"
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}\
         #30=IFCBUILDINGELEMENTPROXY('0000000000000000000030',$,$,$,$,#3,$,$,$);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        wall(10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000016"),
        wall(20, 2.0, 0.0, 0.2, 4.0, "0000000000000000000026"),
    )
}

impl Case {
    /// The fixture packages with the rule swapped for a wall/wall clash check.
    fn clash_packages(&self) -> (PathBuf, PathBuf) {
        let definitions = self.definitions(true);
        let mut definitions: Value =
            serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
        definitions["definitions"]["axioval:example.clash"] = json!({
            "id": "axioval:example.clash",
            "name": {"default": "Clash", "translations": {}},
            "description": {"default": "Bodies must not interpenetrate.", "translations": {}},
            "capability": "axioval:capability.clash",
            "parameters": registry_signature("axioval:capability.clash"),
            "citations": [],
            "tags": [],
        });
        let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["id"] = json!("walls-clash");
        rule["definitionId"] = json!("axioval:example.clash");
        rule["parameters"] = json!({
            "counterparts": {"type": "selector", "value": {
                "kind": "entityType", "objectType": "axioval:example.ifc.wall",
                "includeSubtypes": true}},
            "penetration_tolerance_metres": {"type": "number", "value": 0.01},
        });
        (
            self.write("definitions.json", &definitions.to_string()),
            self.write("ruleset.json", &ruleset.to_string()),
        )
    }

    fn clash_check(&self, extra: &[&str]) -> Output {
        let model = self.write("model.ifc", &crossing_walls());
        let (definitions, ruleset) = self.clash_packages();
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .arg("check")
            .arg("--model")
            .arg(model)
            .arg("--definitions")
            .arg(definitions)
            .arg("--ruleset")
            .arg(ruleset)
            .args(extra)
            .output()
            .unwrap()
    }
}

#[test]
fn with_geometry_a_clash_between_real_ifc_bodies_is_found() {
    let case = Case::new("geometry-clash");
    let saved = case.path("result.json");
    let output = case.clash_check(&["--geometry", "--report", saved.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(message.contains("penetration 0.1000 m"), "{message}");

    // Both walls are rectangular extrusions: exact, not tessellated. The
    // proxy has no body representation, so it is unmeasured, not ignored.
    let geometry = &result["geometry"];
    assert_eq!(geometry["exact"], 2, "{geometry:#}");
    assert_eq!(geometry["tessellated"], 0);
    assert_eq!(geometry["unmeasured"][0]["object"]["local_id"], "#30");
    assert_eq!(
        geometry["unmeasured"][0]["reason"],
        "no body representation"
    );
    assert!(
        stderr(&output).contains("#30 was not meshed: no body representation"),
        "{}",
        stderr(&output)
    );

    let summary = stdout(&report(&[saved.to_str().unwrap()]));
    assert!(
        summary.contains("geometry: 2 exact · 0 tessellated"),
        "{summary}"
    );
    assert!(summary.contains("--section geometry"), "{summary}");
    let listing = stdout(&report(&[saved.to_str().unwrap(), "--section", "geometry"]));
    assert!(
        listing.contains("#30 IFCBUILDINGELEMENTPROXY 0000000000000000000030"),
        "{listing}"
    );
}

/// The crossing walls with `extra` entities added to the model.
fn crossing_walls_with(extra: &str) -> String {
    crossing_walls().replace("ENDSEC;\nEND-ISO", &format!("{extra}ENDSEC;\nEND-ISO"))
}

impl Case {
    /// Runs a wall/wall clash rule with the registry's full signature.
    fn wall_clash(&self, model: &str, parameters: &Value) -> (Output, Value) {
        let mut bound = json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "penetration_tolerance_metres": {"type": "number", "value": 0.01},
        });
        for (name, value) in parameters.as_object().unwrap() {
            bound[name] = value.clone();
        }
        self.geometry_rule(
            model,
            &[],
            "axioval:capability.clash",
            &registry_signature("axioval:capability.clash"),
            entity("wall"),
            bound,
        )
    }
}

/// Only the wall running along x (4 m) is longer than a metre in x; the
/// crossing wall is 0.2 m there. Neither states a reference.
#[test]
fn with_geometry_a_measured_extent_selects_walls() {
    let case = Case::new("measured-extent");
    let (output, result) = case.geometry_rule(
        &crossing_walls(),
        &[],
        "axioval:capability.property-exists",
        &registry_signature("axioval:capability.property-exists"),
        json!({"kind": "allOf", "operands": [
            entity("wall"),
            {"kind": "property", "propertySet": "axioval:measured", "property": "extent_x",
             "operator": "greaterThan", "value": {"type": "quantity", "value": 1, "unit": "m"}},
        ]}),
        json!({"property": {"type": "propertyReference",
                            "property": "axioval:example.ifc.reference",
                            "propertySet": "axioval:example.ifc.pset-wall-common"}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#16", "{result:#}");
}

/// Storeys at 0, 3 and 7 m of one building: only the first floor is more
/// than 3.5 m high, and the top storey has no height.
#[test]
fn a_storey_is_selected_by_its_height_to_the_next_storey() {
    let case = Case::new("level-height");
    let model = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCCARTESIANPOINT((0.,0.,3.));\n\
         #5=IFCAXIS2PLACEMENT3D(#4,$,$);\n\
         #6=IFCLOCALPLACEMENT($,#5);\n\
         #7=IFCCARTESIANPOINT((0.,0.,7.));\n\
         #8=IFCAXIS2PLACEMENT3D(#7,$,$);\n\
         #9=IFCLOCALPLACEMENT($,#8);\n\
         #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #21=IFCUNITASSIGNMENT((#20));\n\
         #22=IFCPROJECT('0000000000000000000022',$,'P',$,$,$,$,$,#21);\n\
         #10=IFCBUILDING('0000000000000000000010',$,'B',$,$,#3,$,$,.ELEMENT.,$,$,$);\n\
         #11=IFCBUILDINGSTOREY('0000000000000000000011',$,'0',$,$,#3,$,$,.ELEMENT.,0.);\n\
         #12=IFCBUILDINGSTOREY('0000000000000000000012',$,'1',$,$,#6,$,$,.ELEMENT.,3.);\n\
         #13=IFCBUILDINGSTOREY('0000000000000000000013',$,'2',$,$,#9,$,$,.ELEMENT.,7.);\n\
         #14=IFCRELAGGREGATES('0000000000000000000014',$,$,$,#10,(#11,#12,#13));\n\
         ENDSEC;\nEND-ISO-10303-21;\n";
    let (output, result) = case.geometry_rule(
        model,
        &[("storey", "IfcBuildingStorey")],
        "axioval:capability.property-exists",
        &registry_signature("axioval:capability.property-exists"),
        json!({"kind": "allOf", "operands": [
            entity("storey"),
            {"kind": "property", "propertySet": "axioval:measured", "property": "level_height",
             "operator": "greaterThan", "value": {"type": "quantity", "value": 3.5, "unit": "m"}},
        ]}),
        json!({"property": {"type": "propertyReference",
                            "property": "axioval:example.ifc.reference",
                            "propertySet": "axioval:example.ifc.pset-wall-common"}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#12", "{result:#}");
}

/// Site #10 has a body, site #20 none, and site #30 a body that cannot be
/// read (an open profile swept as a solid): only #20 lacks geometry, and #30
/// is not evaluated.
#[test]
fn a_site_without_geometry_is_found_from_its_representations() {
    let case = Case::new("site-geometry");
    let model = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCCARTESIANPOINT((10.,0.));\n\
         #7=IFCPOLYLINE((#6,#6));\n\
         #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #21=IFCUNITASSIGNMENT((#20));\n\
         #22=IFCPROJECT('0000000000000000000022',$,'P',$,$,$,$,(#5),#21);\n\
         #11=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,100.,50.);\n\
         #12=IFCEXTRUDEDAREASOLID(#11,#2,#4,1.);\n\
         #13=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#12));\n\
         #14=IFCPRODUCTDEFINITIONSHAPE($,$,(#13));\n\
         #10=IFCSITE('0000000000000000000010',$,'S1',$,$,#3,#14,$,.ELEMENT.,$,$,$,$,$);\n\
         #30=IFCSITE('0000000000000000000030',$,'S3',$,$,#3,#34,$,.ELEMENT.,$,$,$,$,$);\n\
         #31=IFCARBITRARYOPENPROFILEDEF(.CURVE.,$,#7);\n\
         #32=IFCEXTRUDEDAREASOLID(#31,#2,#4,1.);\n\
         #33=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#32));\n\
         #34=IFCPRODUCTDEFINITIONSHAPE($,$,(#33));\n\
         #40=IFCSITE('0000000000000000000040',$,'S2',$,$,#3,$,$,.ELEMENT.,$,$,$,$,$);\n\
         ENDSEC;\nEND-ISO-10303-21;\n";
    let (output, result) = case.geometry_rule(
        model,
        &[("site", "IfcSite")],
        "axioval:capability.property-required",
        &registry_signature("axioval:capability.property-required"),
        entity("site"),
        json!({"property": {"type": "propertyReference", "propertySet": "axioval:body",
                            "property": "axioval:example.ifc.body-count"}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#40", "{result:#}");
    let open: Vec<&Value> = result["report"]["not_evaluated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| &outcome["object_id"]["local_id"])
        .collect();
    assert_eq!(open, [&json!("#30")], "{result:#}");
}

/// Storeys of one building state no `Elevation` and are placed at 0, 3
/// and 6 m; the one at 6 m is named `4` where `3` is due.
#[test]
fn storeys_without_an_elevation_are_named_in_the_order_of_their_placements() {
    let case = Case::new("name-sequence-placement");
    let model = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCCARTESIANPOINT((0.,0.,3.));\n\
         #5=IFCAXIS2PLACEMENT3D(#4,$,$);\n\
         #6=IFCLOCALPLACEMENT($,#5);\n\
         #7=IFCCARTESIANPOINT((0.,0.,6.));\n\
         #8=IFCAXIS2PLACEMENT3D(#7,$,$);\n\
         #9=IFCLOCALPLACEMENT($,#8);\n\
         #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #21=IFCUNITASSIGNMENT((#20));\n\
         #22=IFCPROJECT('0000000000000000000022',$,'P',$,$,$,$,$,#21);\n\
         #10=IFCBUILDING('0000000000000000000010',$,'B',$,$,#3,$,$,.ELEMENT.,$,$,$);\n\
         #11=IFCBUILDINGSTOREY('0000000000000000000011',$,'4',$,$,#9,$,$,.ELEMENT.,$);\n\
         #12=IFCBUILDINGSTOREY('0000000000000000000012',$,'1',$,$,#3,$,$,.ELEMENT.,$);\n\
         #13=IFCBUILDINGSTOREY('0000000000000000000013',$,'2',$,$,#6,$,$,.ELEMENT.,$);\n\
         #14=IFCRELAGGREGATES('0000000000000000000014',$,$,$,#10,(#11,#12,#13));\n\
         ENDSEC;\nEND-ISO-10303-21;\n";
    let attribute = |property: &str| {
        json!({"type": "propertyReference", "propertySet": "axioval:attributes",
               "property": format!("axioval:example.ifc.{property}")})
    };
    let (output, result) = case.geometry_rule(
        model,
        &[("building", "IfcBuilding"), ("storey", "IfcBuildingStorey")],
        "axioval:capability.name-sequence",
        &registry_signature("axioval:capability.name-sequence"),
        entity("building"),
        json!({
            "member_selector": {"type": "selector", "value": entity("storey")},
            "name": attribute("name"),
            "order": attribute("elevation"),
            "relationship": {"type": "string", "value": "IfcRelAggregates"},
            "order_fallback": {"type": "string", "value": "placement_height"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#11".to_owned(),
            "axioval:attributes.axioval:example.ifc.name 4 does not follow 2; expected 3"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// A clash that is a warning in general is an error where a selected wall
/// is involved.
#[test]
fn with_geometry_a_severity_override_raises_a_clash_involving_a_wall() {
    let case = Case::new("clash-severity-override");
    let clash = |extra: &Value| {
        case.write("model.ifc", &crossing_walls());
        case.geometry_rule_with(
            &["model.ifc"],
            &[],
            (
                "axioval:capability.clash",
                &registry_signature("axioval:capability.clash"),
            ),
            entity("wall"),
            json!({
                "counterparts": {"type": "selector", "value": entity("wall")},
                "penetration_tolerance_metres": {"type": "number", "value": 0.01},
            }),
            extra,
            &[],
        )
    };
    let severities = |result: &Value| -> Vec<String> {
        result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["severity"].as_str().unwrap().to_owned())
            .collect()
    };
    let (output, result) = clash(&json!({"severity": "warning"}));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(severities(&result), ["warning"], "{result:#}");
    let (output, result) = clash(&json!({
        "severity": "warning",
        "severityOverrides": [{"selector": entity("wall"), "severity": "error"}],
    }));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(severities(&result), ["error"], "{result:#}");
}

/// Storey #100 ("Level 1") holds wall #19 and duct #29, which crosses the
/// wall inside space #39 ("101"), aggregated to the storey.
fn a_duct_through_a_wall_on_a_storey() -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let duct = "IFCDUCTSEGMENT('GID',$,$,$,$,PL,REP,$,$)";
    let space = "IFCSPACE('GID',$,'101',$,$,PL,REP,$,.ELEMENT.,$,$)";
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
         #100=IFCBUILDINGSTOREY('0000000000000000000100',$,'Level 1',$,$,$,$,$,.ELEMENT.,0.);\n\
         #101=IFCRELAGGREGATES('0000000000000000000101',$,$,$,#100,(#39));\n\
         #102=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000102',$,$,$,(#19,#29),#100);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [5.0, 5.0, 0.0], [0.2, 8.0, 3.0], wall),
        placed_box(20, [5.0, 5.0, 1.0], [4.0, 0.3, 0.3], duct),
        placed_box(30, [5.0, 5.0, 0.0], [10.0, 10.0, 3.0], space),
    )
}

impl Case {
    /// Checks the duct against walls with `args` added, saving the result.
    fn located_clash(&self, args: &[&str]) -> (Output, Value) {
        self.write("model.ifc", &a_duct_through_a_wall_on_a_storey());
        self.geometry_rule_with(
            &["model.ifc"],
            &[("duct", "IfcDuctSegment")],
            (
                "axioval:capability.clash",
                &registry_signature("axioval:capability.clash"),
            ),
            entity("duct"),
            json!({
                "counterparts": {"type": "selector", "value": entity("wall")},
                "penetration_tolerance_metres": {"type": "number", "value": 0.01},
            }),
            &json!({}),
            args,
        )
    }
}

#[test]
fn with_geometry_a_duct_wall_clash_is_located_by_storey_and_derived_space() {
    let case = Case::new("clash-located");
    let (output, result) = case.located_clash(&["--locate", "geometry"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    let location = &findings[0]["location"];
    assert_eq!(
        location["storeys"][0]["id"]["local_id"], "#100",
        "{result:#}"
    );
    assert_eq!(location["storeys"][0]["name"], "Level 1", "{result:#}");
    assert_eq!(location["spaces"][0]["id"]["local_id"], "#39", "{result:#}");
    assert_eq!(location["spaces"][0]["name"], "101", "{result:#}");
    assert!(location.get("unresolved").is_none(), "{result:#}");

    // The saved result filters by storey name.
    let saved = case.path("result.json");
    let saved = saved.to_str().unwrap();
    let listed = report(&[saved, "--location", "Level 1"]);
    assert_eq!(listed.status.code(), Some(0), "{}", stderr(&listed));
    let text = stdout(&listed);
    assert!(text.contains("[finding] error under-test"), "{text}");
    assert!(
        text.contains("location: storey Level 1 (#100); space 101 (#39)"),
        "{text}"
    );
    assert!(text.contains("showing 1–1 of 1"), "{text}");
    let elsewhere = stdout(&report(&[saved, "--location", "Level 2"]));
    assert!(elsewhere.contains("no matching entries"), "{elsewhere}");

    // The BCF topic is labelled with the storey and the space.
    let bcf = case.path("located.bcfzip");
    let (output, _) = case.located_clash(&[
        "--locate",
        "geometry",
        "--bcf",
        bcf.to_str().unwrap(),
        "--bcf-date",
        "2026-09-27T00:00:00Z",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let archive = openbim_bcf::read_slice(&std::fs::read(&bcf).unwrap()).unwrap();
    let topic = &archive.topics().next().unwrap().topic;
    assert_eq!(
        topic.labels,
        ["under-test", "example", "Storey: Level 1", "Space: 101"],
        "{topic:?}"
    );
}

#[test]
fn without_locating_the_result_is_byte_identical() {
    let case = Case::new("clash-unlocated");
    let (output, _) = case.located_clash(&[]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let plain = std::fs::read(case.path("result.json")).unwrap();
    let (output, _) = case.located_clash(&["--locate", "none"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let none = std::fs::read(case.path("result.json")).unwrap();
    assert_eq!(plain, none);
    assert!(!String::from_utf8(plain).unwrap().contains("location"));

    // Deriving spaces needs the bodies.
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .current_dir(case.path("."))
        .args([
            "check",
            "--model",
            "model.ifc",
            "--definitions",
            "definitions.json",
        ])
        .args(["--ruleset", "ruleset.json", "--locate", "geometry"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("--locate geometry"),
        "{}",
        stderr(&output)
    );
}

/// The two walls cross 0.2 m wide in plan and 3 m high. Axis tolerances
/// below that keep the clash; one above it hides it.
#[test]
fn with_geometry_clash_axis_tolerances_are_read_from_the_ifc_bodies() {
    let case = Case::new("clash-axis-tolerances");
    let (output, result) = case.wall_clash(
        &crossing_walls(),
        &json!({"horizontal_tolerance_metres": {"type": "number", "value": 0.15},
               "vertical_tolerance_metres": {"type": "number", "value": 2.5}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = sorted_findings(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert!(
        findings[0]
            .1
            .contains("reaching 0.2000 m in plan and 3.0000 m vertically"),
        "{findings:?}"
    );

    let (output, result) = case.wall_clash(
        &crossing_walls(),
        &json!({"vertical_tolerance_metres": {"type": "number", "value": 3.5}}),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");
}

/// Walls assigned to one system, or drawn on one layer, are not checked
/// against each other; the same walls in different systems still clash.
#[test]
fn with_geometry_clash_exclusions_read_ifc_systems_and_layers() {
    let case = Case::new("clash-exclusions");
    let same_system = crossing_walls_with(
        "#40=IFCSYSTEM('0000000000000000000040',$,'Structure',$,$);\n\
         #41=IFCRELASSIGNSTOGROUP('0000000000000000000041',$,$,$,(#16,#26),$,#40);\n",
    );
    let by_system = json!({"exclude_paths": {"type": "stringList",
                                             "value": ["IfcRelAssignsToGroup:backward"]}});
    let (output, result) = case.wall_clash(&same_system, &by_system);
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");

    let two_systems = crossing_walls_with(
        "#40=IFCSYSTEM('0000000000000000000040',$,'Structure',$,$);\n\
         #41=IFCRELASSIGNSTOGROUP('0000000000000000000041',$,$,$,(#16),$,#40);\n\
         #42=IFCSYSTEM('0000000000000000000042',$,'Partitions',$,$);\n\
         #43=IFCRELASSIGNSTOGROUP('0000000000000000000043',$,$,$,(#26),$,#42);\n",
    );
    let (output, result) = case.wall_clash(&two_systems, &by_system);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), vec!["#16"], "{result:#}");

    let by_layer = json!({"exclude_same_layer": {"type": "boolean", "value": true}});
    let one_layer =
        crossing_walls_with("#40=IFCPRESENTATIONLAYERASSIGNMENT('A-WALL',$,(#14,#24),$);\n");
    let (output, result) = case.wall_clash(&one_layer, &by_layer);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{result:#}",
        stderr(&output)
    );
    assert!(finding_ids(&result).is_empty(), "{result:#}");

    let two_layers = crossing_walls_with(
        "#40=IFCPRESENTATIONLAYERASSIGNMENT('A-WALL',$,(#14),$);\n\
         #41=IFCPRESENTATIONLAYERASSIGNMENT('S-WALL',$,(#24),$);\n",
    );
    let (output, result) = case.wall_clash(&two_layers, &by_layer);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), vec!["#16"], "{result:#}");
}

/// A long wall #16 crossed by three short walls: grouped by type pair, the
/// three clashes are one issue on the long wall.
#[test]
fn with_geometry_clashes_of_one_type_pair_are_one_issue() {
    let case = Case::new("clash-groups");
    let model = walls_file(&[
        (10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000016"),
        (20, 1.0, 0.0, 0.2, 2.0, "0000000000000000000026"),
        (30, 2.0, 0.0, 0.2, 2.0, "0000000000000000000036"),
        (40, 3.0, 0.0, 0.2, 2.0, "0000000000000000000046"),
    ]);
    let (output, result) = case.wall_clash(
        &model,
        &json!({"group_by": {"type": "string", "value": "type_pair"}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#16");
    let related: Vec<&str> = findings[0]["related"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| object["local_id"].as_str().unwrap())
        .collect();
    assert_eq!(related, vec!["#26", "#36", "#46"]);
    assert_eq!(findings[0]["evidence"].as_array().unwrap().len(), 3);
    let message = findings[0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("3 clashes of IFCWALL with IFCWALL: "),
        "{message}"
    );
}

/// The crossing walls share a 0.2 m by 0.2 m by 3 m intersection: graded by
/// its smallest extent, it is an error past 0.1 m and stays at its class's
/// info below 0.3 m.
#[test]
fn with_geometry_clash_severities_grade_the_intersection() {
    let case = Case::new("clash-severities");
    let graded = |above: f64| {
        let (output, result) = case.wall_clash(
            &crossing_walls(),
            &json!({
                "severity_by_class": {"type": "table", "value": [
                    {"class": {"type": "string", "value": "intersection"},
                     "severity": {"type": "string", "value": "info"}}]},
                "grade_by": {"type": "string", "value": "smallest_extent"},
                "severity_grades": {"type": "table", "value": [
                    {"above": {"type": "number", "value": above},
                     "severity": {"type": "string", "value": "error"}}]},
            }),
        );
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        let findings = result["report"]["findings"].as_array().unwrap().clone();
        assert_eq!(findings.len(), 1, "{result:#}");
        (
            findings[0]["severity"].as_str().unwrap().to_owned(),
            findings[0]["message"].as_str().unwrap().to_owned(),
        )
    };
    let (severity, message) = graded(0.1);
    assert_eq!(severity, "error", "{message}");
    assert!(
        message.ends_with(", graded error by its smallest extent of 0.2000 m"),
        "{message}"
    );
    let (severity, message) = graded(0.3);
    assert_eq!(severity, "info", "{message}");
}

/// A wall #26 placed at 30° and a slab #36 placed along it, its edge sunk
/// 10 mm into the wall.
fn slab_in_a_turned_wall() -> String {
    let body = |first: u32, centre: [f64; 2], size: [f64; 2], depth: f64| {
        let [p, pos, profile, solid, shape, definition] =
            [0, 1, 2, 3, 4, 5].map(|offset| first + offset);
        format!(
            "#{p}=IFCCARTESIANPOINT(({:?},{:?}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{:?},{:?});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth:?});\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n",
            centre[0], centre[1], size[0], size[1]
        )
    };
    let (sin, cos) = (std::f64::consts::PI / 6.0).sin_cos();
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCDIRECTION(({cos:?},{sin:?},0.));\n\
         #7=IFCAXIS2PLACEMENT3D(#1,#4,#6);\n\
         #8=IFCLOCALPLACEMENT($,#7);\n\
         #9=IFCCARTESIANPOINT((0.,0.,1.));\n\
         #11=IFCAXIS2PLACEMENT3D(#9,#4,#6);\n\
         #12=IFCLOCALPLACEMENT($,#11);\n\
         #13=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #14=IFCUNITASSIGNMENT((#13));\n\
         #15=IFCPROJECT('0000000000000000000015',$,'P',$,$,$,$,(#5),#14);\n\
         {}#26=IFCWALL('0000000000000000000026',$,$,$,$,#8,#25,$,$);\n\
         {}#36=IFCSLAB('0000000000000000000036',$,$,$,$,#12,#35,$,$);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        body(20, [0.0, 0.0], [4.0, 0.2], 3.0),
        body(30, [0.0, 1.59], [3.0, 3.0], 0.2),
    )
}

/// The slab edge in the turned wall reaches metres along the world axes but
/// 10 mm across the wall's own: a 20 mm orthogonal case lets it pass.
#[test]
fn with_geometry_clash_tolerance_cases_measure_along_the_walls_axes() {
    let case = Case::new("clash-tolerance-cases");
    let check = |cases: Value| {
        let mut parameters = json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "penetration_tolerance_metres": {"type": "number", "value": 0.0},
        });
        if !cases.is_null() {
            parameters["tolerance_cases"] = json!({"type": "table", "value": cases});
        }
        case.geometry_rule(
            &slab_in_a_turned_wall(),
            &[("slab", "IfcSlab")],
            "axioval:capability.clash",
            &registry_signature("axioval:capability.clash"),
            entity("slab"),
            parameters,
        )
    };
    let orthogonal = |tolerance: f64| {
        json!([{
            "case": {"type": "string", "value": "horizontal_orthogonal"},
            "first_selector": {"type": "selector", "value": entity("slab")},
            "second_selector": {"type": "selector", "value": entity("wall")},
            "tolerance_metres": {"type": "number", "value": tolerance},
        }])
    };
    let (output, result) = check(orthogonal(0.02));
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{result:#}",
        stderr(&output)
    );
    let (output, result) = check(Value::Null);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), vec!["#36"], "{result:#}");
    let (output, _) = check(orthogonal(0.005));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
}

/// The two crossing walls, one per file, each in a system of its own file
/// named `systems.0` and `systems.1`, and on a layer named `A-WALL`.
fn walls_in_two_files(case: &Case, systems: [&str; 2]) {
    for (index, (file, (length, width))) in ["one.ifc", "two.ifc"]
        .into_iter()
        .zip([(4.0, 0.2), (0.2, 4.0)])
        .enumerate()
    {
        let extra = format!(
            "#40=IFCSYSTEM('00000000000000000000{index}0',$,'{}',$,$);\n\
             #41=IFCRELASSIGNSTOGROUP('00000000000000000000{index}1',$,$,$,(#16),$,#40);\n\
             #42=IFCPRESENTATIONLAYERASSIGNMENT('A-WALL',$,(#14),$);\n",
            systems[index]
        );
        let model = walls_file(&[(
            10,
            2.0,
            0.0,
            length,
            width,
            &format!("0000000000000000000{index}16"),
        )])
        .replace("ENDSEC;\nEND-ISO", &format!("{extra}ENDSEC;\nEND-ISO"));
        case.write(file, &model);
    }
}

/// Federated models: one system split across two files is one system when
/// the systems share a name, and a layer name shared across files is no
/// shared layer.
#[test]
fn with_geometry_clash_exclusions_match_systems_by_name_across_files() {
    let case = Case::new("clash-federated-exclusions");
    let clash = |parameters: Value| {
        let mut bound = json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "penetration_tolerance_metres": {"type": "number", "value": 0.01},
        });
        for (name, value) in parameters.as_object().unwrap() {
            bound[name] = value.clone();
        }
        case.geometry_rule_over(
            &["one.ifc", "two.ifc"],
            &[],
            "axioval:capability.clash",
            &registry_signature("axioval:capability.clash"),
            entity("wall"),
            bound,
        )
    };
    let by_name = json!({
        "exclude_paths": {"type": "stringList", "value": ["IfcRelAssignsToGroup:backward"]},
        "exclude_target_property": {"type": "propertyReference",
                                    "property": "axioval:example.ifc.name",
                                    "propertySet": "axioval:attributes"},
    });
    walls_in_two_files(&case, ["SUP-01", "SUP-01"]);
    let (output, result) = clash(by_name.clone());
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    walls_in_two_files(&case, ["SUP-01", "RET-01"]);
    let (output, result) = clash(by_name);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), vec!["#16"], "{result:#}");

    let (output, result) = clash(json!({"exclude_same_layer": {"type": "boolean", "value": true}}));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), vec!["#16"], "{result:#}");
}

#[test]
fn without_geometry_geometric_rules_are_not_evaluated_and_say_why() {
    let case = Case::new("geometry-off");
    let saved = case.path("result.json");
    let output = case.clash_check(&["--summary", "--report", saved.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let summary = stdout(&output);
    assert!(summary.contains("missing-service"), "{summary}");
    assert!(summary.contains("with --geometry"), "{summary}");
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    assert!(result.get("geometry").is_none());
}

/// A 10 m × 10 m gross-area space #16 in zone #90 `Envelope`, with walls:
/// #26 along the south edge declared internal, #36 across the middle declared
/// external, #46 along the north edge declared external, and #56 along the
/// west edge with no `IsExternal` at all.
fn envelope_model() -> String {
    let body = |first: u32, x: f64, y: f64, length: f64, width: f64, product: &str| {
        let [p, pos, profile, solid, shape, definition, object] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        format!(
            "#{p}=IFCCARTESIANPOINT(({x},{y}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length},{width});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,3.);\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{object}={product};\n",
            product = product.replace("REP", &format!("#{definition}")),
        )
    };
    let wall = |first: u32, x: f64, y: f64, length: f64, width: f64| {
        let id = first + 6;
        body(
            first,
            x,
            y,
            length,
            width,
            &format!("IFCWALL('00000000000000000000{id}',$,$,$,$,#3,REP,$,$)"),
        )
    };
    let external = |value: &str, first: u32, wall: u32| {
        let [single, set, rel] = [0, 1, 2].map(|offset| first + offset);
        format!(
            "#{single}=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN({value}),$);\n\
             #{set}=IFCPROPERTYSET('00000000000000000000{set}',$,'Pset_WallCommon',$,(#{single}));\n\
             #{rel}=IFCRELDEFINESBYPROPERTIES('00000000000000000000{rel}',$,$,$,(#{wall}),#{set});\n"
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}{}{}{}{}{}\
         #90=IFCZONE('0000000000000000000090',$,'Envelope',$,$,$);\n\
         #91=IFCRELASSIGNSTOGROUP('0000000000000000000091',$,$,$,(#16),$,#90);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        body(
            10,
            5.0,
            5.0,
            10.0,
            10.0,
            "IFCSPACE('0000000000000000000016',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)"
        ),
        wall(20, 5.0, 0.15, 10.0, 0.3),
        wall(30, 5.0, 5.0, 9.4, 0.2),
        wall(40, 5.0, 9.85, 10.0, 0.3),
        wall(50, 0.15, 5.0, 0.3, 9.4),
        external(".F.", 60, 26),
        external(".T.", 70, 36),
        external(".T.", 80, 46),
    )
}

impl Case {
    /// Runs `external-wall-validation` over the envelope model with
    /// `parameters`, its definition built from the registry's signature.
    fn envelope_check(&self, parameters: Value) -> (Output, Value) {
        self.geometry_rule(
            &envelope_model(),
            &[("space", "IfcSpace"), ("zone", "IfcZone")],
            "axioval:capability.external-wall-validation",
            &registry_signature("axioval:capability.external-wall-validation"),
            entity("wall"),
            parameters,
        )
    }
}

/// `(object, message)` of every finding, sorted.
fn sorted_findings(result: &Value) -> Vec<(String, String)> {
    let mut findings: Vec<(String, String)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                finding["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    findings.sort();
    findings
}

/// The rule selects its bounding spaces and its gross-area groups; no flag
/// names them. Both derivations run in one rule, each reported on its own.
#[test]
fn with_geometry_an_envelope_rule_selects_its_bounding_spaces() {
    let case = Case::new("geometry-envelope");
    let (output, result) = case.envelope_check(json!({
        "derivations": {"type": "stringList", "value": ["all-spaces", "gross-area-groups"]},
        "bounding_selector": {"type": "selector", "value": entity("space")},
        "gross_area_group_selector": {"type": "selector", "value": entity("zone")},
        "gross_area_group_path": {"type": "stringList",
                                  "value": ["IfcRelAssignsToGroup:forward"]},
    }));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let mut expected: Vec<(String, String)> = ["all-spaces", "gross-area-groups"]
        .iter()
        .flat_map(|derivation| {
            [
                (
                    "#26".to_owned(),
                    format!("on the {derivation} envelope but not declared external"),
                ),
                (
                    "#36".to_owned(),
                    format!("declared external but not on the {derivation} envelope"),
                ),
            ]
        })
        .collect();
    expected.sort();
    assert_eq!(sorted_findings(&result), expected, "{result:#}");
    // The undeclared wall #56 is not evaluated once per derivation.
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 2, "{result:#}");
    for outcome in not_evaluated {
        assert_eq!(outcome["object_id"]["local_id"], "#56", "{result:#}");
    }
}

/// A selection that bounds nothing leaves the derivation not evaluated, and a
/// derivation without its bounding input is an invalid declaration; neither
/// falls back to a host default.
#[test]
fn with_geometry_an_envelope_rule_without_bounding_spaces_is_not_evaluated() {
    let case = Case::new("geometry-envelope-unbounded");
    let (output, result) = case.envelope_check(json!({
        "derivations": {"type": "stringList", "value": ["all-spaces"]},
        "bounding_selector": {"type": "selector", "value": entity("zone")},
    }));
    // The zone has no body, so its region is unknown.
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");

    let (output, result) = case.envelope_check(json!({
        "derivations": {"type": "stringList", "value": ["gross-area-groups"]},
        "bounding_selector": {"type": "selector", "value": entity("space")},
    }));
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        result
            .to_string()
            .contains("needs `gross_area_group_selector`"),
        "{result:#}"
    );
}

/// The zone flag is gone: the rule states its bounding spaces.
#[test]
fn the_envelope_zone_flag_is_rejected() {
    let case = Case::new("geometry-envelope-flag");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .args(["check", "--model", "m.ifc", "--definitions", "d.json"])
        .args([
            "--ruleset",
            "r.json",
            "--geometry",
            "--envelope-zone",
            "Envelope",
        ])
        .current_dir(case.path(""))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

/// Slab #16 (4 m × 4 m, 0.2 m thick) walled in on every side by 3 m walls,
/// and slab #26 of the same size standing alone 20 m away.
fn landings() -> String {
    let body = |first: u32, x: f64, y: f64, length: f64, width: f64, depth: f64, product: &str| {
        let [p, pos, profile, solid, shape, definition, object] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        format!(
            "#{p}=IFCCARTESIANPOINT(({x},{y}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length},{width});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth});\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{object}={};\n",
            product.replace("REP", &format!("#{definition}")),
        )
    };
    let slab = |first: u32, x: f64| {
        let id = first + 6;
        body(
            first,
            x,
            2.0,
            4.0,
            4.0,
            0.2,
            &format!("IFCSLAB('00000000000000000000{id}',$,$,$,$,#3,REP,$,.LANDING.)"),
        )
    };
    let wall = |first: u32, x: f64, y: f64, length: f64, width: f64| {
        let id = first + 6;
        body(
            first,
            x,
            y,
            length,
            width,
            3.0,
            &format!("IFCWALL('00000000000000000000{id}',$,$,$,$,#3,REP,$,$)"),
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        slab(10, 2.0),
        slab(20, 22.0),
        wall(30, 2.0, 0.0, 4.2, 0.2),
        wall(40, 2.0, 4.0, 4.2, 0.2),
        wall(50, 0.0, 2.0, 0.2, 4.2),
        wall(60, 4.0, 2.0, 0.2, 4.2),
    )
}

#[test]
fn with_geometry_an_unguarded_landing_edge_is_found() {
    let case = Case::new("geometry-guard");
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    definitions["objectTypes"]["axioval:example.ifc.slab"] = json!({
        "id": "axioval:example.ifc.slab",
        "name": {"default": "Slab", "translations": {}},
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcSlab"}],
        "citations": [],
    });
    let numbers = [
        ("minimum_barrier_height_metres", 1.0),
        ("maximum_barrier_gap_metres", 0.1),
        ("maximum_platform_gap_metres", 0.1),
        ("maximum_landing_gap_metres", 0.3),
        ("maximum_fall_height_metres", 0.5),
        ("minimum_landing_width_metres", 1.0),
        ("climbable_barrier_distance_metres", 0.3),
        ("maximum_climbable_height_metres", 0.6),
        ("minimum_climbable_side_length_metres", 0.1),
    ];
    let declare = |id: &str, kind: &str| {
        json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
               "required": true, "allowedValues": [], "citations": []})
    };
    let mut parameters: serde_json::Map<String, Value> = numbers
        .iter()
        .map(|(id, _)| ((*id).to_owned(), declare(id, "number")))
        .collect();
    parameters.insert(
        "measure_barrier_from_curb".into(),
        declare("measure_barrier_from_curb", "boolean"),
    );
    for role in ["barrier_selector", "landing_selector", "climbable_selector"] {
        let mut optional = declare(role, "selector");
        optional["required"] = json!(false);
        parameters.insert(role.into(), optional);
    }
    definitions["definitions"]["axioval:example.guard"] = json!({
        "id": "axioval:example.guard",
        "name": {"default": "Fall protection", "translations": {}},
        "description": {"default": "Walking surface edges are guarded.", "translations": {}},
        "capability": "axioval:capability.horizontal-guard",
        "parameters": parameters,
        "citations": [],
        "tags": [],
    });
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("landings-guarded");
    rule["definitionId"] = json!("axioval:example.guard");
    let mut values: serde_json::Map<String, Value> = numbers
        .iter()
        .map(|(id, value)| ((*id).to_owned(), json!({"type": "number", "value": value})))
        .collect();
    values.insert(
        "measure_barrier_from_curb".into(),
        json!({"type": "boolean", "value": false}),
    );
    rule["parameters"] = values.into();
    rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
        json!("axioval:example.ifc.slab");
    let model = case.write("model.ifc", &landings());
    let definitions = case.write("definitions.json", &definitions.to_string());
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    assert!(!findings.is_empty(), "{result:#}");
    assert!(
        findings
            .iter()
            .all(|finding| finding["object_id"]["local_id"] == "#26"),
        "only the bare slab is unguarded: {result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// A 4 m deep box centred at `x`, as instances `#first` to `#first + 6`;
/// `REP` in `product` becomes its shape.
fn body(first: u32, x: f64, length: f64, depth: f64, product: &str) -> String {
    let [p, pos, profile, solid, shape, definition, object] =
        [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
    format!(
        "#{p}=IFCCARTESIANPOINT(({x},2.));\n\
         #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
         #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length},4.);\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        product.replace("REP", &format!("#{definition}")),
    )
}

/// Spaces #16 (x 0..4) and #26 (x 3..7) in zone #90, and space #46
/// (x 20..24) outside it. With `bodiless_member`, the zone also groups
/// space #50, which has no body.
fn spaces_in_a_zone(bodiless_member: bool) -> String {
    let space = |first: u32, x: f64| {
        body(
            first,
            x,
            4.0,
            3.0,
            &format!(
                "IFCSPACE('00000000000000000000{:02}',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)",
                first + 6
            ),
        )
    };
    let members = if bodiless_member {
        "(#16,#26,#50)"
    } else {
        "(#16,#26)"
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}\
         #50=IFCSPACE('0000000000000000000050',$,$,$,$,#3,$,$,.ELEMENT.,$,$);\n\
         #90=IFCZONE('0000000000000000000090',$,'Compartment',$,$,$);\n\
         #91=IFCRELASSIGNSTOGROUP('0000000000000000000091',$,$,$,{members},$,#90);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        space(10, 2.0),
        space(20, 5.0),
        space(40, 22.0),
    )
}

/// Space #16 lies wholly on slab #36; space #26 only half.
fn spaces_on_a_slab() -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        body(
            10,
            2.0,
            4.0,
            3.0,
            "IFCSPACE('0000000000000000000016',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)"
        ),
        body(
            20,
            5.0,
            4.0,
            3.0,
            "IFCSPACE('0000000000000000000026',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)"
        ),
        body(
            30,
            2.0,
            4.0,
            0.2,
            "IFCSLAB('0000000000000000000036',$,$,$,$,#3,REP,$,.FLOOR.)"
        ),
    )
}

/// Runs `plan-coverage` of spaces against `candidate` objects over `model`
/// with geometry, returning the output and the saved result.
fn plan_coverage(name: &str, model: &str, candidate: (&str, &str)) -> (Output, Value) {
    let case = Case::new(name);
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    for (id, name) in [("space", "IfcSpace"), candidate] {
        definitions["objectTypes"][format!("axioval:example.ifc.{id}")] = json!({
            "id": format!("axioval:example.ifc.{id}"),
            "name": {"default": name, "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
            "citations": [],
        });
    }
    let declare = |id: &str, kind: &str, required: bool| {
        json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
               "required": required, "allowedValues": [], "citations": []})
    };
    definitions["definitions"]["axioval:example.coverage"] = json!({
        "id": "axioval:example.coverage",
        "name": {"default": "Coverage", "translations": {}},
        "description": {"default": "Spaces lie within a candidate.", "translations": {}},
        "capability": "axioval:capability.plan-coverage",
        "parameters": {
            "candidate_selector": declare("candidate_selector", "selector", true),
            "minimum_ratio": declare("minimum_ratio", "number", true),
            "relationship": declare("relationship", "string", false),
            "direction": declare("direction", "string", false),
            "follow_chain": declare("follow_chain", "boolean", false),
            "path": declare("path", "stringList", false),
            "skip_absent_relationship_ends":
                declare("skip_absent_relationship_ends", "boolean", false),
        },
        "citations": [],
        "tags": [],
    });
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("spaces-covered");
    rule["definitionId"] = json!("axioval:example.coverage");
    rule["parameters"] = json!({
        "candidate_selector": {"type": "selector", "value": {
            "kind": "entityType", "objectType": format!("axioval:example.ifc.{}", candidate.0),
            "includeSubtypes": true}},
        "minimum_ratio": {"type": "number", "value": 0.9},
    });
    rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
        json!("axioval:example.ifc.space");
    let model = case.write("model.ifc", model);
    let definitions = case.write("definitions.json", &definitions.to_string());
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    let result = std::fs::read_to_string(&saved)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    (output, result)
}

#[test]
fn with_geometry_plan_coverage_measures_real_footprints() {
    let (output, result) = plan_coverage(
        "geometry-plan-coverage",
        &spaces_on_a_slab(),
        ("slab", "IfcSlab"),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#26", "{result:#}");
}

#[test]
fn with_geometry_plan_coverage_measures_a_zone_as_the_union_of_its_spaces() {
    let (output, result) = plan_coverage(
        "geometry-plan-coverage-zone",
        &spaces_in_a_zone(false),
        ("zone", "IfcZone"),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #16 and #26 lie within the zone they make up; #46 lies outside it.
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#46", "{result:#}");
    let not_evaluated: Vec<&Value> = result["report"]["not_evaluated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["object_id"]["local_id"])
        .collect();
    // Only the bodiless space #50, which has no footprint of its own.
    assert_eq!(not_evaluated, [&json!("#50")], "{result:#}");
}

#[test]
fn with_geometry_a_zone_with_a_bodiless_member_has_no_footprint() {
    let (output, result) = plan_coverage(
        "geometry-plan-coverage-zone-bodiless",
        &spaces_in_a_zone(true),
        ("zone", "IfcZone"),
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        result["report"]["findings"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 4, "{result:#}");
    assert!(
        not_evaluated
            .iter()
            .filter(|entry| entry["object_id"]["local_id"] != "#50")
            .all(|entry| entry["message"]
                .as_str()
                .is_some_and(|message| message.contains("has no body"))),
        "{result:#}"
    );
}

/// Runs `plan-area` over `model` with geometry on the `subject` objects,
/// with `parameters`; spaces are the declared member type. Returns the
/// output and the saved result.
fn plan_area(
    name: &str,
    model: &str,
    subject: (&str, &str),
    parameters: &Value,
) -> (Output, Value) {
    plan_area_with(name, model, subject, parameters, &json!({}))
}

/// [`plan_area`] with `extra` fields on the rule, such as `severityBands`.
fn plan_area_with(
    name: &str,
    model: &str,
    subject: (&str, &str),
    parameters: &Value,
    extra: &Value,
) -> (Output, Value) {
    let case = Case::new(name);
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    for (id, name) in [("space", "IfcSpace"), subject] {
        definitions["objectTypes"][format!("axioval:example.ifc.{id}")] = json!({
            "id": format!("axioval:example.ifc.{id}"),
            "name": {"default": name, "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
            "citations": [],
        });
    }
    let declare = |id: &str, kind: &str| {
        json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
               "required": false, "allowedValues": [], "citations": []})
    };
    definitions["definitions"]["axioval:example.area"] = json!({
        "id": "axioval:example.area",
        "name": {"default": "Area", "translations": {}},
        "description": {"default": "Plan areas lie within a range.", "translations": {}},
        "capability": "axioval:capability.plan-area",
        "parameters": {
            "minimum": declare("minimum", "number"),
            "maximum": declare("maximum", "number"),
            "member_selector": declare("member_selector", "selector"),
            "measure": declare("measure", "string"),
            "relationship": declare("relationship", "string"),
            "direction": declare("direction", "string"),
            "follow_chain": declare("follow_chain", "boolean"),
            "path": declare("path", "stringList"),
            "skip_absent_relationship_ends": declare("skip_absent_relationship_ends", "boolean"),
        },
        "citations": [],
        "tags": [],
    });
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("areas-bounded");
    rule["definitionId"] = json!("axioval:example.area");
    rule["parameters"] = parameters.clone();
    rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
        json!(format!("axioval:example.ifc.{}", subject.0));
    for (field, value) in extra.as_object().into_iter().flatten() {
        rule[field] = value.clone();
    }
    let model = case.write("model.ifc", model);
    let definitions = case.write("definitions.json", &definitions.to_string());
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    let result = std::fs::read_to_string(&saved)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    (output, result)
}

fn flagged_objects(result: &Value, entries: &str) -> Vec<String> {
    let mut objects: Vec<String> = result["report"][entries]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|entry| entry["object_id"]["local_id"].as_str().unwrap().to_owned())
        .collect();
    objects.sort();
    objects
}

#[test]
fn with_geometry_plan_area_bounds_each_space_boundary_included() {
    // Every space with a body measures 4 m x 4 m = 16 m².
    let number = |value: f64| json!({"type": "number", "value": value});
    let (output, result) = plan_area(
        "geometry-plan-area-boundary",
        &spaces_in_a_zone(false),
        ("space", "IfcSpace"),
        &json!({"minimum": number(16.0), "maximum": number(16.0)}),
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        flagged_objects(&result, "findings").is_empty(),
        "{result:#}"
    );
    // Only the bodiless space #50, which has no footprint to measure.
    assert_eq!(
        flagged_objects(&result, "not_evaluated"),
        ["#50"],
        "{result:#}"
    );

    let (output, result) = plan_area(
        "geometry-plan-area-exceeded",
        &spaces_in_a_zone(false),
        ("space", "IfcSpace"),
        &json!({"maximum": number(15.99)}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        flagged_objects(&result, "findings"),
        ["#16", "#26", "#46"],
        "{result:#}"
    );
    assert_eq!(
        result["report"]["findings"][0]["message"], "plan area is 16 m²; required at most 15.99 m²",
        "{result:#}"
    );
}

#[test]
fn with_geometry_severity_bands_grade_an_area_by_how_far_it_misses() {
    // Every space with a body measures 16 m²: 0.06 % over 15.99 m², 6.7 %
    // over 15 m².
    let number = |value: f64| json!({"type": "number", "value": value});
    let bands = json!({"severityBands": [
        {"below": 0.05, "severity": "info"},
        {"below": 0.1, "severity": "warning"},
    ]});
    let severities = |result: &Value| -> Vec<String> {
        result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["severity"].as_str().unwrap().to_owned())
            .collect()
    };
    for (name, maximum, severity) in [
        ("geometry-plan-area-band-info", 15.99, "info"),
        ("geometry-plan-area-band-warning", 15.0, "warning"),
    ] {
        let (output, result) = plan_area_with(
            name,
            &spaces_in_a_zone(false),
            ("space", "IfcSpace"),
            &json!({"maximum": number(maximum)}),
            &bands,
        );
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        assert_eq!(severities(&result), [severity; 3], "{result:#}");
    }
}

#[test]
fn with_geometry_plan_area_sums_the_members_of_a_zone() {
    // Spaces #16 and #26 of zone #90 measure 16 m² each; overlapping
    // members count twice in a sum.
    let parameters = |maximum: f64| {
        json!({
            "member_selector": {"type": "selector", "value": {
                "kind": "entityType", "objectType": "axioval:example.ifc.space",
                "includeSubtypes": true}},
            "relationship": {"type": "string", "value": "IfcRelAssignsToGroup"},
            "maximum": {"type": "number", "value": maximum},
        })
    };
    let (output, result) = plan_area(
        "geometry-plan-area-zone-boundary",
        &spaces_in_a_zone(false),
        ("zone", "IfcZone"),
        &parameters(32.0),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{result:#}",
        stderr(&output)
    );

    let (output, result) = plan_area(
        "geometry-plan-area-zone-exceeded",
        &spaces_in_a_zone(false),
        ("zone", "IfcZone"),
        &parameters(31.0),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(flagged_objects(&result, "findings"), ["#90"], "{result:#}");
    assert_eq!(
        result["report"]["findings"][0]["message"],
        "summed plan area of the members is 32 m²; required at most 31 m²",
        "{result:#}"
    );

    // A member without a body leaves the sum unknown.
    let (output, result) = plan_area(
        "geometry-plan-area-zone-bodiless",
        &spaces_in_a_zone(true),
        ("zone", "IfcZone"),
        &parameters(32.0),
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(
        flagged_objects(&result, "not_evaluated"),
        ["#90"],
        "{result:#}"
    );
}

/// Slabs #16, #26 and #36, 0.2 m thick, stacked at 0, 3 and 6.5 m.
fn stacked_slabs() -> String {
    let slab = |first: u32, elevation: f64| {
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
            "#{origin}=IFCCARTESIANPOINT((0.,0.,{elevation:.1}));\n\
             #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
             #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
             #{p}=IFCCARTESIANPOINT((5.,4.));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},10.,8.);\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,0.2);\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{object}=IFCSLAB('00000000000000000000{object:02}',$,$,$,$,#{placement},#{definition},$,.FLOOR.);\n",
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        slab(7, 0.0),
        slab(17, 3.0),
        slab(27, 6.5),
    )
}

#[test]
fn with_geometry_slab_stack_spacing_measures_real_elevations() {
    let case = Case::new("geometry-slab-stack");
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    definitions["objectTypes"]["axioval:example.ifc.slab"] = json!({
        "id": "axioval:example.ifc.slab",
        "name": {"default": "IfcSlab", "translations": {}},
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcSlab"}],
        "citations": [],
    });
    let declare = |id: &str, kind: &str, required: bool| {
        json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
               "required": required, "allowedValues": [], "citations": []})
    };
    definitions["definitions"]["axioval:example.slab-stack"] = json!({
        "id": "axioval:example.slab-stack",
        "name": {"default": "Slab stack", "translations": {}},
        "description": {"default": "Storeys rise at most 3.2 m.", "translations": {}},
        "capability": "axioval:capability.slab-stack-spacing",
        "parameters": {
            "minimum_overlap_ratio": declare("minimum_overlap_ratio", "number", true),
            "top_to_top_minimum": declare("top_to_top_minimum", "quantity", false),
            "top_to_top_maximum": declare("top_to_top_maximum", "quantity", false),
            "bottom_to_bottom_minimum": declare("bottom_to_bottom_minimum", "quantity", false),
            "bottom_to_bottom_maximum": declare("bottom_to_bottom_maximum", "quantity", false),
            "top_to_bottom_minimum": declare("top_to_bottom_minimum", "quantity", false),
            "top_to_bottom_maximum": declare("top_to_bottom_maximum", "quantity", false),
            "consistent": declare("consistent", "stringList", false),
            "tolerance": declare("tolerance", "quantity", false),
        },
        "citations": [],
        "tags": [],
    });
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("slab-stack");
    rule["definitionId"] = json!("axioval:example.slab-stack");
    rule["parameters"] = json!({
        "minimum_overlap_ratio": {"type": "number", "value": 0.5},
        "top_to_top_maximum": {"type": "quantity", "value": 3.2, "unit": "m"},
    });
    rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
        json!("axioval:example.ifc.slab");
    let model = case.write("model.ifc", &stacked_slabs());
    let definitions = case.write("definitions.json", &definitions.to_string());
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#26", "{result:#}");
}

#[test]
fn an_empty_selection_is_a_source_finding_in_json_summary_listing_and_bcf() {
    let case = Case::new("object-count");
    let (definitions, ruleset) = case.count_packages();
    let model = case.write("model.ifc", &ifc("0000000000000000000002", true));
    let saved = case.path("result.json");
    let bcf = case.path("issues.bcfzip");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--summary", "--report", saved.to_str().unwrap()])
        .args(["--bcf", bcf.to_str().unwrap()])
        .env("SOURCE_DATE_EPOCH", "1790416800")
        .output()
        .unwrap();
    // No space is not a pass: a finding against the model itself.
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert!(findings[0].get("object_id").is_none(), "{result:#}");
    assert_eq!(findings[0]["source"]["document"], "model.ifc");
    assert!(
        findings[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("no object matches the selection in source"),
        "{result:#}"
    );

    let summary = stdout(&output);
    assert!(
        summary.starts_with("status: findings · 1 finding(s)"),
        "{summary}"
    );
    assert!(summary.contains("spaces-exist"), "{summary}");
    assert!(
        !summary.contains("e.g."),
        "no object to give as an example: {summary}"
    );

    let saved = saved.to_str().unwrap();
    let listing = report(&[saved, "--rule", "spaces-exist", "--json"]);
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    let listing: Value = serde_json::from_slice(&listing.stdout).unwrap();
    let entry = &listing["entries"][0];
    assert!(entry.get("object").is_none(), "{listing:#}");
    assert_eq!(entry["scope"], "source model.ifc");
    let text = stdout(&report(&[saved, "--rule", "spaces-exist"]));
    assert!(
        text.contains("[finding] error spaces-exist  (source model.ifc)"),
        "{text}"
    );
    let by_source = report(&[saved, "--object", "model.ifc", "--json"]);
    let by_source: Value = serde_json::from_slice(&by_source.stdout).unwrap();
    assert_eq!(by_source["total"], 1, "{by_source:#}");

    let archive = openbim_bcf::read_path(&bcf).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );
    let topic = &archive.topics().next().unwrap().topic;
    assert_eq!(topic.labels, ["spaces-exist", "example"]);
    assert!(
        topic
            .description
            .as_deref()
            .unwrap()
            .contains("Source: ifc-step:model.ifc; no single object"),
        "{:?}",
        topic.description
    );
}

impl Case {
    /// The fixture packages with the rule swapped for "the model has a space".
    fn count_packages(&self) -> (PathBuf, PathBuf) {
        let definitions = self.definitions(true);
        let mut definitions: Value =
            serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
        definitions["objectTypes"]["axioval:example.ifc.space"] = json!({
            "id": "axioval:example.ifc.space",
            "name": {"default": "Space", "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcSpace"}],
            "citations": [],
        });
        let parameter = |id: &str, kind: &str| {
            json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
               "required": false, "allowedValues": [], "citations": []})
        };
        definitions["definitions"]["axioval:example.count"] = json!({
            "id": "axioval:example.count",
            "name": {"default": "Object count", "translations": {}},
            "description": {"default": "The model has spaces.", "translations": {}},
            "capability": "axioval:capability.object-count",
            "parameters": {
                "minimum": parameter("minimum", "integer"),
                "maximum": parameter("maximum", "integer"),
                "across_sources": parameter("across_sources", "boolean"),
                "disciplines": parameter("disciplines", "stringList"),
            },
            "citations": [],
            "tags": [],
        });
        let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["id"] = json!("spaces-exist");
        rule["definitionId"] = json!("axioval:example.count");
        rule["parameters"] = json!({});
        rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
            json!("axioval:example.ifc.space");
        (
            self.write("definitions.json", &definitions.to_string()),
            self.write("ruleset.json", &ruleset.to_string()),
        )
    }
}

#[test]
fn an_object_count_limited_to_a_discipline_judges_only_its_models() {
    let case = Case::new("object-count-disciplines");
    let (definitions, ruleset) = case.count_packages();
    let mut rules: Value =
        serde_json::from_str(&std::fs::read_to_string(&ruleset).unwrap()).unwrap();
    rules["root"]["rules"][0]["parameters"] =
        json!({"disciplines": {"type": "stringList", "value": ["mep"]}});
    let ruleset = case.write("ruleset.json", &rules.to_string());
    for name in ["arch.ifc", "mep.ifc", "loose.ifc"] {
        case.write(name, &ifc("0000000000000000000002", true));
    }
    let run = |models: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
        command.current_dir(&case.dir).arg("check");
        for model in models {
            command.arg("--model").arg(model);
        }
        command
            .arg("--definitions")
            .arg(&definitions)
            .arg("--ruleset")
            .arg(&ruleset)
            .output()
            .unwrap()
    };
    // Neither has a space; only the MEP model is judged.
    let output = run(&["arch.ifc:architecture", "mep.ifc:mep"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["source"]["document"], "mep.ifc");
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");

    // A model without a discipline may or may not count: never a pass.
    let output = run(&["arch.ifc:architecture", "loose.ifc"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(
        result["report"]["not_evaluated"][0]["source"]["document"], "loose.ifc",
        "{result:#}"
    );
}

/// Rooms #16 (x 0..4) and #26 (x 4.2..8.2) behind a 0.2 m wall, with no
/// spatial containment stated anywhere. Furniture #36 and #46 stand in #16,
/// #56 in #26. Door #66 sits in the shared wall, door #76 in the facade at
/// x -0.2..0, and opening #86 is a void through the shared wall.
fn rooms_without_containment() -> String {
    let product = |first: u32, x: f64, length: f64, height: f64, entity: &str| {
        body(
            first,
            x,
            length,
            height,
            &entity.replace("GID", &format!("00000000000000000000{:02}", first + 6)),
        )
    };
    let space = "IFCSPACE('GID',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)";
    let furniture = "IFCFURNISHINGELEMENT('GID',$,$,$,$,#3,REP,$)";
    let door = "IFCDOOR('GID',$,$,$,$,#3,REP,$,2.1,4.,$,$,$)";
    let opening = "IFCOPENINGELEMENT('GID',$,$,$,$,#3,REP,$,.OPENING.)";
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}{}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        product(10, 2.0, 4.0, 3.0, space),
        product(20, 6.2, 4.0, 3.0, space),
        product(30, 1.0, 0.5, 0.9, furniture),
        product(40, 3.0, 0.5, 0.9, furniture),
        product(50, 7.0, 0.5, 0.9, furniture),
        product(60, 4.1, 0.1, 2.1, door),
        product(70, -0.1, 0.1, 2.1, door),
        product(80, 4.1, 0.2, 2.1, opening),
    )
}

/// Runs one `related-count` rule with geometry over
/// [`rooms_without_containment`]: each `anchor` object has at least
/// `minimum` `related` objects reached through `relationship`.
fn derived_count(
    name: &str,
    anchor: (&str, &str),
    related: (&str, &str),
    relationship: &str,
    direction: &str,
    minimum: i64,
) -> (Output, Value) {
    derived_count_in(
        &rooms_without_containment(),
        name,
        anchor,
        related,
        (relationship, direction),
        minimum,
    )
}

/// [`derived_count`] over `model`.
fn derived_count_in(
    model: &str,
    name: &str,
    anchor: (&str, &str),
    related: (&str, &str),
    (relationship, direction): (&str, &str),
    minimum: i64,
) -> (Output, Value) {
    let case = Case::new(name);
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    for (id, name) in [anchor, related] {
        definitions["objectTypes"][format!("axioval:example.ifc.{id}")] = json!({
            "id": format!("axioval:example.ifc.{id}"),
            "name": {"default": name, "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
            "citations": [],
        });
    }
    let declare = |id: &str, kind: &str| {
        json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
               "required": false, "allowedValues": [], "citations": []})
    };
    definitions["definitions"]["axioval:example.related-count"] = json!({
        "id": "axioval:example.related-count",
        "name": {"default": "Related count", "translations": {}},
        "description": {"default": "Each anchor has enough related objects.", "translations": {}},
        "capability": "axioval:capability.related-count",
        "parameters": {
            "related_selector": declare("related_selector", "selector"),
            "minimum": declare("minimum", "integer"),
            "maximum": declare("maximum", "integer"),
            "same_ends": declare("same_ends", "stringList"),
            "relationship": declare("relationship", "string"),
            "direction": declare("direction", "string"),
            "follow_chain": declare("follow_chain", "boolean"),
            "path": declare("path", "stringList"),
            "skip_absent_relationship_ends":
                declare("skip_absent_relationship_ends", "boolean"),
        },
        "citations": [],
        "tags": [],
    });
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("derived-count");
    rule["definitionId"] = json!("axioval:example.related-count");
    rule["parameters"] = json!({
        "related_selector": {"type": "selector", "value": {
            "kind": "entityType", "objectType": format!("axioval:example.ifc.{}", related.0),
            "includeSubtypes": true}},
        "minimum": {"type": "integer", "value": minimum},
        "relationship": {"type": "string", "value": relationship},
        "direction": {"type": "string", "value": direction},
    });
    rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
        json!(format!("axioval:example.ifc.{}", anchor.0));
    let model = case.write("model.ifc", model);
    let definitions = case.write("definitions.json", &definitions.to_string());
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    let result = std::fs::read_to_string(&saved)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    (output, result)
}

fn finding_ids(result: &Value) -> Vec<String> {
    result["report"]["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .map(|finding| {
                    finding["object_id"]["local_id"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn with_geometry_components_are_counted_in_the_space_that_contains_them() {
    let (output, result) = derived_count(
        "geometry-derived-contained",
        ("space", "IfcSpace"),
        ("furniture", "IfcFurnishingElement"),
        "axioval:derived.contained-in-space",
        "backward",
        2,
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #16 holds two pieces of furniture; #26 only one.
    assert_eq!(finding_ids(&result), ["#26"], "{result:#}");
    let finding = &result["report"]["findings"][0];
    assert!(
        finding["message"]
            .as_str()
            .unwrap()
            .starts_with("1 related object(s)"),
        "{result:#}"
    );
    assert!(
        finding["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["locator"]
                .as_str()
                .unwrap()
                .starts_with("axioval:derived.contained-in-space")),
        "{result:#}"
    );
}

#[test]
fn with_geometry_a_door_connects_two_spaces_and_an_exit_one() {
    let (output, result) = derived_count(
        "geometry-derived-adjacent",
        ("door", "IfcDoor"),
        ("space", "IfcSpace"),
        "axioval:derived.adjacent-space",
        "forward",
        2,
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The internal door #66 reaches both rooms; the exit #76 only #16.
    assert_eq!(finding_ids(&result), ["#76"], "{result:#}");
    let evidence = result["report"]["findings"][0]["evidence"]
        .as_array()
        .unwrap();
    assert!(
        evidence.iter().any(|item| {
            let locator = item["locator"].as_str().unwrap();
            locator.contains("/#76:side=-") && locator.contains(":outside")
        }),
        "{result:#}"
    );
}

/// Spaces #16 (x 0..4) and #26 (x 4.2..8.2), wall #36 between them
/// (x 4..4.2) and wall #46 along the outside of #16 (x -0.2..0), with no
/// space boundaries stated.
fn rooms_between_walls() -> String {
    let product = |first: u32, x: f64, length: f64, entity: &str| {
        body(
            first,
            x,
            length,
            3.0,
            &entity.replace("GID", &format!("00000000000000000000{:02}", first + 6)),
        )
    };
    let space = "IFCSPACE('GID',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)";
    let wall = "IFCWALL('GID',$,$,$,$,#3,REP,$,$)";
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        product(10, 2.0, 4.0, space),
        product(20, 6.2, 4.0, space),
        product(30, 4.1, 0.2, wall),
        product(40, -0.1, 0.2, wall),
    )
}

#[test]
fn with_geometry_a_wall_without_stated_boundaries_relates_to_the_spaces_beside_it() {
    let (output, result) = derived_count_in(
        &rooms_between_walls(),
        "geometry-derived-across",
        ("wall", "IfcWall"),
        ("space", "IfcSpace"),
        ("axioval:derived.adjacent-across", "forward"),
        2,
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The wall between the rooms reaches both; the outer wall only #16.
    assert_eq!(finding_ids(&result), ["#46"], "{result:#}");
    let evidence = result["report"]["findings"][0]["evidence"]
        .as_array()
        .unwrap();
    assert!(
        evidence.iter().any(|item| {
            let locator = item["locator"].as_str().unwrap();
            locator.contains("/#46->") && locator.contains("/#16:side=+")
        }),
        "{result:#}"
    );
}

/// The compartments of [`rooms_between_walls`] when the wall between the
/// rooms states `rating` as its `FireRating`: compartments larger than
/// `maximum` square metres are found.
fn compartments(name: &str, rating: Option<&str>, maximum: f64) -> (Output, Value) {
    compartments_of(name, rooms_between_walls(), rating, maximum)
}

/// [`compartments`] over `model`.
fn compartments_of(
    name: &str,
    mut model: String,
    rating: Option<&str>,
    maximum: f64,
) -> (Output, Value) {
    let case = Case::new(name);
    if let Some(rating) = rating {
        model = model.replace(
            "ENDSEC;\nEND-ISO-10303-21;",
            &format!(
                "#100=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('{rating}'),$);\n\
                 #101=IFCPROPERTYSET('0000000000000000000101',$,'Pset_WallCommon',$,(#100));\n\
                 #102=IFCRELDEFINESBYPROPERTIES('0000000000000000000102',$,$,$,(#36),#101);\n\
                 ENDSEC;\nEND-ISO-10303-21;"
            ),
        );
    }
    case.write("model.ifc", &model);
    let boundary = json!({"kind": "property",
        "propertySet": "axioval:example.ifc.pset-wall-common",
        "property": "axioval:example.ifc.fire-rating",
        "operator": "like", "value": {"type": "string", "value": "EI*"}});
    case.geometry_rule_with(
        &["model.ifc"],
        &[("space", "IfcSpace"), ("wall", "IfcWall")],
        (
            "axioval:capability.keyed-limit",
            &registry_signature("axioval:capability.keyed-limit"),
        ),
        json!({"kind": "derivedGroup", "grouping": "compartments"}),
        json!({
            "limits": {"type": "table", "value": [
                {"maximum": {"type": "number", "value": maximum}},
            ]},
            "quantity": {"type": "string", "value": "plan-area"},
            "key_1": {"type": "propertyReference", "propertySet": "axioval:group",
                      "property": "key"},
        }),
        &json!({"ruleset:groupings": {"compartments": {
            "id": "compartments",
            "name": {"default": "Fire compartments", "translations": {}},
            "members": entity("space"),
            "by": {"kind": "compartment", "separators": entity("wall"), "boundary": boundary},
        }}}),
        &[],
    )
}

#[test]
fn with_geometry_fire_rated_walls_enclose_derived_compartments() {
    // The rated wall separates the rooms: two compartments of 16 m² each,
    // within 20 m² and both beyond 15 m².
    let (output, result) = compartments("geometry-compartments-rated", Some("EI 90"), 20.0);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");
    let (output, result) = compartments("geometry-compartments-small", Some("EI 90"), 15.0);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_ids(&result),
        [
            "axioval:group/compartments/#16",
            "axioval:group/compartments/#26"
        ],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    // A plain wall joins them: one compartment of 32 m², too large.
    let (output, result) = compartments("geometry-compartments-plain", None, 20.0);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_ids(&result),
        ["axioval:group/compartments/#16"],
        "{result:#}"
    );
    // A room exactly the tolerance away from the plain wall may or may not
    // be joined to the other: no compartment is decided, none is guessed.
    let straddling = rooms_between_walls().replace(
        "#20=IFCCARTESIANPOINT((6.2,2.));",
        "#20=IFCCARTESIANPOINT((6.25,2.));",
    );
    let (output, result) =
        compartments_of("geometry-compartments-undecided", straddling, None, 20.0);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(
        open.iter().any(|outcome| outcome["message"]
            .as_str()
            .unwrap()
            .contains("may join are unknown")),
        "{result:#}"
    );
}

#[test]
fn with_geometry_an_opening_void_connects_the_spaces_it_passes_between() {
    let (output, result) = derived_count(
        "geometry-derived-opening",
        ("opening", "IfcOpeningElement"),
        ("space", "IfcSpace"),
        "axioval:derived.adjacent-space",
        "forward",
        2,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");
}

#[test]
fn with_geometry_a_malformed_derived_relationship_is_not_evaluated() {
    let (output, result) = derived_count(
        "geometry-derived-malformed",
        ("space", "IfcSpace"),
        ("furniture", "IfcFurnishingElement"),
        "axioval:derived.contained-in-space;horizontal=-1",
        "backward",
        1,
    );
    // An invalid request, never an empty count and a finding.
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");
}

/// A capability's full parameter signature as the registry declares it, so a
/// definition keeps compiling when the capability gains optional parameters.
/// A table's columns are declared too; a quantity column still needs its
/// `unitDimension`.
fn registry_signature(capability: &str) -> Value {
    let registry = axioval::default_registry().unwrap();
    registry
        .get(capability)
        .unwrap()
        .parameters()
        .into_iter()
        .map(|parameter| {
            let id = parameter.name.clone();
            let mut declared = json!({"id": id, "name": {"default": id, "translations": {}},
                       "kind": parameter.parameter_type.package_kind(),
                       "required": parameter.required, "allowedValues": [], "citations": []});
            if let axioval::engine::ParameterType::Table(columns) = parameter.parameter_type {
                declared["columns"] = columns
                    .iter()
                    .map(|column| {
                        json!({"id": column.id,
                               "name": {"default": column.id, "translations": {}},
                               "kind": column.kind.as_str(), "required": column.required})
                    })
                    .collect();
            }
            (id, declared)
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn signature(parameters: &[(&str, &str, bool)]) -> Value {
    parameters
        .iter()
        .map(|(id, kind, required)| {
            (
                (*id).to_owned(),
                json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
                       "required": required, "allowedValues": [], "citations": []}),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

impl Case {
    /// Runs one rule of `capability` with geometry over `model`: the
    /// fixture packages with the rule swapped for it. `types` binds object
    /// types by `(id suffix, IFC name)`; `IsExternal`,
    /// `SprinklerProtection`, `TotalThickness`, `Access.ClearWidth` and
    /// `Name` (`axioval:example.ifc.name`) are always bound.
    fn geometry_rule(
        &self,
        model: &str,
        types: &[(&str, &str)],
        capability: &str,
        signature: &Value,
        applicability: Value,
        parameters: Value,
    ) -> (Output, Value) {
        self.write("model.ifc", model);
        self.geometry_rule_over(
            &["model.ifc"],
            types,
            capability,
            signature,
            applicability,
            parameters,
        )
    }

    /// As [`Case::geometry_rule`], over the files `models` names in the
    /// case directory, each `PATH[:DISCIPLINE]` as `--model` takes it.
    fn geometry_rule_over(
        &self,
        models: &[&str],
        types: &[(&str, &str)],
        capability: &str,
        signature: &Value,
        applicability: Value,
        parameters: Value,
    ) -> (Output, Value) {
        self.geometry_rule_with(
            models,
            types,
            (capability, signature),
            applicability,
            parameters,
            &json!({}),
            &[],
        )
    }

    /// As [`Case::geometry_rule_over`], with `extra` fields on the rule
    /// (such as `severityOverrides`) and further `check` arguments.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn geometry_rule_with(
        &self,
        models: &[&str],
        types: &[(&str, &str)],
        (capability, signature): (&str, &Value),
        applicability: Value,
        parameters: Value,
        extra: &Value,
        args: &[&str],
    ) -> (Output, Value) {
        let definitions = self.definitions(true);
        let mut definitions: Value =
            serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
        for (id, name) in types {
            definitions["objectTypes"][format!("axioval:example.ifc.{id}")] = json!({
                "id": format!("axioval:example.ifc.{id}"),
                "name": {"default": name, "translations": {}},
                "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
                "citations": [],
            });
        }
        definitions["propertySets"]["axioval:example.ifc.pset-space-common"] = json!({
            "id": "axioval:example.ifc.pset-space-common",
            "name": {"default": "Pset_SpaceCommon", "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Pset_SpaceCommon"}],
            "citations": [],
        });
        definitions["propertySets"]["axioval:example.ifc.pset-space-fire-safety"] = json!({
            "id": "axioval:example.ifc.pset-space-fire-safety",
            "name": {"default": "Pset_SpaceFireSafetyRequirements", "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM,
                               "name": "Pset_SpaceFireSafetyRequirements"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.sprinkler-protection"] = json!({
            "id": "axioval:example.ifc.sprinkler-protection",
            "name": {"default": "SprinklerProtection", "translations": {}},
            "valueKind": "boolean",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "SprinklerProtection"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.fire-rating"] = json!({
            "id": "axioval:example.ifc.fire-rating",
            "name": {"default": "FireRating", "translations": {}},
            "valueKind": "string",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "FireRating"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.is-external"] = json!({
            "id": "axioval:example.ifc.is-external",
            "name": {"default": "IsExternal", "translations": {}},
            "valueKind": "boolean",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IsExternal"}],
            "citations": [],
        });
        definitions["propertySets"]["axioval:example.ifc.pset-access"] = json!({
            "id": "axioval:example.ifc.pset-access",
            "name": {"default": "Access", "translations": {}},
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Access"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.clear-width"] = json!({
            "id": "axioval:example.ifc.clear-width",
            "name": {"default": "ClearWidth", "translations": {}},
            "valueKind": "quantity",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "ClearWidth"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.total-thickness"] = json!({
            "id": "axioval:example.ifc.total-thickness",
            "name": {"default": "TotalThickness", "translations": {}},
            "valueKind": "quantity",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "TotalThickness"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.elevation"] = json!({
            "id": "axioval:example.ifc.elevation",
            "name": {"default": "Elevation", "translations": {}},
            "valueKind": "quantity",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Elevation"}],
            "citations": [],
        });
        definitions["properties"]["axioval:example.ifc.body-count"] = json!({
            "id": "axioval:example.ifc.body-count",
            "name": {"default": "Count", "translations": {}},
            "valueKind": "integer",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Count"}],
            "citations": [],
        });
        declare_name(&mut definitions);
        declare_wall_quantities(&mut definitions);
        for (id, name) in [("name", "Name"), ("object-type", "ObjectType")] {
            definitions["properties"][format!("axioval:example.ifc.{id}")] = json!({
                "id": format!("axioval:example.ifc.{id}"),
                "name": {"default": name, "translations": {}},
                "valueKind": "string",
                "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
                "citations": [],
            });
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
        let rule = &mut ruleset["root"]["rules"][0];
        rule["id"] = json!("under-test");
        rule["definitionId"] = json!("axioval:example.under-test");
        rule["parameters"] = parameters;
        rule["applicability"]["groups"]["walls"]["selector"] = applicability;
        // A `ruleset:` field belongs to the ruleset, every other to the rule.
        let mut outer = Vec::new();
        for (field, value) in extra.as_object().into_iter().flatten() {
            match field.strip_prefix("ruleset:") {
                Some(field) => outer.push((field.to_owned(), value.clone())),
                None => rule[field] = value.clone(),
            }
        }
        for (field, value) in outer {
            ruleset[field] = value;
        }
        let definitions = self.write("definitions.json", &definitions.to_string());
        let ruleset = self.write("ruleset.json", &ruleset.to_string());
        let saved = self.path("result.json");
        let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
        command.current_dir(&self.dir).arg("check");
        for model in models {
            command.arg("--model").arg(model);
        }
        let output = command
            .arg("--definitions")
            .arg(definitions)
            .arg("--ruleset")
            .arg(ruleset)
            .args(["--geometry", "--report", saved.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        let result = std::fs::read_to_string(&saved)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        (output, result)
    }
}

fn entity(id: &str) -> Value {
    json!({"kind": "entityType", "objectType": format!("axioval:example.ifc.{id}"),
           "includeSubtypes": true})
}

/// Rooms #16 (x 0..4) and #26 (x 4.2..8.2), every product 4 m deep in y.
/// Internal wall #200 holds door #66 between the rooms and door #96 past
/// the east face of #26; external wall #201 holds window #76 in the west
/// facade of #16 and window #106 far from any room; wall #202, declaring no
/// `IsExternal`, holds door #116. The walls have no body. Each door and
/// window fills an opening of its own shape, #126 to #166.
fn walls_with_openings() -> String {
    let gid = |n: u32| format!("{n:0>22}");
    let product = |first: u32, x: f64, height: f64, entity: &str| {
        body(
            first,
            x,
            0.1,
            height,
            &entity.replace("GID", &gid(first + 6)),
        )
    };
    let door = "IFCDOOR('GID',$,$,$,$,#3,REP,$,2.1,4.,$,$,$)";
    let window = "IFCWINDOW('GID',$,$,$,$,#3,REP,$,1.2,4.,$,$,$)";
    let opening = "IFCOPENINGELEMENT('GID',$,$,$,$,#3,REP,$,.OPENING.)";
    let mut data = String::new();
    for (first, x) in [(10, 2.0), (20, 6.2)] {
        data.push_str(&body(
            first,
            x,
            4.0,
            3.0,
            &format!(
                "IFCSPACE('{}',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)",
                gid(first + 6)
            ),
        ));
    }
    // (element, its opening, x, entity, wall)
    for (first, void, x, entity, wall) in [
        (60, 120, 4.1, door, 200),
        (70, 130, -0.1, window, 201),
        (90, 140, 8.3, door, 200),
        (100, 150, 12.0, window, 201),
        (110, 160, 4.1, door, 202),
    ] {
        let height = if entity == window { 1.2 } else { 2.1 };
        data.push_str(&product(first, x, height, entity));
        data.push_str(&product(void, x, height, opening));
        let _ = writeln!(
            data,
            "#{}=IFCRELVOIDSELEMENT('{}',$,$,$,#{wall},#{});\n\
             #{}=IFCRELFILLSELEMENT('{}',$,$,$,#{},#{});",
            void + 7,
            gid(void + 7),
            void + 6,
            void + 8,
            gid(void + 8),
            void + 6,
            first + 6,
        );
    }
    for (wall, external) in [(200, Some(".F.")), (201, Some(".T.")), (202, None)] {
        let _ = writeln!(data, "#{wall}=IFCWALL('{}',$,$,$,$,#3,$,$,$);", gid(wall));
        if let Some(value) = external {
            let [single, set, rel] = [10, 20, 30].map(|offset| wall + offset);
            let _ = writeln!(
                data,
                "#{single}=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN({value}),$);\n\
                 #{set}=IFCPROPERTYSET('{}',$,'Pset_WallCommon',$,(#{single}));\n\
                 #{rel}=IFCRELDEFINESBYPROPERTIES('{}',$,$,$,(#{wall}),#{set});",
                gid(set),
                gid(rel),
            );
        }
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {data}\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn with_geometry_doors_and_windows_connect_the_spaces_their_wall_calls_for() {
    let case = Case::new("geometry-opening-spaces");
    let (output, result) = case.geometry_rule(
        &walls_with_openings(),
        &[
            ("door", "IfcDoor"),
            ("window", "IfcWindow"),
            ("space", "IfcSpace"),
        ],
        "axioval:capability.opening-spaces",
        &signature(&[
            ("host_path", "stringList", true),
            ("host_selector", "selector", true),
            ("external_property", "propertyReference", true),
            ("space_path", "stringList", true),
            ("space_selector", "selector", false),
        ]),
        json!({"kind": "anyOf", "operands": [entity("door"), entity("window")]}),
        json!({
            "host_path": {"type": "stringList",
                          "value": ["IfcRelFillsElement:backward", "IfcRelVoidsElement:backward"]},
            "host_selector": {"type": "selector", "value": entity("wall")},
            "external_property": {"type": "propertyReference",
                                  "property": "axioval:example.ifc.is-external",
                                  "propertySet": "axioval:example.ifc.pset-wall-common"},
            "space_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
            "space_selector": {"type": "selector", "value": entity("space")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #66 connects both rooms and #76 opens #16 to the outside. The internal
    // door #96 reaches #26 only, and the external window #106 no room.
    let mut flagged = finding_ids(&result);
    flagged.sort();
    assert_eq!(flagged, ["#106", "#96"], "{result:#}");
    let message = |id: &str| {
        result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["object_id"]["local_id"] == id)
            .and_then(|finding| finding["message"].as_str())
            .unwrap()
            .to_owned()
    };
    assert!(
        message("#96").starts_with(
            "relates to 1 space(s) via axioval:derived.adjacent-space; in an internal wall (#200)"
        ),
        "{result:#}"
    );
    assert!(
        message("#106").starts_with("relates to 0 space(s)")
            && message("#106").contains("in an external wall (#201)"),
        "{result:#}"
    );
    // #116's wall does not declare IsExternal: not evaluated, never guessed.
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 1, "{result:#}");
    assert_eq!(
        not_evaluated[0]["object_id"]["local_id"], "#116",
        "{result:#}"
    );
}

#[test]
fn a_related_step_through_several_relationships_climbs_from_a_door_to_its_wall() {
    let run = |case: &str, step: &str| {
        Case::new(case).geometry_rule(
            &walls_with_openings(),
            &[("door", "IfcDoor"), ("window", "IfcWindow")],
            "axioval:capability.manual-issue",
            &signature(&[
                ("title", "string", true),
                ("description", "string", false),
                ("category", "string", false),
            ]),
            json!({"kind": "allOf", "operands": [
                {"kind": "anyOf", "operands": [entity("door"), entity("window")]},
                {"kind": "related", "path": [step], "selector": entity("wall")},
            ]}),
            json!({"title": {"type": "string", "value": "in a wall"}}),
        )
    };
    // One hop through either relationship reaches only the opening, so no
    // object is selected and the rule reports that about the model.
    let (output, result) = run(
        "related-alternation-hop",
        "IfcRelFillsElement|IfcRelVoidsElement:backward",
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        result["report"]["findings"][0]["message"], "in a wall (no object matches the selection)",
        "{result:#}"
    );
    assert!(result["report"]["findings"][0]["object_id"].is_null());
    // The chain mixes them: door or window, opening, wall.
    let (output, result) = run(
        "related-alternation-chain",
        "IfcRelFillsElement|IfcRelVoidsElement:backward+",
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // A manual issue names the first selected object and relates the rest.
    let finding = &result["report"]["findings"][0];
    let mut selected: Vec<&str> = std::iter::once(&finding["object_id"])
        .chain(finding["related"].as_array().unwrap())
        .map(|object| object["local_id"].as_str().unwrap())
        .collect();
    selected.sort_unstable();
    assert_eq!(
        selected,
        ["#106", "#116", "#66", "#76", "#96"],
        "{result:#}"
    );
}

/// Offices #19 (x 0..4, floor at 0 m) and #29 (x 4.2..8.2, floor raised to
/// 0.5 m), both 3 m high and 4 m deep. Window #39 between them has its
/// bottom at 1.2 m, window #49 in the west facade of #19 at 0.8 m; both are
/// 1.2 m high and 0.1 m thick. `Pset_SpaceCommon.Reference` gives each
/// room's use.
fn offices_with_windows() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let window = "IFCWINDOW('GID',$,$,$,$,PL,REP,$,1.2,1.,$,$,$)";
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
         {}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Office'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19,#29),#201);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [6.2, 2.0, 0.5], [4.0, 4.0, 3.0], space),
        placed_box(30, [4.1, 2.0, 1.2], [0.1, 1.0, 1.2], window),
        placed_box(40, [-0.1, 2.0, 0.8], [0.1, 1.0, 1.2], window),
    )
}

#[test]
fn with_geometry_a_window_too_high_above_one_rooms_floor_is_found() {
    let case = Case::new("geometry-sill-height");
    let reference = json!({"type": "propertyReference",
                           "property": "axioval:example.ifc.reference",
                           "propertySet": "axioval:example.ifc.pset-space-common"});
    let (output, result) = case.geometry_rule(
        &offices_with_windows(),
        &[("window", "IfcWindow"), ("space", "IfcSpace")],
        "axioval:capability.keyed-limit",
        &registry_signature("axioval:capability.keyed-limit"),
        entity("window"),
        json!({
            "limits": {"type": "table", "value": [
                {"key_1": {"type": "string", "value": "Office"},
                 "maximum": {"type": "number", "value": 1.0}},
            ]},
            "quantity": {"type": "string", "value": "sill-height"},
            "floor_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
            "key_1": reference,
            "key_1_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #39 is 1.2 m above #19's floor but only 0.7 m above raised #29's; #49
    // is 0.8 m above #19's.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0].1.starts_with("sill height above the floor of ")
            && findings[0]
                .1
                .contains("#19 is 1.2 m; required at most 1 m (limit row 0: ")
            && !findings[0].1.contains("#29"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// A window between an office and a corridor is judged by the row for that
/// pair of space types, and a window in the facade by the row for the
/// outside, the reserved key `exterior`.
#[test]
fn with_geometry_a_window_is_limited_by_the_pair_of_spaces_it_joins() {
    let case = Case::new("geometry-sill-height-pairs");
    let reference = json!({"type": "propertyReference",
                           "property": "axioval:example.ifc.reference",
                           "propertySet": "axioval:example.ifc.pset-space-common"});
    // #29 becomes a corridor; #19 stays an office.
    let model = offices_with_windows().replace(
        "#202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19,#29),#201);\n",
        "#202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19),#201);\n\
         #210=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Corridor'),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'Pset_SpaceCommon',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#29),#211);\n",
    );
    let adjacent = json!(["axioval:derived.adjacent-space"]);
    let (output, result) = case.geometry_rule(
        &model,
        &[("window", "IfcWindow"), ("space", "IfcSpace")],
        "axioval:capability.keyed-limit",
        &registry_signature("axioval:capability.keyed-limit"),
        entity("window"),
        json!({
            "limits": {"type": "table", "value": [
                {"key_1": {"type": "string", "value": "Corridor"},
                 "other_side": {"type": "string", "value": "Office"},
                 "maximum": {"type": "number", "value": 1.0}},
                {"key_1": {"type": "string", "value": "exterior"},
                 "other_side": {"type": "string", "value": "*"},
                 "maximum": {"type": "number", "value": 0.7}},
            ]},
            "quantity": {"type": "string", "value": "sill-height"},
            "floor_path": {"type": "stringList", "value": adjacent},
            "key_1": reference,
            "key_1_path": {"type": "stringList", "value": adjacent},
            "pair_key": {"type": "string", "value": "key_1"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #39 is 1.2 m above the office's floor, beyond the 1 m of its pair
    // row; #49 is 0.8 m above it, beyond the 0.7 m towards the outside.
    let mut findings = finding_messages(&result);
    findings.sort();
    assert_eq!(findings.len(), 2, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0]
            .1
            .contains("#19 is 1.2 m; required at most 1 m (limit row 0: ")
            && findings[0].1.contains("`Office`")
            && findings[0].1.contains("`Corridor`"),
        "{result:#}"
    );
    assert_eq!(findings[1].0, "#49", "{result:#}");
    assert!(
        findings[1]
            .1
            .contains("is 0.8 m; required at most 0.7 m (limit row 1: ")
            && findings[1].1.contains("`exterior`"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// The window found too high is filed under the type of the spaces it
/// adjoins.
#[test]
fn with_geometry_sill_height_findings_are_categorised_by_the_adjacent_space() {
    let case = Case::new("geometry-sill-height-categories");
    let reference = json!({"type": "propertyReference",
                           "property": "axioval:example.ifc.reference",
                           "propertySet": "axioval:example.ifc.pset-space-common"});
    case.write("model.ifc", &offices_with_windows());
    let adjacent = json!(["axioval:derived.adjacent-space"]);
    let (output, result) = case.geometry_rule_with(
        &["model.ifc"],
        &[("window", "IfcWindow"), ("space", "IfcSpace")],
        (
            "axioval:capability.keyed-limit",
            &registry_signature("axioval:capability.keyed-limit"),
        ),
        entity("window"),
        json!({
            "limits": {"type": "table", "value": [
                {"key_1": {"type": "string", "value": "Office"},
                 "maximum": {"type": "number", "value": 1.0}},
            ]},
            "quantity": {"type": "string", "value": "sill-height"},
            "floor_path": {"type": "stringList", "value": adjacent},
            "key_1": reference,
            "key_1_path": {"type": "stringList", "value": adjacent},
        }),
        &json!({"categories": [{
            "propertySet": "axioval:example.ifc.pset-space-common",
            "property": "axioval:example.ifc.reference",
            "path": adjacent,
        }]}),
        &[],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0]
            .1
            .starts_with("[Office] sill height above the floor of "),
        "{result:#}"
    );
}

/// Halls #19 (x 0..20, y 0..10) and #29 (x 0..20, y 20..30), both 3 m
/// high, each with two 1 x 0.1 m doors in its north wall: #39 and #49 at
/// x 1..2 and 4..5 for #19, #59 and #69 at x 1..2 and 10..11 for #29.
/// `Pset_SpaceFireSafetyRequirements.SprinklerProtection` is false for #19
/// and true for #29.
fn halls_with_exits() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
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
         {}{}{}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('SprinklerProtection',$,IFCBOOLEAN(.F.),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceFireSafetyRequirements',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19),#201);\n\
         #210=IFCPROPERTYSINGLEVALUE('SprinklerProtection',$,IFCBOOLEAN(.T.),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'Pset_SpaceFireSafetyRequirements',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#29),#211);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [10.0, 5.0, 0.0], [20.0, 10.0, 3.0], space),
        placed_box(20, [10.0, 25.0, 0.0], [20.0, 10.0, 3.0], space),
        placed_box(30, [1.5, 10.05, 0.0], [1.0, 0.1, 2.1], door),
        placed_box(40, [4.5, 10.05, 0.0], [1.0, 0.1, 2.1], door),
        placed_box(50, [1.5, 30.05, 0.0], [1.0, 0.1, 2.1], door),
        placed_box(60, [10.5, 30.05, 0.0], [1.0, 0.1, 2.1], door),
    )
}

#[test]
fn with_geometry_exits_too_close_for_their_hall_are_found() {
    for (separation, apart) in [
        ("closest", "2 m apart between closest points"),
        ("centres", "3 m apart between centres"),
    ] {
        let case = Case::new(&format!("geometry-exit-separation-{separation}"));
        let (output, result) = case.geometry_rule(
            &halls_with_exits(),
            &[("door", "IfcDoor"), ("space", "IfcSpace")],
            "axioval:capability.exit-separation",
            &registry_signature("axioval:capability.exit-separation"),
            entity("space"),
            json!({
                "exit_path": {"type": "stringList",
                              "value": ["axioval:derived.adjacent-space:backward"]},
                "exit_selector": {"type": "selector", "value": entity("door")},
                "separation": {"type": "string", "value": separation},
                "flag": {"type": "propertyReference",
                         "property": "axioval:example.ifc.sprinkler-protection",
                         "propertySet": "axioval:example.ifc.pset-space-fire-safety"},
                "flagged_fraction": {"type": "number", "value": 1.0 / 3.0},
            }),
        );
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        // #19's doors are too close for half its 22.36 m diagonal; #29's,
        // 8 m apart, need only a third because it is sprinklered.
        let findings = finding_messages(&result);
        assert_eq!(findings.len(), 1, "{result:#}");
        assert_eq!(findings[0].0, "#19", "{result:#}");
        assert!(
            findings[0].1.contains(apart)
                && findings[0].1.contains(
                    "required at least 11.1803 m (0.5 × the longest plan diagonal of 22.3607 m, "
                )
                && findings[0].1.ends_with("false)"),
            "{result:#}"
        );
        assert!(
            result["report"]["not_evaluated"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "{result:#}"
        );
    }
}

#[test]
fn with_geometry_an_exit_flag_is_read_from_its_sources_in_order() {
    let case = Case::new("geometry-exit-separation-flag-sources");
    let source = |path: Option<&str>| {
        let mut row = json!({
            "property_set": {"type": "string",
                             "value": "axioval:example.ifc.pset-space-fire-safety"},
            "property": {"type": "string", "value": "axioval:example.ifc.sprinkler-protection"},
        });
        if let Some(path) = path {
            row["path"] = json!({"type": "string", "value": path});
        }
        row
    };
    let (output, result) = case.geometry_rule(
        &halls_with_exits(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.exit-separation",
        &registry_signature("axioval:capability.exit-separation"),
        entity("space"),
        json!({
            "exit_path": {"type": "stringList",
                          "value": ["axioval:derived.adjacent-space:backward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "flag_sources": {"type": "table", "value": [
                source(None),
                source(Some("IfcRelContainedInSpatialStructure:backward")),
            ]},
            "flag_default": {"type": "boolean", "value": true},
            "flagged_fraction": {"type": "number", "value": 1.0 / 3.0},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Each hall states its own flag, so the storey and the default are never
    // consulted: #19 is not sprinklered and its exits are too close.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#19", "{result:#}");
    assert!(findings[0].1.ends_with("false)"), "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Corridor #19 (x 0..20, y 0..2) and office #29 (x 0..6, y 4..9), both 3 m
/// high, with 1 m windows 0.2 m deep: #39 in the corridor's east end wall,
/// #49 in its south side wall, #59 in the office's east wall.
/// `Pset_SpaceCommon.Reference` is `Corridor` for #19 and `Office` for #29.
fn corridor_and_office_with_windows() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let window = "IFCWINDOW('GID',$,$,$,$,PL,REP,$,1.2,1.,$,$,$)";
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
         {}{}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Corridor'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19),#201);\n\
         #210=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Office'),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'Pset_SpaceCommon',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#29),#211);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [10.0, 1.0, 0.0], [20.0, 2.0, 3.0], space),
        placed_box(20, [3.0, 6.5, 0.0], [6.0, 5.0, 3.0], space),
        placed_box(30, [20.1, 1.0, 1.0], [0.2, 1.0, 1.2], window),
        placed_box(40, [10.5, -0.1, 1.0], [1.0, 0.2, 1.2], window),
        placed_box(50, [6.1, 6.5, 1.0], [0.2, 1.0, 1.2], window),
    )
}

#[test]
fn with_geometry_a_window_at_a_corridors_end_is_found() {
    let case = Case::new("geometry-corridor-end-openings");
    let (output, result) = case.geometry_rule(
        &corridor_and_office_with_windows(),
        &[("window", "IfcWindow"), ("space", "IfcSpace")],
        "axioval:capability.corridor-end-openings",
        &registry_signature("axioval:capability.corridor-end-openings"),
        json!({"kind": "allOf", "operands": [
            entity("space"),
            {"kind": "property", "propertySet": "axioval:example.ifc.pset-space-common",
             "property": "axioval:example.ifc.reference", "operator": "equals",
             "value": {"type": "string", "value": "Corridor"}},
        ]}),
        json!({
            "opening_path": {"type": "stringList",
                             "value": ["axioval:derived.adjacent-space:backward"]},
            "opening_selector": {"type": "selector", "value": entity("window")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #39 ends the corridor; #49 is in its side wall, and the office's #59
    // is in no corridor.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0]
            .1
            .starts_with("sits in the end wall of corridor ")
            && findings[0].1.contains("#19")
            && findings[0]
                .1
                .ends_with("0 m from the wall (20, 0)–(20, 2) and facing 1 m of it"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_property_comparison_finds_components_in_the_same_derived_space() {
    // Issue #43: `same_space` climbs the declared relationship to the
    // nearest container, so a derived containment serves as a stated one.
    let case = Case::new("geometry-same-derived-space");
    let (output, result) = case.geometry_rule(
        &rooms_without_containment(),
        &[("space", "IfcSpace"), ("furniture", "IfcFurnishingElement")],
        "axioval:capability.property-comparison",
        &registry_signature("axioval:capability.property-comparison"),
        entity("furniture"),
        json!({
            "compared_selector": {"type": "selector", "value": entity("furniture")},
            "component_mode": {"type": "string", "value": "same_space"},
            "container_selector": {"type": "selector", "value": entity("space")},
            "relationship": {"type": "string", "value": "axioval:derived.contained-in-space"},
            "direction": {"type": "string", "value": "forward"},
            "quantifier": {"type": "string", "value": "count"},
            "operator": {"type": "string", "value": "greater_or_equal"},
            "target_number": {"type": "number", "value": 1.0},
            "factor": {"type": "number", "value": 1.0},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #36 and #46 share room #16; #56 is alone in #26.
    assert_eq!(finding_ids(&result), ["#56"], "{result:#}");
    assert!(
        result["report"]["findings"][0]["message"]
            .as_str()
            .unwrap()
            .starts_with("count of compared components is 0"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// A box extruded `depth` up from `z`, its plan rectangle `length` x `width`
/// centred at (`x`, `y`), as instances `#first` to `#first + 9`. `PL` and
/// `REP` in `product` become its placement and shape.
fn placed_box(
    first: u32,
    [x, y, z]: [f64; 3],
    [length, width, depth]: [f64; 3],
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
        "#{origin}=IFCCARTESIANPOINT((0.,0.,{z:.2}));\n\
         #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
         #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
         #{p}=IFCCARTESIANPOINT(({x:.2},{y:.2}));\n\
         #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
         #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length:.2},{width:.2});\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth:.2});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        product
            .replace("GID", &format!("{object:022}"))
            .replace("PL", &format!("#{placement}"))
            .replace("REP", &format!("#{definition}")),
    )
}

/// Building #100 with storeys #101 at 0 m and #102 at 3 m, in metres.
///
/// The ground storey holds a 10 m wall #19 facing space #49, with a
/// 2 x 1.5 m window #39 filling opening #29; the upper storey holds wall #59,
/// 3.5 m high, beside the 3 m space #69.
fn storeys_with_facades() -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);\n\
         #8=IFCUNITASSIGNMENT((#6,#7));\n\
         #9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,(#5),#8);\n\
         {}{}{}{}{}{}\
         #100=IFCBUILDING('0000000000000000000100',$,'B',$,$,#3,$,$,.ELEMENT.,$,$,$);\n\
         #101=IFCBUILDINGSTOREY('0000000000000000000101',$,'EG',$,$,#3,$,$,.ELEMENT.,0.);\n\
         #102=IFCBUILDINGSTOREY('0000000000000000000102',$,'OG',$,$,#3,$,$,.ELEMENT.,3.);\n\
         #103=IFCRELAGGREGATES('0000000000000000000103',$,$,$,#100,(#101,#102));\n\
         #104=IFCRELAGGREGATES('0000000000000000000104',$,$,$,#101,(#49));\n\
         #105=IFCRELAGGREGATES('0000000000000000000105',$,$,$,#102,(#69));\n\
         #106=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000106',$,$,$,(#19,#39),#101);\n\
         #107=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000107',$,$,$,(#59),#102);\n\
         #108=IFCRELVOIDSELEMENT('0000000000000000000108',$,$,$,#19,#29);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [5.0, -0.15, 0.0], [10.0, 0.3, 3.0], wall),
        placed_box(
            20,
            [5.0, -0.15, 1.0],
            [2.0, 0.5, 1.5],
            "IFCOPENINGELEMENT('GID',$,$,$,$,PL,REP,$,.OPENING.)"
        ),
        placed_box(
            30,
            [5.0, -0.15, 1.0],
            [2.0, 0.3, 1.5],
            "IFCWINDOW('GID',$,$,$,$,PL,REP,$,1.5,2.,$,$,$)"
        ),
        placed_box(40, [5.0, 2.0, 0.0], [10.0, 4.0, 3.0], space),
        placed_box(50, [5.0, -0.15, 3.0], [10.0, 0.3, 3.5], wall),
        placed_box(60, [5.0, 2.0, 3.0], [10.0, 4.0, 3.0], space),
    )
}

/// `(object, message)` of every finding in a saved result, sorted.
fn finding_messages(result: &Value) -> Vec<(String, String)> {
    let mut findings: Vec<(String, String)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                finding["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    findings.sort();
    findings
}

/// The fixture definitions with storey metric rules: `level-spacing` as
/// `storey-heights` and `area-ratio` as `window-to-wall`.
fn storey_metric_definitions(case: &Case) -> PathBuf {
    let text = |value: &str| json!({"default": value, "translations": {}});
    let concept = |name: &str| {
        json!({"id": format!("axioval:example.ifc.{name}"), "name": text(name),
               "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
               "citations": []})
    };
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(case.definitions(true)).unwrap()).unwrap();
    for name in ["IfcBuilding", "IfcBuildingStorey", "IfcSpace", "IfcWindow"] {
        definitions["objectTypes"][format!("axioval:example.ifc.{name}")] = concept(name);
    }
    let mut elevation = concept("Elevation");
    elevation["valueKind"] = json!("quantity");
    definitions["properties"]["axioval:example.ifc.Elevation"] = elevation;
    let declare = |parameters: &[(&str, &str, bool)]| -> Value {
        parameters
            .iter()
            .chain(&[
                ("relationship", "string", false),
                ("direction", "string", false),
                ("follow_chain", "boolean", false),
                ("path", "stringList", false),
                ("skip_absent_relationship_ends", "boolean", false),
            ])
            .map(|(id, kind, required)| {
                (
                    (*id).to_owned(),
                    json!({"id": id, "name": text(id), "kind": kind,
                    "required": required, "allowedValues": [], "citations": []}),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    let definition = |id: &str, capability: &str, parameters: Value| {
        json!({"id": id, "name": text(id), "description": text(id),
               "capability": format!("axioval:capability.{capability}"),
               "parameters": parameters, "citations": [], "tags": []})
    };
    definitions["definitions"]["axioval:example.storey-heights"] = definition(
        "axioval:example.storey-heights",
        "level-spacing",
        declare(&[
            ("member_selector", "selector", true),
            ("order", "propertyReference", true),
            ("minimum", "quantity", false),
            ("maximum", "quantity", false),
            ("consistent", "boolean", false),
            ("tolerance", "quantity", false),
            ("ignore_lowest", "boolean", false),
            ("ignore_highest", "boolean", false),
            ("content_path", "stringList", false),
            ("content_selector", "selector", false),
            ("space_selector", "selector", false),
            ("space_path", "stringList", false),
            ("space_tolerance", "quantity", false),
            ("space_height", "boolean", false),
            ("space_elevation", "string", false),
        ]),
    );
    definitions["definitions"]["axioval:example.window-to-wall"] = definition(
        "axioval:example.window-to-wall",
        "area-ratio",
        declare(&[
            ("numerator_selector", "selector", true),
            ("denominator_selector", "selector", false),
            ("minimum", "number", false),
            ("maximum", "number", false),
            ("numerator_property", "propertyReference", false),
            ("denominator_property", "propertyReference", false),
            ("measure", "string", false),
            ("numerator_measure", "string", false),
            ("denominator_measure", "string", false),
            ("numerator_derivation", "string", false),
            ("empty_numerator_finding", "boolean", false),
            ("overall_width", "propertyReference", false),
            ("overall_height", "propertyReference", false),
            ("light_area_table", "table", false),
            ("light_type", "propertyReference", false),
            ("light_type_path", "stringList", false),
            ("light_size_tolerance", "quantity", false),
            ("frame_width", "quantity", false),
        ]),
    );
    let column = |id: &str, required: bool, dimension: Option<&str>| {
        let mut column = json!({"id": id, "name": text(id), "required": required,
                                "kind": if dimension.is_some() { "quantity" } else { "textPattern" }});
        if let Some(dimension) = dimension {
            column["unitDimension"] = json!(dimension);
        }
        column
    };
    definitions["definitions"]["axioval:example.window-to-wall"]["parameters"]["light_area_table"]
        ["columns"] = json!([
        column("type", false, None),
        column("width", true, Some("length")),
        column("height", true, Some("length")),
        column("light_area", true, Some("area")),
    ]);
    case.write("definitions.json", &definitions.to_string())
}

#[test]
fn with_geometry_storey_heights_and_window_to_wall_ratios_are_measured() {
    let case = Case::new("geometry-storey-metrics");
    let of = |name: &str| {
        json!({"kind": "entityType", "objectType": format!("axioval:example.ifc.{name}"),
               "includeSubtypes": true})
    };
    let text_file = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text_file).unwrap();
    let template = ruleset["root"]["rules"][0].clone();
    let rule = |id: &str, definition: &str, applies_to: &str, parameters: Value| {
        let mut rule = template.clone();
        rule["id"] = json!(id);
        rule["definitionId"] = json!(definition);
        rule["parameters"] = parameters;
        rule["applicability"]["groups"]["walls"]["selector"] = of(applies_to);
        rule
    };
    let metres = |value: f64| json!({"type": "quantity", "value": value, "unit": "m"});
    ruleset["root"]["rules"] = json!([
        rule(
            "storey-heights",
            "axioval:example.storey-heights",
            "IfcBuilding",
            json!({
                "member_selector": {"type": "selector", "value": of("IfcBuildingStorey")},
                "order": {"type": "propertyReference", "property": "axioval:example.ifc.Elevation",
                          "propertySet": "axioval:attributes"},
                "relationship": {"type": "string", "value": "IfcRelAggregates"},
                "maximum": metres(3.2),
                "content_path": {"type": "stringList",
                                 "value": ["IfcRelContainedInSpatialStructure"]},
                "content_selector": {"type": "selector", "value": of("wall")},
                "space_selector": {"type": "selector", "value": of("IfcSpace")},
                "space_path": {"type": "stringList", "value": ["IfcRelAggregates"]},
                "space_tolerance": metres(0.05),
            }),
        ),
        rule(
            "window-to-wall",
            "axioval:example.window-to-wall",
            "IfcBuildingStorey",
            json!({
                "measure": {"type": "string", "value": "facade"},
                "numerator_selector": {"type": "selector", "value": of("IfcWindow")},
                "denominator_selector": {"type": "selector", "value":
                    {"kind": "anyOf", "operands": [of("wall"), of("IfcWindow")]}},
                "maximum": {"type": "number", "value": 0.09},
                "relationship": {"type": "string", "value": "IfcRelContainedInSpatialStructure"},
            }),
        ),
    ]);
    let model = case.write("model.ifc", &storeys_with_facades());
    let definitions = storey_metric_definitions(&case);
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    assert_eq!(
        finding_messages(&result),
        [
            // Wall #19: its 27 m² outer face net of the window and two
            // 0.9 m² free ends; the window's outer face is 3 m².
            (
                "#101".to_owned(),
                "facade area ratio is 0.0943 (3 m² of 31.8 m²); required at most 0.09".to_owned()
            ),
            // The upper storey's wall rises 3.5 m above its elevation.
            (
                "#102".to_owned(),
                "level height is 3.5 m; required at most 3.2 m".to_owned()
            ),
            (
                "#69".to_owned(),
                "space height is 3 m and its level's height 3.5 m; they may differ by at most \
                 0.05 m"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    storey_tables_are_reported(&result, saved.to_str().unwrap());
}

/// What the storey metrics measured is reported beside the findings,
/// passing or not, and `report` summarizes and lists it.
fn storey_tables_are_reported(result: &Value, saved: &str) {
    let tables: Vec<(&str, &str, usize)> = result["report"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|table| {
            (
                table["rule_id"].as_str().unwrap(),
                table["name"].as_str().unwrap(),
                table["rows"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(
        tables,
        [
            ("storey-heights", "levels", 2),
            ("storey-heights", "spaces", 2),
            ("window-to-wall", "ratios", 2),
        ],
        "{result:#}"
    );
    let summary = stdout(&report(&[saved]));
    assert!(
        summary.contains("table:\n       2  levels   storey-heights\n          columns: elevation (m), height (m)"),
        "{summary}"
    );
    assert!(
        summary.contains(&format!("axioval report {saved} --section tables\n")),
        "{summary}"
    );
    let listing = stdout(&report(&[
        saved,
        "--section",
        "tables",
        "--rule",
        "storey-heights",
    ]));
    assert!(
        listing.contains("[table] levels storey-heights  #102 IFCBUILDINGSTOREY"),
        "{listing}"
    );
    assert!(
        listing.contains("elevation 3 m · height 3.5 m"),
        "{listing}"
    );
    assert!(listing.contains("showing 1–4 of 4"), "{listing}");
    let listing = report(&[saved, "--section", "tables", "--object", "#101", "--json"]);
    let listing: Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(listing["total"], 2, "{listing:#}");
    assert_eq!(listing["entries"][1]["section"], "tables");
    assert_eq!(listing["entries"][1]["level"], "ratios");
}

/// A file of 3 m-high rectangular walls: `(first id, centre x, centre y,
/// x extent, y extent, GlobalId)`. Each wall's product is `first + 6`.
fn walls_file(walls: &[(u32, f64, f64, f64, f64, &str)]) -> String {
    walls_of_height(walls, 3.0)
}

/// As [`walls_file`], with walls `height` metres high.
fn walls_of_height(walls: &[(u32, f64, f64, f64, f64, &str)], height: f64) -> String {
    let mut data = String::new();
    for &(first, x, y, length, width, global) in walls {
        let [p, pos, profile, solid, shape, product, wall] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        let _ = write!(
            data,
            "#{p}=IFCCARTESIANPOINT(({x},{y}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length},{width});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{height:?});\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{product}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{wall}=IFCWALL('{global}',$,$,$,$,#3,#{product},$,$);\n"
        );
    }
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
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

impl Case {
    /// Architectural walls (subjects) against structural bodies
    /// (counterparts), in two files of one check.
    fn discipline_clash(&self, models: &[&str], extra: &[&str]) -> Output {
        self.write("arch.ifc", &discipline_walls().0);
        self.write("struct.ifc", &discipline_walls().1);
        self.clash_across(models, extra)
    }

    /// The clash rule of [`Case::discipline_clash`] over the files already
    /// written.
    fn clash_across(&self, models: &[&str], extra: &[&str]) -> Output {
        let (definitions, ruleset) = self.clash_packages();
        let mut ruleset: Value =
            serde_json::from_str(&std::fs::read_to_string(&ruleset).unwrap()).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["applicability"]["groups"]["walls"]["selector"] = json!({"kind": "allOf", "operands": [
            {"kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true},
            {"kind": "discipline", "value": "architecture"},
        ]});
        rule["parameters"]["counterparts"]["value"] = json!({"kind": "allOf", "operands": [
            {"kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true},
            {"kind": "discipline", "value": "structure"},
        ]});
        let ruleset = self.write("ruleset.json", &ruleset.to_string());
        let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
        command.current_dir(&self.dir).arg("check");
        for model in models {
            command.arg("--model").arg(model);
        }
        command
            .arg("--definitions")
            .arg(definitions)
            .arg("--ruleset")
            .arg(ruleset)
            .args(extra)
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap()
    }
}

#[test]
fn a_clash_rule_between_two_disciplines_finds_a_clash_across_files() {
    let case = Case::new("discipline-clash");
    let saved = case.path("result.json");
    let bcf = case.path("issues.bcfzip");
    let output = case.discipline_clash(
        &["arch.ifc:architecture", "struct.ifc:structure"],
        &[
            "--geometry",
            "--report",
            saved.to_str().unwrap(),
            "--bcf",
            bcf.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    // One clash: the architectural wall against the structural one. The two
    // architectural walls also cross, but neither is a counterpart.
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["source"]["document"], "arch.ifc");
    assert_eq!(findings[0]["object_id"]["local_id"], "#16");
    assert_eq!(
        findings[0]["related"][0]["source"]["document"],
        "struct.ifc"
    );
    assert_eq!(findings[0]["related"][0]["local_id"], "#16");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(message.contains("penetration 0.1000 m"), "{message}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    // Both files were meshed into one set: three walls, all exact.
    assert_eq!(result["geometry"]["exact"], 3, "{result:#}");
    assert_eq!(
        result["objects"]["ifc-step:struct.ifc/#16"]["global_id"], "0000000000000000000S16",
        "{result:#}"
    );
    assert!(std::fs::metadata(&bcf).unwrap().len() > 0);

    // Over two documents the summary qualifies ids, and its hint runs.
    let summary = stdout(&report(&[saved.to_str().unwrap()]));
    assert!(summary.contains("arch.ifc/#16"), "{summary}");
    let listing = stdout(&report(&[
        saved.to_str().unwrap(),
        "--object",
        "arch.ifc/#16",
    ]));
    assert!(listing.contains("showing 1–1 of 1"), "{listing}");
}

/// Two crossing architectural walls, which clash with each other although
/// the rule only compares architecture with structure, and a structural wall
/// through the first architectural wall at x = 0.5.
fn discipline_walls() -> (String, String) {
    (
        walls_file(&[
            (10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000A16"),
            (20, 2.0, 0.0, 0.2, 4.0, "0000000000000000000A26"),
        ]),
        walls_file(&[(10, 0.5, 0.0, 0.2, 4.0, "0000000000000000000S16")]),
    )
}

/// `model` georeferenced onto EPSG:25832 at `easting` metres.
fn georeferenced(model: &str, easting: f64) -> String {
    model.replace(
        "ENDSEC;\nEND-ISO",
        &format!(
            "#90=IFCPROJECTEDCRS('EPSG:25832',$,$,$,$,$,#6);\n\
             #91=IFCMAPCONVERSION(#5,#90,{easting:?},5600000.,50.,$,$,$);\n\
             ENDSEC;\nEND-ISO"
        ),
    )
}

/// A `coordinate-consistency` rule over the architecture and structure
/// models, the architecture the reference.
fn coordinate_consistency(case: &Case) -> (Output, Value) {
    case.geometry_rule_over(
        &["arch.ifc:architecture", "struct.ifc:structure"],
        &[],
        "axioval:capability.coordinate-consistency",
        &registry_signature("axioval:capability.coordinate-consistency"),
        entity("wall"),
        json!({"reference": {"type": "string", "value": "architecture"}}),
    )
}

#[test]
fn federated_models_with_one_georeference_pass_and_a_shifted_one_is_named() {
    let case = Case::new("coordinate-consistency");
    let (arch, structure) = discipline_walls();
    case.write("arch.ifc", &georeferenced(&arch, 500_000.0));
    case.write("struct.ifc", &georeferenced(&structure, 500_000.0));
    let (output, result) = coordinate_consistency(&case);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");

    case.write("struct.ifc", &georeferenced(&structure, 500_001.0));
    let (output, result) = coordinate_consistency(&case);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(
        findings[0]["source"]["document"], "struct.ifc",
        "{result:#}"
    );
    assert_eq!(
        findings[0]["message"],
        "`ifc-step:struct.ifc` does not share the coordinate system of `ifc-step:arch.ifc`: map offset moved by 1.0000 m"
    );

    // Not georeferenced: never assumed to agree.
    case.write("struct.ifc", &structure);
    let (output, result) = coordinate_consistency(&case);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let outcomes = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(outcomes.len(), 1, "{result:#}");
    assert_eq!(outcomes[0]["source"]["document"], "struct.ifc");
    assert_eq!(outcomes[0]["reason"], "not_recorded");
}

#[test]
fn a_clash_with_a_model_in_another_coordinate_system_is_not_evaluated() {
    let case = Case::new("discipline-clash-shifted");
    let (arch, structure) = discipline_walls();
    case.write("arch.ifc", &georeferenced(&arch, 500_000.0));
    case.write("struct.ifc", &georeferenced(&structure, 500_000.0));
    let models = ["arch.ifc:architecture", "struct.ifc:structure"];
    let output = case.clash_across(&models, &["--geometry"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        json(&output)["report"]["findings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // The structural model's georeference is 1 m east: its walls are not in
    // the architecture's frame, so the clash is neither found nor passed.
    case.write("struct.ifc", &georeferenced(&structure, 500_001.0));
    let output = case.clash_across(&models, &["--geometry"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(result["report"]["findings"], json!([]), "{result:#}");
    assert_eq!(result["geometry"]["exact"], 2, "{result:#}");
    let unmeasured = result["geometry"]["unmeasured"].as_array().unwrap();
    assert_eq!(unmeasured.len(), 1, "{result:#}");
    assert!(
        unmeasured[0]["reason"].as_str().unwrap().contains(
            "not in the coordinate system of `ifc-step:arch.ifc`: map offset moved by 1.0000 m"
        ),
        "{result:#}"
    );
    assert!(
        !result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{result:#}"
    );
}

/// `model`, a STEP file, written as ifcXML in the codec's own layout with
/// IFC4's attribute names.
fn as_ifc_xml(model: &str) -> Vec<u8> {
    use ifc_model::Codec;
    let model = ifc_step::StepCodec.read_bytes(model.as_bytes()).unwrap();
    ifc_xml::XmlCodec::with_schema(std::sync::Arc::new(ifc_schema::ifc4().clone()))
        .write_bytes(&model)
        .unwrap()
}

/// The report with the structural model's source spelled as its STEP form
/// and without finding ids, which are derived from it.
fn as_step_report(result: &Value) -> Value {
    let mut report = result["report"].clone();
    if let Some(findings) = report["findings"].as_array_mut() {
        for finding in findings {
            finding.as_object_mut().unwrap().remove("id");
        }
    }
    let text = report
        .to_string()
        .replace("struct.ifcxml", "struct.ifc")
        .replace("ifc-xml", "ifc-step");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn an_ifcxml_model_gives_the_report_of_its_step_form() {
    let case = Case::new("discipline-clash-ifcxml");
    let (arch, structure) = discipline_walls();
    case.write("arch.ifc", &arch);
    case.write("struct.ifc", &structure);
    std::fs::write(case.path("struct.ifcxml"), as_ifc_xml(&structure)).unwrap();
    let step = case.clash_across(
        &["arch.ifc:architecture", "struct.ifc:structure"],
        &["--geometry"],
    );
    assert_eq!(step.status.code(), Some(3), "{}", stderr(&step));
    let xml = case.clash_across(
        &["arch.ifc:architecture", "struct.ifcxml:structure"],
        &["--geometry"],
    );
    assert_eq!(xml.status.code(), Some(3), "{}", stderr(&xml));
    let (step, xml) = (json(&step), json(&xml));
    assert_eq!(
        xml["geometry"]["exact"], 3,
        "the ifcXML model is meshed: {xml:#}"
    );
    assert_eq!(as_step_report(&xml), as_step_report(&step), "{xml:#}");
    assert_eq!(
        xml["report"]["findings"][0]["related"][0]["source"],
        json!({"document": "struct.ifcxml", "system": "ifc-xml"})
    );
}

#[test]
fn a_discipline_scoped_rule_over_an_undeclared_model_is_not_evaluated() {
    let case = Case::new("discipline-undeclared");
    let output = case.discipline_clash(&["arch.ifc:architecture", "struct.ifc"], &["--geometry"]);
    // The structural file declares no discipline, so whether its wall is a
    // counterpart is unknown: never a pass.
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let result = json(&output);
    let outcomes = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(
        outcomes
            .iter()
            .any(|outcome| outcome["source"]["document"] == "struct.ifc"
                && outcome["message"]
                    .as_str()
                    .unwrap()
                    .contains("declares no discipline")),
        "{result:#}"
    );
}

#[test]
fn two_models_with_one_file_name_or_a_bad_discipline_are_refused() {
    let case = Case::new("discipline-refused");
    std::fs::create_dir_all(case.path("other")).unwrap();
    std::fs::write(
        case.path("other/arch.ifc"),
        walls_file(&[(10, 0.0, 0.0, 1.0, 0.2, "0000000000000000000O16")]),
    )
    .unwrap();
    let output = case.discipline_clash(&["arch.ifc", "other/arch.ifc"], &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("share the file name `arch.ifc`"),
        "{}",
        stderr(&output)
    );

    let output = case.discipline_clash(&["arch.ifc:Architecture"], &[]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("invalid discipline"),
        "{}",
        stderr(&output)
    );
}

/// An IFC4 file written by `application`: wall `#10`, without the
/// `Reference` the example rule requires, and a project named `project`.
fn authored(application: &str, project: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCPERSON($,'Doe','Jane',$,$,$,$,$);\n\
         #2=IFCORGANIZATION($,'Firm',$,$,$);\n\
         #3=IFCPERSONANDORGANIZATION(#1,#2,$);\n\
         #4=IFCAPPLICATION(#2,'1','{application}','A');\n\
         #5=IFCOWNERHISTORY(#3,#4,$,.ADDED.,$,$,$,0);\n\
         #6=IFCPROJECT('0000000000000000000006',#5,'{project}',$,$,$,$,$,$);\n\
         #10=IFCWALL('0000000000000000000010',#5,$,$,$,$,$,$,$);\n\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

impl Case {
    /// The example rule over `arch.ifc` (written by an architecture
    /// application) and `struct.ifc` (by a structure application), its walls
    /// narrowed by `selector`.
    fn authored_check(&self, models: &[&str], selector: &Value, extra: &[&str]) -> Output {
        self.write(
            "arch.ifc",
            &authored("Modeller Architecture 2024", "Clinic"),
        );
        self.write("struct.ifc", &authored("Modeller Structure 2024", "Clinic"));
        let definitions = self.definitions(true);
        let mut ruleset: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap(),
        )
        .unwrap();
        ruleset["root"]["rules"][0]["applicability"]["groups"]["walls"]["selector"] = json!({
            "kind": "allOf",
            "operands": [
                {"kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true},
                selector,
            ],
        });
        let ruleset = self.write("ruleset.json", &ruleset.to_string());
        let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
        command.current_dir(&self.dir).arg("check");
        for model in models {
            command.arg("--model").arg(model);
        }
        command
            .arg("--definitions")
            .arg(definitions)
            .arg("--ruleset")
            .arg(ruleset)
            .args(extra)
            .output()
            .unwrap()
    }
}

fn finding_documents(result: &Value) -> Vec<String> {
    let mut documents: Vec<String> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            finding["object_id"]["source"]["document"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    documents.sort();
    documents
}

#[test]
fn a_source_selector_selects_the_objects_of_models_a_matching_application_wrote() {
    let case = Case::new("source-application");
    let output = case.authored_check(
        &["arch.ifc", "struct.ifc"],
        &json!({
            "kind": "source",
            "field": "application",
            "operator": "like",
            "value": {"type": "string", "value": "*Architecture*"},
        }),
        &[],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(finding_documents(&result), ["arch.ifc"], "{result:#}");
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");

    // The file name is the host's, the schema the file's.
    let output = case.authored_check(
        &["arch.ifc", "struct.ifc"],
        &json!({"kind": "anyOf", "operands": [
            {
                "kind": "source",
                "field": "fileName",
                "operator": "equals",
                "value": {"type": "string", "value": "struct.ifc"},
            },
            {
                "kind": "source",
                "field": "schema",
                "operator": "equals",
                "value": {"type": "string", "value": "IFC2X3"},
            },
        ]}),
        &[],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(finding_documents(&result), ["struct.ifc"], "{result:#}");
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

fn either_discipline() -> Value {
    json!({"kind": "anyOf", "operands": [
        {"kind": "discipline", "value": "architecture"},
        {"kind": "discipline", "value": "structure"},
    ]})
}

#[test]
fn a_discipline_map_assigns_disciplines_by_application_and_records_how() {
    let case = Case::new("discipline-map");
    let map = [
        "--discipline-map",
        "application:*Architecture*=architecture",
        "--discipline-map",
        "application:*Structure*=structure",
    ];
    let output = case.authored_check(&["arch.ifc", "struct.ifc"], &either_discipline(), &map);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    // The discipline rule evaluates both models.
    assert_eq!(
        finding_documents(&result),
        ["arch.ifc", "struct.ifc"],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
    assert_eq!(
        result["sources"],
        json!([
            {
                "source": "ifc-step:arch.ifc",
                "discipline": "architecture",
                "discipline_origin": "mapped",
                "mapped_by": "application:*Architecture*=architecture",
                "mapped_value": "Modeller Architecture 2024",
            },
            {
                "source": "ifc-step:struct.ifc",
                "discipline": "structure",
                "discipline_origin": "mapped",
                "mapped_by": "application:*Structure*=structure",
                "mapped_value": "Modeller Structure 2024",
            },
        ]),
        "{result:#}"
    );
    // A declared discipline wins; an unmapped model behaves as without a map.
    let output = case.authored_check(
        &["arch.ifc:structure", "struct.ifc"],
        &json!({"kind": "discipline", "value": "architecture"}),
        &map[..2],
    );
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(
        result["sources"][0]["discipline"], "structure",
        "{result:#}"
    );
    assert_eq!(result["sources"][0]["discipline_origin"], "declared");
    assert_eq!(
        result["sources"][1]["unmapped"],
        "no rule of the discipline map matches"
    );
    let outcomes = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(
        outcomes
            .iter()
            .any(|outcome| outcome["source"]["document"] == "struct.ifc"
                && outcome["message"]
                    .as_str()
                    .unwrap()
                    .contains("declares no discipline")),
        "{result:#}"
    );
}

#[test]
fn a_malformed_discipline_map_is_a_usage_error() {
    let case = Case::new("discipline-map-usage");
    for rule in [
        "application*Architecture*=architecture",
        "owner:*=architecture",
        "application:*=Architecture",
        "application:x\\=architecture",
    ] {
        let output = case.authored_check(
            &["arch.ifc"],
            &either_discipline(),
            &["--discipline-map", rule],
        );
        assert_eq!(output.status.code(), Some(2), "{rule}: {}", stderr(&output));
    }
}

impl Case {
    /// The example ruleset as package `package`, its rule named `r1`.
    fn ruleset_as(&self, package: &str) -> PathBuf {
        let mut ruleset: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap(),
        )
        .unwrap();
        ruleset["package"]["id"] = json!(package);
        ruleset["root"]["rules"][0]["id"] = json!("r1");
        self.write(&format!("{package}.json"), &ruleset.to_string())
    }

    fn check_rulesets(&self, rulesets: &[PathBuf]) -> Output {
        let model = self.write("model.ifc", &ifc("0000000000000000000002", false));
        let definitions = self.definitions(true);
        let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
        command
            .arg("check")
            .arg("--model")
            .arg(model)
            .arg("--definitions")
            .arg(definitions);
        for ruleset in rulesets {
            command.arg("--ruleset").arg(ruleset);
        }
        command.output().unwrap()
    }
}

#[test]
fn several_rulesets_run_in_one_check_under_ids_qualified_by_package() {
    let case = Case::new("several-rulesets");
    let rulesets = [
        case.ruleset_as("org.example.client"),
        case.ruleset_as("org.example.discipline"),
    ];
    let output = case.check_rulesets(&rulesets);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    // Both define `r1`; wall #2 lacks the reference under each, grouped by
    // package in rule id order.
    let findings: Vec<(&str, &str)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["rule_id"].as_str().unwrap(),
                finding["object_id"]["local_id"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        findings,
        [
            ("org.example.client/r1", "#2"),
            ("org.example.discipline/r1", "#2"),
        ],
        "{result:#}"
    );

    // One ruleset keeps its ids; one package twice is refused.
    let output = case.check_rulesets(&rulesets[..1]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(json(&output)["report"]["findings"][0]["rule_id"], "r1");
    let output = case.check_rulesets(&[rulesets[0].clone(), rulesets[0].clone()]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("duplicate ruleset package `org.example.client`"),
        "{}",
        stderr(&output)
    );
}

/// A clash matrix cell keyed by discipline on both sides.
fn discipline_cell(subject: &str, counterpart: &str, tolerance: f64, severity: &str) -> Value {
    json!({
        "label": {"type": "string", "value": format!("{subject} x {counterpart}")},
        "severity": {"type": "string", "value": severity},
        "subject_discipline": {"type": "string", "value": subject},
        "counterpart_discipline": {"type": "string", "value": counterpart},
        "penetration_tolerance_metres": {"type": "number", "value": tolerance},
    })
}

impl Case {
    /// Three files: architectural walls #16 (y = 0) and #26 (y = 20), a
    /// structural wall through #16 and a building-services wall through
    /// #26, each 0.1 m deep; every wall checked against every other by a
    /// clash matrix of `cells`, with `extra` parameters.
    fn clash_matrix(&self, cells: &Value, extra: &Value) -> (Output, Value) {
        let case = self;
        case.write(
            "arch.ifc",
            &walls_file(&[
                (10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000A16"),
                (20, 2.0, 20.0, 4.0, 0.2, "0000000000000000000A26"),
            ]),
        );
        case.write(
            "struct.ifc",
            &walls_file(&[(10, 0.5, 0.0, 0.2, 4.0, "0000000000000000000S16")]),
        );
        case.write(
            "mep.ifc",
            &walls_file(&[(10, 0.5, 20.0, 0.2, 4.0, "0000000000000000000M16")]),
        );
        let (definitions, ruleset) = case.clash_packages();
        let mut definitions: Value =
            serde_json::from_str(&std::fs::read_to_string(&definitions).unwrap()).unwrap();
        let definition = &mut definitions["definitions"]["axioval:example.clash"];
        definition["capability"] = json!("axioval:capability.clash-matrix");
        definition["parameters"] = registry_signature("axioval:capability.clash-matrix");
        let mut ruleset: Value =
            serde_json::from_str(&std::fs::read_to_string(&ruleset).unwrap()).unwrap();
        let mut parameters = json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "cells": {"type": "table", "value": cells},
            // Same-system exclusion is on by default; IFC reaches a system
            // through its group assignment.
            "system_path": {"type": "string", "value": "IfcRelAssignsToGroup:backward"},
        });
        for (name, value) in extra.as_object().unwrap() {
            parameters[name] = value.clone();
        }
        ruleset["root"]["rules"][0]["parameters"] = parameters;
        let definitions = case.write("definitions.json", &definitions.to_string());
        let ruleset = case.write("ruleset.json", &ruleset.to_string());
        let saved = case.path("result.json");
        let _ = std::fs::remove_file(&saved);
        let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(&case.dir)
            .arg("check")
            .args(["--model", "arch.ifc:architecture"])
            .args(["--model", "struct.ifc:structure"])
            .args(["--model", "mep.ifc:mep"])
            .arg("--definitions")
            .arg(definitions)
            .arg("--ruleset")
            .arg(ruleset)
            .args(["--geometry", "--report", saved.to_str().unwrap()])
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap();
        let result = std::fs::read_to_string(&saved)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        (output, result)
    }
}

/// One matrix judges both crossings, each with its own cell.
#[test]
fn a_clash_matrix_judges_each_discipline_pair_with_its_own_cell_across_files() {
    let case = Case::new("clash-matrix");
    // Architecture against structure accepts 0.05 m, against building
    // services 0.15 m: only the structural crossing is a clash.
    let (output, result) = case.clash_matrix(
        &json!([
            discipline_cell("architecture", "structure", 0.05, "warning"),
            discipline_cell("architecture", "mep", 0.15, "error"),
        ]),
        &json!({}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    let finding = &findings[0];
    assert_eq!(finding["severity"], "warning", "{finding:#}");
    let documents = [
        finding["object_id"]["source"]["document"].as_str().unwrap(),
        finding["related"][0]["source"]["document"]
            .as_str()
            .unwrap(),
    ];
    assert!(
        documents == ["arch.ifc", "struct.ifc"] || documents == ["struct.ifc", "arch.ifc"],
        "{finding:#}"
    );
    let message = finding["message"].as_str().unwrap();
    assert!(message.contains("penetration 0.1000 m"), "{message}");
    assert!(
        message.ends_with("(clash matrix cell 0 `architecture x structure`)"),
        "{message}"
    );

    // With the structural tolerance relaxed and no building-services cell,
    // the structural crossing passes and the uncovered pair is reported
    // when asked to.
    let (output, result) = case.clash_matrix(
        &json!([discipline_cell(
            "architecture",
            "structure",
            0.15,
            "warning"
        )]),
        &json!({"report_unmatched": {"type": "boolean", "value": true}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("no clash matrix cell covers")
            && message.contains("discipline `mep`")
            && message.contains("discipline `architecture`"),
        "{message}"
    );
}

/// Walls in metres, each 4 m long and 3 m high. #19 is 0.3 m thick and #29
/// 0.25 m, both across world y and both stating a 0.3 m layer set. #49 is
/// placed a quarter turn round, so it runs along world y and is 0.24 m thick
/// across world x; its layer set states 0.24 m.
fn walls_with_layers() -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,.STANDARD.)";
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
         {}{}\
         #40=IFCCARTESIANPOINT((10.,0.,0.));\n\
         #41=IFCDIRECTION((0.,1.,0.));\n\
         #42=IFCAXIS2PLACEMENT3D(#40,#4,#41);\n\
         #43=IFCLOCALPLACEMENT($,#42);\n\
         #44=IFCCARTESIANPOINT((2.,0.));\n\
         #45=IFCAXIS2PLACEMENT2D(#44,$);\n\
         #46=IFCRECTANGLEPROFILEDEF(.AREA.,$,#45,4.,0.24);\n\
         #47=IFCEXTRUDEDAREASOLID(#46,#2,#4,3.);\n\
         #48=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#47));\n\
         #50=IFCPRODUCTDEFINITIONSHAPE($,$,(#48));\n\
         #49=IFCWALL('0000000000000000000049',$,$,$,$,#43,#50,$,.STANDARD.);\n\
         #300=IFCMATERIAL('Concrete',$,$);\n\
         #301=IFCMATERIALLAYER(#300,0.3,$,$,$,$,$);\n\
         #302=IFCMATERIALLAYERSET((#301),'W300',$);\n\
         #303=IFCMATERIALLAYERSETUSAGE(#302,.AXIS2.,.POSITIVE.,0.,$);\n\
         #304=IFCRELASSOCIATESMATERIAL('0000000000000000000304',$,$,$,(#19,#29),#303);\n\
         #311=IFCMATERIALLAYER(#300,0.24,$,$,$,$,$);\n\
         #312=IFCMATERIALLAYERSET((#311),'W240',$);\n\
         #313=IFCMATERIALLAYERSETUSAGE(#312,.AXIS2.,.POSITIVE.,0.,$);\n\
         #314=IFCRELASSOCIATESMATERIAL('0000000000000000000314',$,$,$,(#49),#313);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 0.0, 0.0], [4.0, 0.3, 3.0], wall),
        placed_box(20, [2.0, 5.0, 0.0], [4.0, 0.25, 3.0], wall),
    )
}

#[test]
fn with_geometry_a_wall_body_is_measured_against_its_layer_thickness() {
    let case = Case::new("geometry-layer-thickness");
    let (output, result) = case.geometry_rule(
        &walls_with_layers(),
        &[("wall", "IfcWall")],
        "axioval:capability.body-extent",
        &registry_signature("axioval:capability.body-extent"),
        entity("wall"),
        json!({
            "axis": {"type": "string", "value": "forward"},
            "target_property": {"type": "propertyReference",
                                "property": "axioval:example.ifc.total-thickness",
                                "propertySet": "axioval:material"},
            "tolerance": {"type": "quantity", "value": 1.0, "unit": "mm"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Only #29's body departs from its layers; #49 is measured across its
    // own placement, not along world y where it is 4 m long.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#29", "{result:#}");
    assert!(
        findings[0]
            .1
            .starts_with("body extent along `forward` is 0.25 m; ")
            && findings[0].1.ends_with(" states 0.3 m within 0.001 m"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_meshes_with_too_many_triangles_are_found() {
    let case = Case::new("geometry-triangle-count");
    let (output, result) = case.geometry_rule(
        &walls_with_layers(),
        &[("wall", "IfcWall")],
        "axioval:capability.triangle-count",
        &registry_signature("axioval:capability.triangle-count"),
        entity("wall"),
        json!({"maximum": {"type": "integer", "value": 11}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // A box meshes to twelve triangles, planar and so counted exactly.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 3, "{result:#}");
    assert!(
        findings
            .iter()
            .all(|(_, message)| message == "mesh has 12 triangles; at most 11 allowed"),
        "{result:#}"
    );
}

#[test]
fn with_geometry_walls_are_graded_by_how_much_structure_stands_under_them() {
    let case = Case::new("geometry-counterpart-coverage");
    // #16 stands on a structural wall of its own size, #26 on nothing, and
    // #36 on one along half of it.
    case.write(
        "arch.ifc",
        &walls_file(&[
            (10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000A16"),
            (20, 2.0, 5.0, 4.0, 0.2, "0000000000000000000A26"),
            (30, 2.0, 10.0, 4.0, 0.2, "0000000000000000000A36"),
        ]),
    );
    // The structural walls stop 5 cm below the architectural ones' tops.
    case.write(
        "struct.ifc",
        &walls_of_height(
            &[
                (10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000S16"),
                (20, 1.0, 10.0, 2.0, 0.2, "0000000000000000000S26"),
            ],
            2.95,
        ),
    );
    let walls_of = |discipline: &str| {
        json!({"kind": "allOf", "operands": [
            entity("wall"), {"kind": "discipline", "value": discipline},
        ]})
    };
    let (output, result) = case.geometry_rule_over(
        &["arch.ifc:architecture", "struct.ifc:structure"],
        &[],
        "axioval:capability.counterpart-coverage",
        &registry_signature("axioval:capability.counterpart-coverage"),
        walls_of("architecture"),
        json!({
            "counterparts": {"type": "selector", "value": walls_of("structure")},
            "horizontal_tolerance": {"type": "quantity", "value": 0.02, "unit": "m"},
            "vertical_tolerance": {"type": "quantity", "value": 0.1, "unit": "m"},
            "info_above": {"type": "number", "value": 0.01},
            "warning_above": {"type": "number", "value": 0.25},
            "error_above": {"type": "number", "value": 0.75},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let mut findings: Vec<(String, String, String)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            assert_eq!(
                finding["object_id"]["source"]["document"], "arch.ifc",
                "{finding:#}"
            );
            (
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                finding["severity"].as_str().unwrap().to_owned(),
                finding["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    findings.sort();
    assert_eq!(findings.len(), 3, "{result:#}");
    // Nothing of the structure stands under #26, in plan or in height.
    assert_eq!(findings[0].0, "#26");
    assert_eq!(findings[0].1, "error");
    assert!(
        findings[0].2.starts_with("height: 1 of the height"),
        "{findings:#?}"
    );
    assert_eq!(findings[1].0, "#26");
    assert_eq!(findings[1].1, "error");
    assert!(
        findings[1]
            .2
            .starts_with("plan: 1 of the footprint (0.8 of 0.8 m²)"),
        "{findings:#?}"
    );
    // Half of #36 is uncovered, less the 2 cm tolerance past the end; its
    // height is covered within 10 cm, and so is all of #16.
    assert_eq!(findings[2].0, "#36");
    assert_eq!(findings[2].1, "warning");
    assert!(
        findings[2]
            .2
            .starts_with("plan: 0.495 of the footprint (0.396 of 0.8 m²)"),
        "{findings:#?}"
    );
    assert_eq!(
        result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["object_id"]["local_id"] == "#36")
            .unwrap()["related"][0]["source"]["document"],
        "struct.ifc"
    );
}

#[test]
fn with_geometry_a_wall_over_two_heights_of_structure_is_found_only_in_elevation() {
    let case = Case::new("geometry-counterpart-elevation");
    // A 4 m wall over a full-height structural wall along its left half and
    // a 1.5 m high one along its right half.
    case.write(
        "arch.ifc",
        &walls_file(&[(10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000A16")]),
    );
    case.write(
        "full.ifc",
        &walls_file(&[(10, 1.0, 0.0, 2.0, 0.2, "0000000000000000000F16")]),
    );
    case.write(
        "half.ifc",
        &walls_of_height(&[(10, 3.0, 0.0, 2.0, 0.2, "0000000000000000000H16")], 1.5),
    );
    let walls_of = |discipline: &str| {
        json!({"kind": "allOf", "operands": [
            entity("wall"), {"kind": "discipline", "value": discipline},
        ]})
    };
    let check = |measure: &str| {
        case.geometry_rule_over(
            &[
                "arch.ifc:architecture",
                "full.ifc:structure",
                "half.ifc:structure",
            ],
            &[],
            "axioval:capability.counterpart-coverage",
            &registry_signature("axioval:capability.counterpart-coverage"),
            walls_of("architecture"),
            json!({
                "counterparts": {"type": "selector", "value": walls_of("structure")},
                "tolerance": {"type": "quantity", "value": 0.02, "unit": "m"},
                "measure": {"type": "string", "value": measure},
                "info_above": {"type": "number", "value": 0.01},
                "warning_above": {"type": "number", "value": 0.25},
            }),
        )
    };
    let (output, result) = check("plan_and_height");
    assert_eq!(output.status.code(), Some(0), "{result:#}");
    let (output, result) = check("elevation");
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    // The upper right quarter, less the 2 cm tolerance each way.
    assert!(
        findings[0]
            .1
            .starts_with("elevation: 0.2442 of the elevation (2.9304 of 12 m²)"),
        "{findings:#?}"
    );
}

/// A side profile extruded 1.2 m across, as instances `#first` to
/// `#first + 8` and its corners from `#first + 10`: `points` (along, up)
/// stand in the vertical plane through `origin` along world x and are swept
/// towards -y. `PL` and `REP` in `product` become its placement and shape;
/// the product is `#first + 8`.
fn profiled(first: u32, origin: [f64; 3], points: &[[f64; 2]], product: &str) -> String {
    swept(first, origin, points, 1.2, product)
}

/// A side profile as [`profiled`], extruded `depth` across.
fn swept(
    first: u32,
    [x, y, z]: [f64; 3],
    points: &[[f64; 2]],
    depth: f64,
    product: &str,
) -> String {
    let [
        location,
        position,
        placement,
        polyline,
        profile,
        solid,
        shape,
        definition,
        object,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8].map(|offset| first + offset);
    let mut text = String::new();
    let mut corners = Vec::new();
    for (index, [along, up]) in points.iter().chain(points.first()).enumerate() {
        let corner = first + 10 + u32::try_from(index).unwrap();
        writeln!(text, "#{corner}=IFCCARTESIANPOINT(({along:.3},{up:.3}));").unwrap();
        corners.push(format!("#{corner}"));
    }
    // The profile's plane stands upright: its x along world x (#10), its
    // normal and so the sweep along world -y (#9).
    write!(
        text,
        "#{location}=IFCCARTESIANPOINT(({x:.2},{y:.2},{z:.2}));\n\
         #{position}=IFCAXIS2PLACEMENT3D(#{location},#9,#10);\n\
         #{placement}=IFCLOCALPLACEMENT($,#2);\n\
         #{polyline}=IFCPOLYLINE(({}));\n\
         #{profile}=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#{polyline});\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#{position},#4,{depth:.3});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        corners.join(","),
        product
            .replace("GID", &format!("{object:022}"))
            .replace("PL", &format!("#{placement}"))
            .replace("REP", &format!("#{definition}")),
    )
    .unwrap();
    text
}

/// A stair side profile on a flat base: `risers` high, 0.28 m goings, the
/// last tread its top.
fn stair_profile(risers: &[f64]) -> Vec<[f64; 2]> {
    let top: f64 = risers.iter().sum();
    let mut points = vec![
        [0.0, 0.0],
        [0.28 * f64::from(u32::try_from(risers.len()).unwrap()), 0.0],
    ];
    points.push([points[1][0], top]);
    let mut elevation = top;
    for step in (0..risers.len()).rev() {
        let front = 0.28 * f64::from(u32::try_from(step).unwrap());
        points.push([front, elevation]);
        elevation -= risers[step];
        if step > 0 {
            points.push([front, elevation]);
        }
    }
    points
}

/// A ramp side profile: a 1 m landing at 0.1 m, a run rising 0.5 m over
/// `length`, and a 1 m landing at 0.6 m.
fn ramp_profile(length: f64) -> Vec<[f64; 2]> {
    vec![
        [0.0, 0.0],
        [length + 2.0, 0.0],
        [length + 2.0, 0.6],
        [length + 1.0, 0.6],
        [1.0, 0.1],
        [0.0, 0.1],
    ]
}

/// Stair flights #108 (four 0.17 m risers) and #208 (its third riser
/// 0.21 m), ramp flights #308 (0.5 m over 6 m) and #408 (0.5 m over 3 m),
/// and beam #509, its underside 2.5 m up across #108's second and third
/// treads (x 0.41 to 0.71). Every flight is 1.2 m wide.
fn stairs_and_ramps() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let ramp = "IFCRAMPFLIGHT('GID',$,$,$,$,PL,REP,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        profiled(
            200,
            [0.0, 5.0, 0.0],
            &stair_profile(&[0.17, 0.17, 0.21, 0.17]),
            flight
        ),
        profiled(300, [5.0, 0.0, 0.0], &ramp_profile(6.0), ramp),
        profiled(400, [5.0, 5.0, 0.0], &ramp_profile(3.0), ramp),
        placed_box(
            500,
            [0.56, -0.6, 2.5],
            [0.3, 2.0, 0.3],
            "IFCBEAM('GID',$,$,$,$,PL,REP,$,$)"
        ),
    )
}

#[test]
fn with_geometry_a_sprinkler_is_measured_to_the_sloped_slab_right_above_it() {
    let case = Case::new("geometry-distance-surfaces");
    // The slab's underside rises from 3 m at x = 0 to 4 m at x = 4; the
    // sprinkler's top stands at 2.6 m under x = 1.9..2.1.
    let model = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCUNITASSIGNMENT((#6));\n\
         #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(
            100,
            [0.0, 1.0, 0.0],
            &[[0.0, 3.0], [4.0, 4.0], [4.0, 4.2], [0.0, 3.2]],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,$)",
        ),
        placed_box(
            200,
            [2.0, 0.5, 2.5],
            [0.2, 0.2, 0.1],
            "IFCBUILDINGELEMENTPROXY('GID',$,$,$,$,PL,REP,$,$)",
        ),
    );
    let check = |surfaces: bool| {
        let mut parameters = json!({
            "counterparts": {"type": "selector", "value": entity("slab")},
            "maximum_metres": {"type": "number", "value": 0.5},
            "projection": {"type": "string", "value": "vertical"},
            "vertical_direction": {"type": "string", "value": "above"},
        });
        if surfaces {
            parameters["subject_surface"] = json!({"type": "string", "value": "top"});
            parameters["counterpart_surface"] = json!({"type": "string", "value": "nearest"});
        }
        case.geometry_rule(
            &model,
            &[
                ("slab", "IfcSlab"),
                ("sprinkler", "IfcBuildingElementProxy"),
            ],
            "axioval:capability.distance",
            &registry_signature("axioval:capability.distance"),
            entity("sprinkler"),
            parameters,
        )
    };
    // The slab's lowest point is 0.4 m above the sprinkler, elsewhere.
    let (output, result) = check(false);
    assert_eq!(output.status.code(), Some(0), "{result:#}");
    // Right above it, the underside is 0.875 m up.
    let (output, result) = check(true);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert!(
        findings[0].1.contains(
            "vertical distance from its top to the nearest surface 0.8750 m above, farther \
             than the allowed 0.5000 m"
        ),
        "{findings:#?}"
    );
}

#[test]
fn with_geometry_an_irregular_riser_and_too_little_headroom_are_found() {
    let case = Case::new("geometry-stair-flights");
    let (output, result) = case.geometry_rule(
        &stairs_and_ramps(),
        &[("flight", "IfcStairFlight"), ("beam", "IfcBeam")],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "riser_minimum": {"type": "quantity", "value": 14, "unit": "cm"},
            "riser_maximum": {"type": "quantity", "value": 19, "unit": "cm"},
            "riser_tolerance": {"type": "quantity", "value": 5, "unit": "mm"},
            "going_minimum": {"type": "quantity", "value": 26, "unit": "cm"},
            "minimum_headroom": {"type": "quantity", "value": 2, "unit": "m"},
            "headroom_obstacles": {"type": "selector", "value": entity("beam")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #208's third riser is found twice, too high and irregular; #108 is
    // regular, but the beam leaves 2.5 - 0.51 m above its third tread.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 3, "{result:#}");
    assert_eq!(findings[0].0, "#108", "{result:#}");
    assert!(
        findings[0]
            .1
            .starts_with("headroom above the walking surface is 1.99 m under ")
            && findings[0].1.ends_with("#509; at least 2 m required"),
        "{result:#}"
    );
    assert_eq!(
        findings[1..],
        [
            (
                "#208".to_owned(),
                "riser 3 of 4 is 0.21 m; 0.14 m to 0.19 m required".to_owned()
            ),
            (
                "#208".to_owned(),
                "risers differ by 0.04 m (0.17 m, 0.17 m, 0.21 m, 0.17 m); at most 0.005 m \
                 allowed"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_a_ramp_too_steep_for_its_run_is_found() {
    let case = Case::new("geometry-ramps");
    let (output, result) = case.geometry_rule(
        &stairs_and_ramps(),
        &[("ramp", "IfcRampFlight")],
        "axioval:capability.ramp-geometry",
        &registry_signature("axioval:capability.ramp-geometry"),
        entity("ramp"),
        json!({
            "slope_limits": {"type": "table", "value": [
                {"maximum_slope": {"type": "number", "value": 0.0834},
                 "maximum_rise": {"type": "quantity", "value": 0.76, "unit": "m"}},
            ]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#408".to_owned(),
            "run 1 of 1 rises 0.5 m over 3 m, a slope of 0.166667; required slope at most \
             0.0834 rising at most 0.76 m"
                .to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Stair flight #108 (four 0.17 m risers, x 0 to 1.12, y -1.2 to 0) with
/// landing slab #209 beyond its top tread (x 1.12 to 1.92, top at 0.68 m)
/// and floor slab #309 under its foot (x -2 to 2, top at 0); flight #408,
/// its underside sloping up from its foot 1 m above the floor of space #509
/// (x 4.06 to 7.06, from 0 m).
fn landings_and_soffits() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let soffit = [
        [0.0, 0.0],
        [1.12, 0.4],
        [1.12, 0.68],
        [0.84, 0.68],
        [0.84, 0.51],
        [0.56, 0.51],
        [0.56, 0.34],
        [0.28, 0.34],
        [0.28, 0.17],
        [0.0, 0.17],
    ];
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        placed_box(
            200,
            [1.52, -0.6, 0.48],
            [0.8, 1.2, 0.2],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.LANDING.)"
        ),
        placed_box(
            300,
            [0.0, -0.6, -0.2],
            [4.0, 3.0, 0.2],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.FLOOR.)"
        ),
        profiled(400, [5.0, 0.0, 1.0], &soffit, flight),
        placed_box(
            500,
            [5.56, -0.6, 0.0],
            [3.0, 3.0, 3.0],
            "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)"
        ),
    )
}

#[test]
fn with_geometry_a_shallow_landing_and_a_low_soffit_are_found() {
    let case = Case::new("geometry-stair-landings");
    let (output, result) = case.geometry_rule(
        &landings_and_soffits(),
        &[
            ("flight", "IfcStairFlight"),
            ("slab", "IfcSlab"),
            ("space", "IfcSpace"),
        ],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "width_minimum": {"type": "quantity", "value": 1.2, "unit": "m"},
            "landing_objects": {"type": "selector", "value": entity("slab")},
            "landing_depth_minimum": {"type": "quantity", "value": 1.2, "unit": "m"},
            "landing_at_least_walking_width": {"type": "boolean", "value": true},
            "minimum_headroom_below": {"type": "quantity", "value": 2, "unit": "m"},
            "headroom_below_spaces": {"type": "selector", "value": entity("space")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #108 is 1.2 m wide and so is its landing, but the landing reaches only
    // 1.08 m past the last riser; the floor gives it 2 m at its foot. #408
    // stands on nothing selected, 1 m above the space's floor.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert_eq!(
        findings[0],
        (
            "#108".to_owned(),
            "the landing at the top of the flight is 1.08 m deep; at least 1.2 m and the \
             flight's width (1.2 m) required"
                .to_owned()
        ),
        "{result:#}"
    );
    assert_eq!(findings[1].0, "#408", "{result:#}");
    assert!(
        findings[1]
            .1
            .starts_with("headroom below the flight is 1 m over the floor of ")
            && findings[1].1.ends_with("#509; at least 2 m required"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Stair flight #108 (1.2 m wide, y -1.2 to 0) arriving at landing slab
/// #200 as wide, with walls #300 and #400 standing on the landing 1 m apart.
fn a_narrow_landing() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        placed_box(
            200,
            [1.72, -0.6, 0.48],
            [1.2, 1.2, 0.2],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.LANDING.)"
        ),
        placed_box(300, [1.72, -1.15, 0.68], [1.2, 0.1, 2.0], wall),
        placed_box(400, [1.72, -0.05, 0.68], [1.2, 0.1, 2.0], wall),
    )
}

#[test]
fn with_geometry_a_landing_narrower_than_its_flight_fails_its_clear_widths() {
    let case = Case::new("geometry-stair-landing-clear-width");
    let run = |parameters: Value| {
        case.geometry_rule(
            &a_narrow_landing(),
            &[
                ("flight", "IfcStairFlight"),
                ("slab", "IfcSlab"),
                ("wall", "IfcWall"),
            ],
            "axioval:capability.stair-geometry",
            &registry_signature("axioval:capability.stair-geometry"),
            entity("flight"),
            parameters,
        )
    };
    let band = || {
        json!({
            "clear_width_minimum": {"type": "quantity", "value": 1.1, "unit": "m"},
            "clear_width_obstacles": {"type": "selector", "value": entity("wall")},
            "clear_width_band_from": {"type": "quantity", "value": 0.5, "unit": "m"},
            "clear_width_band_to": {"type": "quantity", "value": 1.5, "unit": "m"},
            "landing_objects": {"type": "selector", "value": entity("slab")},
        })
    };
    let mut parameters = band();
    parameters["landing_clear_width_minimum"] =
        json!({"type": "quantity", "value": 1.1, "unit": "m"});
    parameters["total_clear_width_minimum"] =
        json!({"type": "quantity", "value": 1.1, "unit": "m"});
    let (output, result) = run(parameters);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The flight is 1.2 m clear; the walls leave the landing 1 m.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert!(
        findings.iter().all(|(object, _)| object == "#108"),
        "{result:#}"
    );
    assert!(
        findings[0].1.starts_with(
            "the clear width of the landing at the top of the flight 0.5 m to 1.5 m above its \
             level is 1 m beside "
        ) && findings[0].1.ends_with("; at least 1.1 m required"),
        "{result:#}"
    );
    assert!(
        findings[1].1.starts_with(
            "the least clear width of the flight and its landings is 1 m, at the landing at the \
             top of the flight"
        ),
        "{result:#}"
    );
    // Its bottom has no selected landing, which is nothing to measure.
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    // The flight's own minimum passes.
    let mut parameters = band();
    parameters
        .as_object_mut()
        .unwrap()
        .remove("landing_objects");
    let (output, result) = run(parameters);
    assert_eq!(output.status.code(), Some(0), "{result:#}");
}

/// The side profile of a rail 0.05 m deep whose top runs `height` above
/// the nosing line of a four-riser flight with 0.17 m risers (nosings from
/// x 0 at 0.17 m to x 0.84 at 0.68 m), level for 0.3 m beyond either end.
fn rail_profile(height: f64) -> Vec<[f64; 2]> {
    let top = [
        [-0.3, 0.17 + height],
        [0.0, 0.17 + height],
        [0.84, 0.68 + height],
        [1.14, 0.68 + height],
    ];
    let mut points: Vec<[f64; 2]> = top.iter().map(|[x, z]| [*x, z - 0.05]).collect();
    points.extend(top.iter().rev());
    points
}

/// Stair flight #108 (four 0.17 m risers, x 0 to 1.12, y -1.2 to 0) with
/// handrail #208 along its left side (y 0 to 0.05, seen climbing) 0.9 m
/// above its nosing line and handrail #308 along its right (y -1.25 to
/// -1.2) only 0.75 m above it; ramp flight #408 (a run from x 6 to 12 between
/// landings of its own) and furnishing element #509 standing on its lower
/// landing, x 5.05 to 5.35.
fn handrails_and_ramp_ends() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
    let ramp = "IFCRAMPFLIGHT('GID',$,$,$,$,PL,REP,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        swept(200, [0.0, 0.05, 0.0], &rail_profile(0.9), 0.05, rail),
        swept(300, [0.0, -1.2, 0.0], &rail_profile(0.75), 0.05, rail),
        profiled(400, [5.0, 0.0, 0.0], &ramp_profile(6.0), ramp),
        placed_box(
            500,
            [5.2, -0.6, 0.1],
            [0.3, 0.3, 0.8],
            "IFCFURNISHINGELEMENT('GID',$,$,$,$,PL,REP,$)"
        ),
    )
}

#[test]
fn with_geometry_a_handrail_too_low_above_the_nosing_line_is_found() {
    let case = Case::new("geometry-stair-handrails");
    let (output, result) = case.geometry_rule(
        &handrails_and_ramp_ends(),
        &[("flight", "IfcStairFlight"), ("rail", "IfcRailing")],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "handrail_objects": {"type": "selector", "value": entity("rail")},
            "handrail_reach_across": {"type": "quantity", "value": 0.2, "unit": "m"},
            "handrail_reach_above": {"type": "quantity", "value": 1.5, "unit": "m"},
            "handrail_height_minimum": {"type": "quantity", "value": 0.8, "unit": "m"},
            "handrail_height_maximum": {"type": "quantity", "value": 1.1, "unit": "m"},
            "handrail_extension_minimum": {"type": "quantity", "value": 30, "unit": "cm"},
            "handrail_sides": {"type": "string", "value": "both"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Both rails reach 0.3 m level past either end and run along a side
    // each; #308 runs too low.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#108", "{result:#}");
    assert!(
        findings[0].1.starts_with("handrail ")
            && findings[0].1.ends_with(
                "#308 runs 0.75 m above the pitch line of the flight at its lowest; 0.8 m to \
                 1.1 m required"
            ),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// The side profile of a piece of [`rail_profile`]'s rail from `from` to
/// `to` along x.
fn rail_piece_profile(height: f64, from: f64, to: f64) -> Vec<[f64; 2]> {
    let top = |x: f64| 0.17 + height + x.clamp(0.0, 0.84) * 0.51 / 0.84;
    let mut along = vec![from];
    along.extend([0.0, 0.84].into_iter().filter(|x| *x > from && *x < to));
    along.push(to);
    let mut points: Vec<[f64; 2]> = along.iter().map(|x| [*x, top(*x) - 0.05]).collect();
    points.extend(along.iter().rev().map(|x| [*x, top(*x)]));
    points
}

/// Stair flight #108 as in [`handrails_and_ramp_ends`] with its left
/// handrail in two pieces, #208 from x -0.3 to 0.4 and #308 from x 0.5 to
/// 1.14, 0.1 m apart, and its right handrail #408 in one.
fn handrail_in_pieces() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        swept(
            200,
            [0.0, 0.05, 0.0],
            &rail_piece_profile(0.9, -0.3, 0.4),
            0.05,
            rail
        ),
        swept(
            300,
            [0.0, 0.05, 0.0],
            &rail_piece_profile(0.9, 0.5, 1.14),
            0.05,
            rail
        ),
        swept(400, [0.0, -1.2, 0.0], &rail_profile(0.9), 0.05, rail),
    )
}

#[test]
fn with_geometry_a_gap_between_handrail_pieces_is_found() {
    let case = Case::new("geometry-stair-handrail-pieces");
    let run = |gap: f64| {
        case.geometry_rule(
            &handrail_in_pieces(),
            &[("flight", "IfcStairFlight"), ("rail", "IfcRailing")],
            "axioval:capability.stair-geometry",
            &registry_signature("axioval:capability.stair-geometry"),
            entity("flight"),
            json!({
                "handrail_objects": {"type": "selector", "value": entity("rail")},
                "handrail_reach_across": {"type": "quantity", "value": 0.2, "unit": "m"},
                "handrail_reach_above": {"type": "quantity", "value": 1.5, "unit": "m"},
                "handrail_height_minimum": {"type": "quantity", "value": 0.8, "unit": "m"},
                "handrail_extension_minimum": {"type": "quantity", "value": 30, "unit": "cm"},
                "handrail_gap_maximum": {"type": "quantity", "value": gap, "unit": "m"},
                "handrail_sides": {"type": "string", "value": "both"},
            }),
        )
    };
    let (output, result) = run(0.05);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The left handrail reaches 0.3 m beyond either end from its first and
    // last piece, but breaks off for 0.1 m between them.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#108", "{result:#}");
    assert!(
        findings[0].1.ends_with(
            "#308 along the left side of the flight leave a gap of 0.1 m in plan; at most 0.05 m \
             allowed"
        ),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    // A gap of 0.15 m is allowed: the handrail in pieces passes.
    let (output, result) = run(0.15);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_messages(&result).is_empty(), "{result:#}");
}

#[test]
fn with_geometry_an_obstacle_at_the_foot_of_a_ramp_is_found() {
    let case = Case::new("geometry-ramp-ends");
    let (output, result) = case.geometry_rule(
        &handrails_and_ramp_ends(),
        &[
            ("ramp", "IfcRampFlight"),
            ("furniture", "IfcFurnishingElement"),
        ],
        "axioval:capability.ramp-geometry",
        &registry_signature("axioval:capability.ramp-geometry"),
        entity("ramp"),
        json!({
            "end_space_depth": {"type": "quantity", "value": 1.5, "unit": "m"},
            "end_space_width": {"type": "quantity", "value": 1.5, "unit": "m"},
            "end_space_height": {"type": "quantity", "value": 2, "unit": "m"},
            "end_space_obstacles": {"type": "selector", "value": entity("furniture")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The 1.5 m square before the run's foot (x 4.5 to 6) holds #509; the
    // one past its head (x 12 to 13.5) is clear.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#408", "{result:#}");
    assert!(
        findings[0].1.ends_with(
            "#509 obstructs the free space at the bottom of the ramp (1.5 m deep, 1.5 m wide)"
        ),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// A stair near a ramp is the `distance` capability's nearest mode: the
/// ramp's nearest flight must lie within the maximum.
#[test]
fn with_geometry_a_ramp_without_a_stair_nearby_is_found() {
    let case = Case::new("geometry-stair-near-ramp");
    let near = |maximum: f64| {
        case.geometry_rule(
            &handrails_and_ramp_ends(),
            &[("ramp", "IfcRampFlight"), ("flight", "IfcStairFlight")],
            "axioval:capability.distance",
            &registry_signature("axioval:capability.distance"),
            entity("ramp"),
            json!({
                "counterparts": {"type": "selector", "value": entity("flight")},
                "mode": {"type": "string", "value": "nearest"},
                "maximum_metres": {"type": "number", "value": maximum},
                "projection": {"type": "string", "value": "horizontal"},
            }),
        )
    };
    // The flight ends at x 1.12, the ramp starts at x 5: 3.88 m apart.
    let (output, result) = near(3.0);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), ["#408"], "{result:#}");
    let (output, result) = near(4.0);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_ids(&result).is_empty(), "{result:#}");
}

/// A closed, outward solid standing on `z = 0` as `(positions, triangles)`:
/// every cell, a convex counter-clockwise polygon of `points` sharing
/// corners by index, rises to its height, and a wall falls from each cell
/// to a lower neighbour or the outside, split at every height meeting its
/// ends.
/// A rail's side profile 0.05 m deep under the top line `top`.
fn rail_under(top: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut points: Vec<[f64; 2]> = top.iter().map(|[x, z]| [*x, z - 0.05]).collect();
    points.extend(top.iter().rev().copied());
    points
}

/// Ramp flight #108 (1.2 m wide, y -1.2 to 0) of two runs, x 1 to 4 and
/// 5.5 to 8.5, each rising 0.25 m, with a level landing between them, and
/// rails #208 and #308 beside its left side 0.9 m above its surface, the
/// first along the first run and the second along the second, stopping
/// 0.4 m short of each other on the landing.
fn a_ramp_with_rails_in_pieces() -> String {
    let ramp = "IFCRAMPFLIGHT('GID',$,$,$,$,PL,REP,$,$)";
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
    let profile = [
        [0.0, 0.0],
        [9.5, 0.0],
        [9.5, 0.6],
        [8.5, 0.6],
        [5.5, 0.35],
        [4.0, 0.35],
        [1.0, 0.1],
        [0.0, 0.1],
    ];
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &profile, ramp),
        swept(
            200,
            [0.0, 0.05, 0.0],
            &rail_under(&[[0.7, 1.0], [1.0, 1.0], [4.0, 1.25], [4.55, 1.25]]),
            0.05,
            rail
        ),
        swept(
            300,
            [0.0, 0.05, 0.0],
            &rail_under(&[[4.95, 1.25], [5.5, 1.25], [8.5, 1.5], [8.8, 1.5]]),
            0.05,
            rail
        ),
    )
}

#[test]
fn with_geometry_ramp_rails_stopping_short_on_a_landing_are_found() {
    let case = Case::new("geometry-ramp-rail-continuity");
    let run = |tolerance: f64| {
        case.geometry_rule(
            &a_ramp_with_rails_in_pieces(),
            &[("ramp", "IfcRampFlight"), ("rail", "IfcRailing")],
            "axioval:capability.ramp-geometry",
            &registry_signature("axioval:capability.ramp-geometry"),
            entity("ramp"),
            json!({
                "handrail_objects": {"type": "selector", "value": entity("rail")},
                "handrail_reach_across": {"type": "quantity", "value": 0.2, "unit": "m"},
                "handrail_reach_above": {"type": "quantity", "value": 1.5, "unit": "m"},
                "check_continuous_handrails": {"type": "boolean", "value": true},
                "handrail_continuity_tolerance": {"type": "quantity", "value": tolerance, "unit": "m"},
            }),
        )
    };
    let (output, result) = run(0.1);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(
        findings,
        [(
            "#108".to_owned(),
            "the handrail along the left side stops at the landing between run 1 of 2 and run 2 \
             of 2: ifc-step:model.ifc/#208 and ifc-step:model.ifc/#308 are not joined by \
             selected rails within 0.1 m of each other"
                .to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    // Allowing half a metre, they continue.
    let (output, result) = run(0.5);
    assert_eq!(output.status.code(), Some(0), "{result:#}");
}

/// Ramp flight #108 (as [`ramp_profile`] over 6 m, y -1.2 to 0) with rails
/// #208 beside its left side and #308 beside its right, each reaching
/// 0.3 m level past the run's top onto its upper landing, and space #409,
/// the clear path over that landing (x 7 to 8, y -1 to 0.03), which #208
/// reaches into.
fn a_ramp_with_rails_into_a_path() -> String {
    let ramp = "IFCRAMPFLIGHT('GID',$,$,$,$,PL,REP,$,$)";
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
    let top = [[0.7, 1.0], [1.0, 1.0], [7.0, 1.5], [7.3, 1.5]];
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &ramp_profile(6.0), ramp),
        swept(200, [0.0, 0.05, 0.0], &rail_under(&top), 0.05, rail),
        swept(300, [0.0, -1.2, 0.0], &rail_under(&top), 0.05, rail),
        placed_box(
            400,
            [7.5, -0.485, 0.6],
            [1.0, 1.03, 2.0],
            "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)"
        ),
    )
}

#[test]
fn with_geometry_a_ramp_rail_reaching_into_an_accessible_path_is_found() {
    let case = Case::new("geometry-ramp-rail-obstruction");
    let (output, result) = case.geometry_rule(
        &a_ramp_with_rails_into_a_path(),
        &[
            ("ramp", "IfcRampFlight"),
            ("rail", "IfcRailing"),
            ("space", "IfcSpace"),
        ],
        "axioval:capability.ramp-geometry",
        &registry_signature("axioval:capability.ramp-geometry"),
        entity("ramp"),
        json!({
            "handrail_objects": {"type": "selector", "value": entity("rail")},
            "handrail_reach_across": {"type": "quantity", "value": 0.2, "unit": "m"},
            "handrail_reach_above": {"type": "quantity", "value": 1.5, "unit": "m"},
            "check_rails_obstruction": {"type": "boolean", "value": true},
            "accessible_surface_selector": {"type": "selector", "value": entity("space")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #208 reaches 0.3 m into the path; #308 runs beside it.
    let findings = finding_messages(&result);
    assert_eq!(
        findings,
        [(
            "#108".to_owned(),
            "handrail ifc-step:model.ifc/#208 of the ramp reaches over the accessible surface \
             ifc-step:model.ifc/#409 in plan"
                .to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

fn stepped(points: &[[f64; 2]], cells: &[(Vec<usize>, f64)]) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let mut heights: Vec<Vec<f64>> = vec![vec![0.0]; points.len()];
    for (corners, height) in cells {
        for corner in corners {
            heights[*corner].push(*height);
        }
    }
    let mut positions = Vec::new();
    let mut first = Vec::with_capacity(points.len());
    for (point, levels) in points.iter().zip(&mut heights) {
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        first.push(positions.len());
        positions.extend(levels.iter().map(|z| [point[0], point[1], *z]));
    }
    let vertex = |point: usize, height: f64| {
        first[point]
            + heights[point]
                .iter()
                .position(|z| z.total_cmp(&height).is_eq())
                .unwrap()
    };
    let owner: std::collections::BTreeMap<(usize, usize), f64> = cells
        .iter()
        .flat_map(|(corners, height)| {
            (0..corners.len()).map(|k| ((corners[k], corners[(k + 1) % corners.len()]), *height))
        })
        .collect();
    let mut triangles = Vec::new();
    for (corners, height) in cells {
        for k in 1..corners.len() - 1 {
            triangles.push([
                vertex(corners[0], *height),
                vertex(corners[k], *height),
                vertex(corners[k + 1], *height),
            ]);
            triangles.push([
                vertex(corners[0], 0.0),
                vertex(corners[k + 1], 0.0),
                vertex(corners[k], 0.0),
            ]);
        }
        for k in 0..corners.len() {
            let (a, b) = (corners[k], corners[(k + 1) % corners.len()]);
            let other = owner.get(&(b, a)).copied().unwrap_or(0.0);
            if other >= *height {
                continue;
            }
            let side = |point: usize| -> Vec<usize> {
                heights[point]
                    .iter()
                    .filter(|z| **z >= other && **z <= *height)
                    .map(|z| vertex(point, *z))
                    .collect()
            };
            let (left, right) = (side(a), side(b));
            let (mut i, mut j) = (0, 0);
            while i + 1 < left.len() || j + 1 < right.len() {
                if j + 1 < right.len()
                    && (i + 1 == left.len()
                        || positions[right[j + 1]][2] <= positions[left[i + 1]][2])
                {
                    triangles.push([left[i], right[j], right[j + 1]]);
                    j += 1;
                } else {
                    triangles.push([left[i], right[j], left[i + 1]]);
                    i += 1;
                }
            }
        }
    }
    (positions, triangles)
}

/// Stair flight #105, a quarter turn 1 m wide as one triangulated face set:
/// three straight treads 0.28 m deep along +x, three winders turning left
/// 30° each about the inner corner (0.84, 1), three straight treads along
/// +y, every riser 0.18 m and closed, the last tread its top.
fn quarter_turn_flight() -> String {
    quarter_turn_flight_with("")
}

/// [`quarter_turn_flight`] with more instances.
fn quarter_turn_flight_with(records: &str) -> String {
    let slope = 1.0 / 3.0_f64.sqrt();
    let points = [
        [0.0, 0.0],
        [0.0, 1.0],
        [0.28, 0.0],
        [0.28, 1.0],
        [0.56, 0.0],
        [0.56, 1.0],
        [0.84, 0.0],
        [0.84, 1.0],
        [0.84 + slope, 0.0],
        [1.84, 0.0],
        [1.84, 1.0 - slope],
        [1.84, 1.0],
        [0.84, 1.28],
        [1.84, 1.28],
        [0.84, 1.56],
        [1.84, 1.56],
        [0.84, 1.84],
        [1.84, 1.84],
    ];
    let cells: Vec<(Vec<usize>, f64)> = [
        vec![0, 2, 3, 1],
        vec![2, 4, 5, 3],
        vec![4, 6, 7, 5],
        vec![6, 8, 7],
        vec![8, 9, 10, 7],
        vec![10, 11, 7],
        vec![7, 11, 13, 12],
        vec![12, 13, 15, 14],
        vec![14, 15, 17, 16],
    ]
    .into_iter()
    .zip(1_u32..)
    .map(|(cell, step)| (cell, 0.18 * f64::from(step)))
    .collect();
    let (positions, triangles) = stepped(&points, &cells);
    let coordinates = positions
        .iter()
        .map(|[x, y, z]| format!("({x:?},{y:?},{z:?})"))
        .collect::<Vec<_>>()
        .join(",");
    let indices = triangles
        .iter()
        .map(|[a, b, c]| format!("({},{},{})", a + 1, b + 1, c + 1))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCUNITASSIGNMENT((#6));\n\
         #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
         #100=IFCCARTESIANPOINTLIST3D(({coordinates}));\n\
         #101=IFCTRIANGULATEDFACESET(#100,$,.T.,({indices}),$);\n\
         #102=IFCSHAPEREPRESENTATION(#5,'Body','Tessellation',(#101));\n\
         #103=IFCPRODUCTDEFINITIONSHAPE($,$,(#102));\n\
         #104=IFCLOCALPLACEMENT($,#2);\n\
         #105=IFCSTAIRFLIGHT('{:022}',$,$,$,$,#104,#103,$,$,$,$,$,$);\n\
         {records}ENDSEC;\nEND-ISO-10303-21;\n",
        105
    )
}

/// A side profile as [`swept`], but standing in the vertical plane through
/// `origin` along world y and swept `depth` towards +x.
fn swept_along_y(
    first: u32,
    [x, y, z]: [f64; 3],
    points: &[[f64; 2]],
    depth: f64,
    product: &str,
) -> String {
    let at = |offset: u32| first + offset;
    let mut text = String::new();
    let mut corners = Vec::new();
    for (index, [along, up]) in points.iter().chain(points.first()).enumerate() {
        let corner = first + 20 + u32::try_from(index).unwrap();
        writeln!(text, "#{corner}=IFCCARTESIANPOINT(({along:.3},{up:.3}));").unwrap();
        corners.push(format!("#{corner}"));
    }
    // The profile's x along world y, its normal and so the sweep along
    // world +x: local y is then world up.
    write!(
        text,
        "#{a}=IFCDIRECTION((1.,0.,0.));\n\
         #{b}=IFCDIRECTION((0.,1.,0.));\n\
         #{c}=IFCCARTESIANPOINT(({x:.2},{y:.2},{z:.2}));\n\
         #{d}=IFCAXIS2PLACEMENT3D(#{c},#{a},#{b});\n\
         #{e}=IFCLOCALPLACEMENT($,#2);\n\
         #{f}=IFCPOLYLINE(({}));\n\
         #{g}=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#{f});\n\
         #{h}=IFCDIRECTION((0.,0.,1.));\n\
         #{i}=IFCEXTRUDEDAREASOLID(#{g},#{d},#{h},{depth:.3});\n\
         #{j}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{i}));\n\
         #{k}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{j}));\n\
         #{l}={};\n",
        corners.join(","),
        product
            .replace("GID", &format!("{:022}", at(11)))
            .replace("PL", &format!("#{}", at(4)))
            .replace("REP", &format!("#{}", at(10))),
        a = at(0),
        b = at(1),
        c = at(2),
        d = at(3),
        e = at(4),
        f = at(5),
        g = at(6),
        h = at(7),
        i = at(8),
        j = at(9),
        k = at(10),
        l = at(11),
    )
    .unwrap();
    text
}

/// [`quarter_turn_flight`] with floor slab #209 before its foot (x -1.5
/// to 0, y -0.2 to 1.2, top at 0), landing slab #309 beyond its top tread
/// (x 0.84 to 1.84, y 1.84 to 3, top at 1.62 m) and its outer handrail in
/// two pieces meeting at the corner (1.84, 0): #408 along y = 0 from x
/// -0.3, #511 along x = 1.84 to y 1.86, their top 0.9 m above the nosings'
/// outer ends and level 0.3 m beyond either end.
fn quarter_turn_with_landings_and_rail() -> String {
    let slope = 1.0 / 3.0_f64.sqrt();
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
    let profile = |top: &[[f64; 2]]| {
        let mut points: Vec<[f64; 2]> = top.iter().map(|[a, z]| [*a, z - 0.05]).collect();
        points.extend(top.iter().rev());
        points
    };
    let lower = profile(&[
        [-0.3, 1.08],
        [0.0, 1.08],
        [0.84, 1.62],
        [0.84 + slope, 1.80],
        [1.89, 1.98],
    ]);
    let upper = profile(&[
        [-0.1, 1.98],
        [1.0 - slope, 1.98],
        [1.0, 2.16],
        [1.28, 2.34],
        [1.56, 2.52],
        [1.86, 2.52],
    ]);
    let slab = "IFCSLAB('GID',$,$,$,$,PL,REP,$,.FLOOR.)";
    quarter_turn_flight_with(&format!(
        "#4=IFCDIRECTION((0.,0.,1.));\n\
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}",
        placed_box(200, [-0.75, 0.5, -0.2], [1.5, 1.4, 0.2], slab),
        placed_box(300, [1.34, 2.42, 1.42], [1.0, 1.16, 0.2], slab),
        swept(400, [0.0, -0.05, 0.0], &lower, 0.05, rail),
        swept_along_y(500, [1.84, 0.0, 0.0], &upper, 0.05, rail),
    ))
}

#[test]
fn with_geometry_a_quarter_turns_landings_and_handrail_are_placed_in_its_parts() {
    let case = Case::new("geometry-quarter-turn-ends");
    let (output, result) = case.geometry_rule(
        &quarter_turn_with_landings_and_rail(),
        &[
            ("flight", "IfcStairFlight"),
            ("slab", "IfcSlab"),
            ("rail", "IfcRailing"),
        ],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "landing_objects": {"type": "selector", "value": entity("slab")},
            "landing_depth_minimum": {"type": "quantity", "value": 1.45, "unit": "m"},
            "landing_at_least_walking_width": {"type": "boolean", "value": true},
            "handrail_objects": {"type": "selector", "value": entity("rail")},
            "handrail_reach_across": {"type": "quantity", "value": 0.2, "unit": "m"},
            "handrail_reach_above": {"type": "quantity", "value": 1.5, "unit": "m"},
            "handrail_height_minimum": {"type": "quantity", "value": 0.7, "unit": "m"},
            "handrail_height_maximum": {"type": "quantity", "value": 1.2, "unit": "m"},
            "handrail_extension_minimum": {"type": "quantity", "value": 0.3, "unit": "m"},
            "handrail_gap_maximum": {"type": "quantity", "value": 0.05, "unit": "m"},
            "handrail_sides": {"type": "string", "value": "one"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The floor gives the foot 1.5 m along -x; the top landing reaches
    // 1.44 m along +y from the top tread's nosing. The handrail's pieces
    // meet at the corner, reach 0.3 m level beyond either end and stand
    // high enough wherever the pitch line is known or bracketed.
    assert_eq!(
        finding_messages(&result),
        [(
            "#105".to_owned(),
            "the landing at the top of the flight is 1.44 m deep; at least 1.45 m and the \
             flight's width (1 m) required"
                .to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_a_quarter_turn_flights_winders_are_found_and_its_width_left_open() {
    let case = Case::new("geometry-quarter-turn");
    let (output, result) = case.geometry_rule(
        &quarter_turn_flight(),
        &[("flight", "IfcStairFlight")],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "riser_maximum": {"type": "quantity", "value": 19, "unit": "cm"},
            "going_minimum": {"type": "quantity", "value": 26, "unit": "cm"},
            "winder_angle_maximum": {"type": "quantity", "value": 25, "unit": "deg"},
            "width_minimum": {"type": "quantity", "value": 1.1, "unit": "m"},
            "forbid_open_risers": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The three winders turn 30° each. Risers, goings along the centre line
    // and closed risers pass.
    assert_eq!(
        finding_messages(&result),
        [(
            "#105".to_owned(),
            "winder angle 4 of 8 is 30°, winder angle 5 of 8 is 30°, winder angle 6 of 8 is 30°; \
             at most 25° required"
                .to_owned()
        )],
        "{result:#}"
    );
    // Every straight tread is 1 m wide, but a winder tapers: the flight's
    // width is not measured, so the width check is left open.
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 1, "{result:#}");
    assert!(
        not_evaluated[0]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("the flight's width is not measured")),
        "{result:#}"
    );
}

/// Lobby #19 (x 0..4) and rooms #29 (x 4.2..8), #39 (x -4..-0.2) and #49
/// (above the lobby, floor at 3.3 m), all 4 m deep in y. Door #59, 0.9 m
/// wide, joins the lobby to #29 and states a clear width of 0.85 m; door
/// #69, as wide, joins it to #39 and states 0.75 m. Stair #79 climbs from
/// the lobby to #49 and is the only way up. `Pset_SpaceCommon.Reference`
/// is `Lobby` for #19 and `Room` for the rest.
fn rooms_doors_and_a_stair() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,0.9,$,$,$)";
    let stair = "IFCSTAIR('GID',$,$,$,$,PL,REP,$,$)";
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
         {}{}{}{}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Lobby'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19),#201);\n\
         #210=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Room'),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'Pset_SpaceCommon',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#29,#39,#49),#211);\n\
         #220=IFCPROPERTYSINGLEVALUE('ClearWidth',$,IFCPOSITIVELENGTHMEASURE(0.85),$);\n\
         #221=IFCPROPERTYSET('0000000000000000000221',$,'Access',$,(#220));\n\
         #222=IFCRELDEFINESBYPROPERTIES('0000000000000000000222',$,$,$,(#59),#221);\n\
         #230=IFCPROPERTYSINGLEVALUE('ClearWidth',$,IFCPOSITIVELENGTHMEASURE(0.75),$);\n\
         #231=IFCPROPERTYSET('0000000000000000000231',$,'Access',$,(#230));\n\
         #232=IFCRELDEFINESBYPROPERTIES('0000000000000000000232',$,$,$,(#69),#231);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [6.1, 2.0, 0.0], [3.8, 4.0, 3.0], space),
        placed_box(30, [-2.1, 2.0, 0.0], [3.8, 4.0, 3.0], space),
        placed_box(40, [2.0, 2.0, 3.3], [4.0, 4.0, 3.0], space),
        placed_box(50, [4.1, 1.45, 0.0], [0.1, 0.9, 2.1], door),
        placed_box(60, [-0.1, 1.45, 0.0], [0.1, 0.9, 2.1], door),
        placed_box(70, [2.0, 1.75, 0.0], [1.0, 2.5, 3.3], stair),
    )
}

#[test]
fn with_geometry_an_accessible_route_finds_a_narrow_door_and_a_stairs_only_room() {
    let case = Case::new("geometry-accessible-route");
    let reference = |value: &str| {
        json!({"kind": "allOf", "operands": [
            entity("space"),
            {"kind": "property", "propertySet": "axioval:example.ifc.pset-space-common",
             "property": "axioval:example.ifc.reference", "operator": "equals",
             "value": {"type": "string", "value": value}},
        ]})
    };
    let (output, result) = case.geometry_rule(
        &rooms_doors_and_a_stair(),
        &[
            ("space", "IfcSpace"),
            ("door", "IfcDoor"),
            ("stair", "IfcStair"),
        ],
        "axioval:capability.accessible-route",
        &registry_signature("axioval:capability.accessible-route"),
        reference("Room"),
        json!({
            "route_selector": {"type": "selector", "value": entity("space")},
            "start_selector": {"type": "selector", "value": reference("Lobby")},
            "portal_selector": {"type": "selector", "value": entity("door")},
            "stair_selector": {"type": "selector", "value": entity("stair")},
            "width_metres": {"type": "number", "value": 0.8},
            "door_width_metres": {"type": "number", "value": 0.8},
            "clear_width_property": {"type": "propertyReference",
                                     "property": "axioval:example.ifc.clear-width",
                                     "propertySet": "axioval:example.ifc.pset-access"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #29 is reached through #59 and passes: the stated clear width lets
    // the geometry prove the body through the door.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0]
            .1
            .contains("#69 states a clear width of 0.75 m, less than the 0.8 m required"),
        "{result:#}"
    );
    assert_eq!(findings[1].0, "#49", "{result:#}");
    assert!(
        findings[1]
            .1
            .contains("is connected to the starts by stairs only"),
        "{result:#}"
    );
    let related = |local: &str| -> Vec<String> {
        result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["object_id"]["local_id"] == local)
            .unwrap()["related"]
            .as_array()
            .unwrap()
            .iter()
            .map(|related| related["local_id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(related("#39"), ["#69"]);
    assert_eq!(related("#49"), ["#79"]);
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Lobby #19 (x 0..4) opens through door #49 onto corridor #29 (x 4.2..10,
/// y 1..2.2), which opens through door #59 onto room #39 (x 10.2..14). Two
/// columns, #69 south and #79 north, stand in the corridor at x 7..7.3 and
/// leave 0.4 m between them. Both doors state a clear width of 0.85 m.
fn a_corridor_pinched_by_columns() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,0.9,$,$,$)";
    let column = "IFCCOLUMN('GID',$,$,$,$,PL,REP,$,$)";
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
         {}{}{}{}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Lobby'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19),#201);\n\
         #210=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Room'),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'Pset_SpaceCommon',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#39),#211);\n\
         #220=IFCPROPERTYSINGLEVALUE('ClearWidth',$,IFCPOSITIVELENGTHMEASURE(0.85),$);\n\
         #221=IFCPROPERTYSET('0000000000000000000221',$,'Access',$,(#220));\n\
         #222=IFCRELDEFINESBYPROPERTIES('0000000000000000000222',$,$,$,(#49,#59),#221);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [7.1, 1.6, 0.0], [5.8, 1.2, 3.0], space),
        placed_box(30, [12.1, 2.0, 0.0], [3.8, 4.0, 3.0], space),
        placed_box(40, [4.1, 1.6, 0.0], [0.1, 0.9, 2.1], door),
        placed_box(50, [10.1, 1.6, 0.0], [0.1, 0.9, 2.1], door),
        placed_box(60, [7.15, 1.2, 0.0], [0.3, 0.4, 3.0], column),
        placed_box(70, [7.15, 2.0, 0.0], [0.3, 0.4, 3.0], column),
    )
}

#[test]
fn with_geometry_an_accessible_route_names_where_a_corridor_is_obstructed() {
    let case = Case::new("geometry-accessible-route-pinch");
    let reference = |value: &str| {
        json!({"kind": "allOf", "operands": [
            entity("space"),
            {"kind": "property", "propertySet": "axioval:example.ifc.pset-space-common",
             "property": "axioval:example.ifc.reference", "operator": "equals",
             "value": {"type": "string", "value": value}},
        ]})
    };
    let (output, result) = case.geometry_rule(
        &a_corridor_pinched_by_columns(),
        &[
            ("space", "IfcSpace"),
            ("door", "IfcDoor"),
            ("column", "IfcColumn"),
        ],
        "axioval:capability.accessible-route",
        &registry_signature("axioval:capability.accessible-route"),
        reference("Room"),
        json!({
            "route_selector": {"type": "selector", "value": entity("space")},
            "start_selector": {"type": "selector", "value": reference("Lobby")},
            "portal_selector": {"type": "selector", "value": entity("door")},
            "obstacle_selector": {"type": "selector", "value": entity("column")},
            "width_metres": {"type": "number", "value": 0.8},
            "door_width_metres": {"type": "number", "value": 0.8},
            "obstruction_depth_metres": {"type": "number", "value": 0.01},
            "clear_width_property": {"type": "propertyReference",
                                     "property": "axioval:example.ifc.clear-width",
                                     "propertySet": "axioval:example.ifc.pset-access"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0]
            .1
            .contains("#29 is obstructed near (7.15, 1.6, 0) for a body 0.8 m wide by"),
        "{result:#}"
    );
    let related: Vec<&str> = result["report"]["findings"][0]["related"]
        .as_array()
        .unwrap()
        .iter()
        .map(|related| related["local_id"].as_str().unwrap())
        .collect();
    assert_eq!(related, ["#29", "#69", "#79"], "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Three 2.1 m doors in one wall line, 0.1 m thick at y 0: #19 at x 0..1
/// (`OverallWidth` 1 m) and #29 at x 1.5..2.4 (0.9 m), both single swing;
/// #39 at x 10..11, a double door stating a clear width of 1.15 m and a
/// threshold of 3 cm in the project's `DoorAccessibility` set.
fn doors_in_a_wall() -> String {
    let door = |width: f64, operation: &str| {
        format!("IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,{width:.2},.DOOR.,.{operation}.,$)")
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
         #200=IFCPROPERTYSINGLEVALUE('ClearWidth',$,IFCPOSITIVELENGTHMEASURE(1.15),$);\n\
         #201=IFCPROPERTYSINGLEVALUE('ThresholdHeight',$,IFCLENGTHMEASURE(0.03),$);\n\
         #202=IFCPROPERTYSET('0000000000000000000202',$,'DoorAccessibility',$,(#200,#201));\n\
         #203=IFCRELDEFINESBYPROPERTIES('0000000000000000000203',$,$,$,(#39),#202);\n\
         #210=IFCPROPERTYSINGLEVALUE('ThresholdHeight',$,IFCLENGTHMEASURE(0.),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'DoorAccessibility',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#19,#29),#211);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(
            10,
            [0.5, 0.0, 0.0],
            [1.0, 0.1, 2.1],
            &door(1.0, "SINGLE_SWING_LEFT")
        ),
        placed_box(
            20,
            [1.95, 0.0, 0.0],
            [0.9, 0.1, 2.1],
            &door(0.9, "SINGLE_SWING_RIGHT")
        ),
        placed_box(
            30,
            [10.5, 0.0, 0.0],
            [1.0, 0.1, 2.1],
            &door(1.25, "DOUBLE_DOOR_SINGLE_SWING"),
        ),
    )
}

/// The door compositions of the capability model's "Doors" section as
/// packages: a clear width per door type from a stated property or the
/// overall width less a stated deduction, a stated threshold height, and a
/// minimum plan distance between doors. Returns the definitions and the
/// ruleset.
fn door_packages(case: &Case) -> (PathBuf, PathBuf) {
    let text = |value: &str| json!({"default": value, "translations": {}});
    let concept = |name: &str| {
        json!({"id": format!("axioval:example.ifc.{name}"), "name": text(name),
               "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
               "citations": []})
    };
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(case.definitions(true)).unwrap()).unwrap();
    definitions["objectTypes"]["axioval:example.ifc.IfcDoor"] = concept("IfcDoor");
    definitions["propertySets"]["axioval:example.ifc.DoorAccessibility"] =
        concept("DoorAccessibility");
    for (name, kind) in [
        ("OverallWidth", "quantity"),
        ("OperationType", "enum"),
        ("ClearWidth", "quantity"),
        ("ThresholdHeight", "quantity"),
    ] {
        let mut property = concept(name);
        property["valueKind"] = json!(kind);
        definitions["properties"][format!("axioval:example.ifc.{name}")] = property;
    }
    for (id, capability) in [
        ("door-clear-width", "keyed-limit"),
        ("door-threshold", "keyed-limit"),
        ("door-spacing", "distance"),
    ] {
        let capability = format!("axioval:capability.{capability}");
        definitions["definitions"][format!("axioval:example.{id}")] = json!({
            "id": format!("axioval:example.{id}"), "name": text(id), "description": text(id),
            "capability": capability, "parameters": registry_signature(&capability),
            "citations": [], "tags": [],
        });
    }
    let reference = |set: &str, name: &str| {
        json!({"type": "propertyReference", "property": format!("axioval:example.ifc.{name}"),
               "propertySet": set})
    };
    let accessibility = "axioval:example.ifc.DoorAccessibility";
    let operation = reference("axioval:attributes", "OperationType");
    let doors = entity("IfcDoor");
    let text_file = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text_file).unwrap();
    let template = ruleset["root"]["rules"][0].clone();
    let rule = |id: &str, parameters: Value| {
        let mut rule = template.clone();
        rule["id"] = json!(id);
        rule["definitionId"] = json!(format!("axioval:example.{id}"));
        rule["parameters"] = parameters;
        rule["applicability"]["groups"]["walls"]["selector"] = doors.clone();
        rule
    };
    ruleset["root"]["rules"] = json!([
        rule(
            "door-clear-width",
            json!({
                "limits": {"type": "table", "value": [
                    {"key_1": {"type": "string", "value": "SINGLE_SWING_*"},
                     "minimum": {"type": "number", "value": 0.9}},
                    {"key_1": {"type": "string", "value": "DOUBLE_DOOR_*"},
                     "minimum": {"type": "number", "value": 1.2}},
                ]},
                "quantity": {"type": "string", "value": "clear-width"},
                "quantity_property": reference(accessibility, "ClearWidth"),
                "overall_width": reference("axioval:attributes", "OverallWidth"),
                "width_deduction": {"type": "quantity", "value": 0.1, "unit": "m"},
                "key_1": operation,
            }),
        ),
        rule(
            "door-threshold",
            json!({
                "limits": {"type": "table", "value": [
                    {"key_1": {"type": "string", "value": "*"},
                     "maximum": {"type": "number", "value": 0.02}},
                ]},
                "quantity": {"type": "string", "value": "property"},
                "quantity_property": reference(accessibility, "ThresholdHeight"),
                "key_1": operation,
            }),
        ),
        rule(
            "door-spacing",
            json!({
                "counterparts": {"type": "selector", "value": doors},
                "mode": {"type": "string", "value": "none_closer_than"},
                "projection": {"type": "string", "value": "horizontal"},
                "minimum_metres": {"type": "number", "value": 1.5},
            }),
        ),
    ]);
    (
        case.write("definitions.json", &definitions.to_string()),
        case.write("ruleset.json", &ruleset.to_string()),
    )
}

#[test]
fn with_geometry_door_clear_widths_thresholds_and_spacing_are_checked() {
    let case = Case::new("geometry-doors");
    let (definitions, ruleset) = door_packages(&case);
    let model = case.write("model.ifc", &doors_in_a_wall());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let mut findings: Vec<(String, String, String)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["rule_id"].as_str().unwrap().to_owned(),
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                finding["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    findings.sort();
    let found: Vec<(&str, &str)> = findings
        .iter()
        .map(|(rule, object, _)| (rule.as_str(), object.as_str()))
        .collect();
    // #29 is 0.8 m clear after the deduction; #19 meets 0.9 m exactly and
    // #39's stated 1.15 m is judged instead of its 1.25 m overall width.
    // #39's stated threshold is too high; #19 and #29 are 0.5 m apart.
    assert_eq!(
        found,
        [
            ("door-clear-width", "#29"),
            ("door-clear-width", "#39"),
            ("door-spacing", "#19"),
            ("door-spacing", "#29"),
            ("door-threshold", "#39"),
        ],
        "{result:#}"
    );
    assert_eq!(
        findings[0].2,
        "clear width (axioval:attributes.axioval:example.ifc.OverallWidth 0.9 m less the rule's \
         deduction 0.1 m, an approximation) is 0.8 m; required at least 0.9 m (limit row 0: \
         axioval:attributes.axioval:example.ifc.OperationType `SINGLE_SWING_RIGHT`)",
        "{result:#}"
    );
    assert!(
        findings[1]
            .2
            .starts_with("clear width (axioval:example.ifc.DoorAccessibility.")
            && findings[1].2.contains("is 1.15 m; required at least 1.2 m"),
        "{result:#}"
    );
    assert!(
        findings[2]
            .2
            .ends_with("/#29 at horizontal distance 0.5000 m"),
        "{result:#}"
    );
    assert!(
        findings[4].2.contains("is 0.03 m; required at most 0.02 m"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// A door type's default width deduction replaces the rule's for the doors
/// its row picks, and the finding names it.
#[test]
fn with_geometry_a_door_types_default_deduction_is_named_in_the_finding() {
    let case = Case::new("geometry-door-type-defaults");
    let (definitions, ruleset) = door_packages(&case);
    let mut packaged: Value =
        serde_json::from_str(&std::fs::read_to_string(&ruleset).unwrap()).unwrap();
    packaged["root"]["rules"][0]["parameters"]["door_type_defaults"] = json!({
    "type": "table", "value": [
        {"applies_to": {"type": "selector", "value": {
            "kind": "property", "propertySet": "axioval:attributes",
            "property": "axioval:example.ifc.OperationType", "operator": "equals",
            "value": {"type": "string", "value": "SINGLE_SWING_RIGHT"}}},
         "width_deduction": {"type": "quantity", "value": 0.05, "unit": "m"}},
    ]});
    let ruleset = case.write("ruleset.json", &packaged.to_string());
    let model = case.write("model.ifc", &doors_in_a_wall());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let finding = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| {
            finding["rule_id"] == "door-clear-width" && finding["object_id"]["local_id"] == "#29"
        })
        .unwrap_or_else(|| panic!("{result:#}"));
    // #29 is 0.9 m less its type's 0.05 m: 0.85 m; #19 keeps the rule's
    // 0.1 m and meets 0.9 m.
    assert_eq!(
        finding["message"],
        "clear width (axioval:attributes.axioval:example.ifc.OverallWidth 0.9 m less the door \
         type's default width deduction 0.05 m (door_type_defaults row 0), an approximation) is \
         0.85 m; required at least 0.9 m (limit row 0: \
         axioval:attributes.axioval:example.ifc.OperationType `SINGLE_SWING_RIGHT`)",
        "{result:#}"
    );
    assert!(
        finding["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|evidence| {
                evidence["locator"]
                    .as_str()
                    .is_some_and(|locator| locator.starts_with("axioval:default.door-type:"))
                    && evidence["exact"] == false
            }),
        "{result:#}"
    );
}

#[test]
fn with_geometry_a_forbidden_connection_and_a_missing_exit_are_found() {
    let case = Case::new("geometry-space-connection");
    let (output, result) = case.geometry_rule(
        &walls_with_openings(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.space-connection",
        &registry_signature("axioval:capability.space-connection"),
        entity("space"),
        json!({
            "connections": {"type": "table", "value": [
                {"from": {"type": "selector", "value": entity("space")},
                 "to": {"type": "selector", "value": entity("space")},
                 "access": {"type": "string", "value": "forbidden"},
                 "access_type": {"type": "string", "value": "doors"}},
                {"label": {"type": "string", "value": "exit"},
                 "from": {"type": "selector", "value": entity("space")},
                 "exit": {"type": "string", "value": "required"},
                 "access_type": {"type": "string", "value": "doors"}},
            ]},
            "access_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
            "door_selector": {"type": "selector", "value": entity("door")},
            "space_selector": {"type": "selector", "value": entity("space")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Doors #66 and #116 join the two rooms, which no door may; door #96
    // opens #26 to the outside, while #16 has only a window there.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 3, "{result:#}");
    let of = |id: &str| -> Vec<&str> {
        findings
            .iter()
            .filter(|(object, _)| object == id)
            .map(|(_, message)| message.as_str())
            .collect()
    };
    assert!(
        of("#16")
            .iter()
            .any(|message| message.starts_with("has direct access to ")
                && message.contains("/#26 through ")
                && message.ends_with("which row 0 forbids for a door")),
        "{result:#}"
    );
    assert!(
        of("#16").contains(&"has no door directly to the outside, which row 1 (exit) requires"),
        "{result:#}"
    );
    assert_eq!(of("#26").len(), 1, "{result:#}");
    assert!(of("#26")[0].contains("/#16 through "), "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Rooms #19 (x 0..4) and #29 (x 4.2..8.2), both 4 m deep and 3 m high,
/// joined only by the bodiless opening #39 in the gap between them at
/// y 3..4, so the walk from centre to centre detours north.
fn rooms_through_an_opening() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let opening = "IFCOPENINGELEMENT('GID',$,$,$,$,PL,REP,$,.OPENING.)";
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
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [6.2, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(30, [4.1, 3.5, 0.0], [0.2, 1.0, 2.1], opening),
    )
}

#[test]
fn with_geometry_a_walking_distance_too_long_is_found() {
    let case = Case::new("geometry-space-distance");
    let row = |measure: &str| {
        json!({"from": {"type": "selector", "value": entity("space")},
               "to": {"type": "selector", "value": entity("space")},
               "measure": {"type": "string", "value": measure},
               "maximum": {"type": "number", "value": 4.5}})
    };
    let (output, result) = case.geometry_rule(
        &rooms_through_an_opening(),
        &[("space", "IfcSpace")],
        "axioval:capability.space-distance",
        &registry_signature("axioval:capability.space-distance"),
        entity("space"),
        json!({
            "distances": {"type": "table", "value": [row("straight"), row("walking")]},
            "walking_radius": {"type": "number", "value": 0.3},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The centres lie 4.2 m apart, within 4.5 m; the walk through the
    // opening is at least 2·√5 + 0.2 ≈ 4.67 m.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    for (space, message) in &findings {
        assert!(["#19", "#29"].contains(&space.as_str()), "{result:#}");
        assert!(
            message.starts_with("the nearest destination, ")
                && message.contains(" m away walking; row 1 allows at most 4.5 m"),
            "{result:#}"
        );
    }
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_the_closest_distance_is_measured_between_bodies() {
    let case = Case::new("geometry-space-distance-closest");
    let row = |measure: &str| {
        json!({"from": {"type": "selector", "value": entity("space")},
               "to": {"type": "selector", "value": entity("space")},
               "measure": {"type": "string", "value": measure},
               "maximum": {"type": "number", "value": 1.0}})
    };
    let (output, result) = case.geometry_rule(
        &rooms_through_an_opening(),
        &[("space", "IfcSpace")],
        "axioval:capability.space-distance",
        &registry_signature("axioval:capability.space-distance"),
        entity("space"),
        json!({"distances": {"type": "table", "value": [row("closest"), row("straight")]}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The bodies lie 0.2 m apart, within 1 m; the centres 4.2 m.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    for (_, message) in &findings {
        assert!(
            message.contains(" m away in a straight line between centres; row 1 allows"),
            "{result:#}"
        );
    }
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Two 3 x 3 m washrooms, #19 (x 0 to 3) and #49 (x 4 to 7), each with a
/// WC against its south wall, 0.4 m wide, 0.7 m deep and 0.4 m high: #29
/// at x 1 to 1.4 and #59 at x 5 to 5.4. Vanity unit #39 hangs 0.8 to 1 m
/// above the floor west of #29, inside its transfer area.
fn washrooms() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let wc = "IFCSANITARYTERMINAL('GID',$,$,$,$,PL,REP,$,.TOILETPAN.)";
    let basin = "IFCFURNISHINGELEMENT('GID',$,$,$,$,PL,REP,$)";
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
         {}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [1.5, 1.5, 0.0], [3.0, 3.0, 2.5], space),
        placed_box(20, [1.2, 0.35, 0.0], [0.4, 0.7, 0.4], wc),
        placed_box(30, [0.45, 0.35, 0.8], [0.3, 0.3, 0.2], basin),
        placed_box(40, [5.5, 1.5, 0.0], [3.0, 3.0, 2.5], space),
        placed_box(50, [5.2, 0.35, 0.0], [0.4, 0.7, 0.4], wc),
    )
}

#[test]
fn with_geometry_an_obstructed_wc_transfer_area_is_found() {
    let case = Case::new("geometry-component-clearance");
    let not_a_space = json!({"kind": "not", "operand": entity("space")});
    let (output, result) = case.geometry_rule(
        &washrooms(),
        &[("space", "IfcSpace"), ("terminal", "IfcSanitaryTerminal")],
        "axioval:capability.component-clearance",
        &registry_signature("axioval:capability.component-clearance"),
        entity("terminal"),
        json!({
            "side": {"type": "string", "value": "left"},
            "front_axis": {"type": "string", "value": "forward"},
            "width": {"type": "quantity", "value": 70, "unit": "cm"},
            "depth": {"type": "quantity", "value": 90, "unit": "cm"},
            "height": {"type": "quantity", "value": 2, "unit": "m"},
            "align": {"type": "string", "value": "right"},
            "height_reference": {"type": "string", "value": "floor"},
            "space_path": {"type": "stringList",
                           "value": ["axioval:derived.contained-in-space:forward"]},
            "within_space": {"type": "boolean", "value": true},
            "obstacles": {"type": "selector", "value": not_a_space},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #29's transfer area (x 0.1 to 1, y 0 to 0.7) holds the vanity unit;
    // #59's is clear and lies inside #49.
    assert_eq!(
        finding_messages(&result),
        [(
            "#29".to_owned(),
            "left clearance (0.7 m wide, 0.9 m deep, 2 m high) is obstructed by \
             ifc-step:model.ifc/#39"
                .to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Washroom #19 (x 0..3, y 0..3) with WC #29 (x 1..1.4, y 0..0.7) against
/// the south wall #39 (y -0.2..0), the west wall #49 (x -0.2..0) and a
/// chair #59 north of the WC (x 1.1..1.3, y 1.2..1.4).
fn washroom_with_walls() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let wc = "IFCSANITARYTERMINAL('GID',$,$,$,$,PL,REP,$,.TOILETPAN.)";
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let chair = "IFCFURNISHINGELEMENT('GID',$,$,$,$,PL,REP,$)";
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
         {}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [1.5, 1.5, 0.0], [3.0, 3.0, 2.5], space),
        placed_box(20, [1.2, 0.35, 0.0], [0.4, 0.7, 0.4], wc),
        placed_box(30, [1.5, -0.1, 0.0], [3.4, 0.2, 2.5], wall),
        placed_box(40, [-0.1, 1.5, 0.0], [0.2, 3.4, 2.5], wall),
        placed_box(50, [1.2, 1.3, 0.0], [0.2, 0.2, 0.9], chair),
    )
}

#[test]
fn with_geometry_a_front_is_derived_from_the_wall_behind_a_wc() {
    let case = Case::new("geometry-clearance-against-wall");
    let (output, result) = case.geometry_rule(
        &washroom_with_walls(),
        &[
            ("wall", "IfcWall"),
            ("terminal", "IfcSanitaryTerminal"),
            ("furniture", "IfcFurnishingElement"),
        ],
        "axioval:capability.component-clearance",
        &registry_signature("axioval:capability.component-clearance"),
        entity("terminal"),
        json!({
            "side": {"type": "string", "value": "front"},
            "front_axis": {"type": "string", "value": "against-wall"},
            "wall_selector": {"type": "selector", "value": entity("wall")},
            "wall_reach": {"type": "quantity", "value": 1.5, "unit": "m"},
            "wall_inset": {"type": "quantity", "value": 1, "unit": "cm"},
            "width": {"type": "quantity", "value": 0.8, "unit": "m"},
            "depth": {"type": "quantity", "value": 1.2, "unit": "m"},
            "height": {"type": "quantity", "value": 2, "unit": "m"},
            "height_reference": {"type": "string", "value": "bottom"},
            "obstacles": {"type": "selector", "value": entity("furniture")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#29".to_owned(),
            "front clearance (0.8 m wide, 1.2 m deep, 2 m high) is obstructed by \
             ifc-step:model.ifc/#59"
                .to_owned()
        )],
        "{result:#}"
    );
}

#[test]
fn with_geometry_one_free_side_is_enough_under_any() {
    let case = Case::new("geometry-clearance-any-side");
    let not_a_space = json!({"kind": "not", "operand": entity("space")});
    let run = |quantifier: &str| {
        case.geometry_rule(
            &washrooms(),
            &[("space", "IfcSpace"), ("terminal", "IfcSanitaryTerminal")],
            "axioval:capability.component-clearance",
            &registry_signature("axioval:capability.component-clearance"),
            entity("terminal"),
            json!({
                "sides": {"type": "stringList", "value": ["left", "right"]},
                "quantifier": {"type": "string", "value": quantifier},
                "front_axis": {"type": "string", "value": "forward"},
                "width": {"type": "quantity", "value": 70, "unit": "cm"},
                "depth": {"type": "quantity", "value": 90, "unit": "cm"},
                "height_reference": {"type": "string", "value": "floor"},
                "top_datum": {"type": "string", "value": "floor"},
                "top_offset": {"type": "quantity", "value": 2, "unit": "m"},
                "align": {"type": "string", "value": "right"},
                "space_path": {"type": "stringList",
                               "value": ["axioval:derived.contained-in-space:forward"]},
                "obstacles": {"type": "selector", "value": not_a_space.clone()},
            }),
        )
    };
    // #29's left side holds the vanity unit #39, its right side is free.
    let (output, result) = run("any");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(finding_messages(&result).is_empty(), "{result:#}");
    let (output, result) = run("all");
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#29".to_owned(),
            "left clearance (0.7 m wide, 0.9 m deep, up to 2 m above the floor) is obstructed \
             by ifc-step:model.ifc/#39"
                .to_owned()
        )],
        "{result:#}"
    );
}

#[test]
fn with_geometry_a_wc_axis_far_from_the_side_wall_is_found() {
    let case = Case::new("geometry-centre-line-distance");
    let (output, result) = case.geometry_rule(
        &washroom_with_walls(),
        &[("wall", "IfcWall"), ("terminal", "IfcSanitaryTerminal")],
        "axioval:capability.centre-line-distance",
        &registry_signature("axioval:capability.centre-line-distance"),
        entity("terminal"),
        json!({
            "wall_selector": {"type": "selector", "value": entity("wall")},
            "centre_line": {"type": "string", "value": "against-wall"},
            "sides": {"type": "string", "value": "nearest"},
            "minimum": {"type": "quantity", "value": 405, "unit": "mm"},
            "maximum": {"type": "quantity", "value": 455, "unit": "mm"},
            "reach": {"type": "quantity", "value": 1.5, "unit": "m"},
            "inset": {"type": "quantity", "value": 1, "unit": "cm"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #29's axis (x 1.2) lies 1.2 m from the west wall's face (x 0).
    assert_eq!(
        finding_messages(&result),
        [(
            "#29".to_owned(),
            "centre line to the nearest wall: too far: 1.2 m from ifc-step:model.ifc/#49, more \
             than the maximum 0.455 m"
                .to_owned()
        )],
        "{result:#}"
    );
}

/// As [`placed_box`], with the profile's length turned `degrees` from the
/// x-axis about its centre, as instances `#first` to `#first + 10`; the
/// product is `#first + 10`.
fn turned_box(
    first: u32,
    [x, y, z]: [f64; 3],
    [length, width, depth]: [f64; 3],
    degrees: f64,
    product: &str,
) -> String {
    let [
        origin,
        frame,
        placement,
        p,
        direction,
        pos,
        profile,
        solid,
        shape,
        definition,
        object,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(|offset| first + offset);
    let (sin, cos) = degrees.to_radians().sin_cos();
    format!(
        "#{origin}=IFCCARTESIANPOINT((0.,0.,{z:?}));\n\
         #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
         #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
         #{p}=IFCCARTESIANPOINT(({x:?},{y:?}));\n\
         #{direction}=IFCDIRECTION(({cos:?},{sin:?}));\n\
         #{pos}=IFCAXIS2PLACEMENT2D(#{p},#{direction});\n\
         #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length:?},{width:?});\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth:?});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        product
            .replace("GID", &format!("{object:022}"))
            .replace("PL", &format!("#{placement}"))
            .replace("REP", &format!("#{definition}")),
    )
}

/// The file header, units and project, then `data`.
fn model_with(data: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);\n\
         #8=IFCUNITASSIGNMENT((#6,#7));\n\
         #9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,(#5),#8);\n\
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

/// An aisle slab #19 (x -5..25, y 0..6) with parking spaces north of it,
/// 2.2 m high: #29 (2.5 x 5 m, square to the aisle at x 0..2.5) and #40
/// (2.5 x 4.8 m turned 45 degrees at x 10, its bounding box 5.16 m
/// square). Column #59 stands 0.1 m west of #29's side and #69 0.05 m past
/// its far end.
fn car_park() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,.PARKING.,$)";
    let column = "IFCCOLUMN('GID',$,$,$,$,PL,REP,$,.COLUMN.)";
    let turned_centre = 6.0 + 7.3 / 2.0_f64.sqrt() / 2.0;
    model_with(&format!(
        "{}{}{}{}{}",
        placed_box(
            10,
            [10.0, 3.0, -0.2],
            [30.0, 6.0, 0.2],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.FLOOR.)"
        ),
        placed_box(20, [1.25, 8.5, 0.0], [2.5, 5.0, 2.2], space),
        turned_box(30, [10.0, turned_centre, 0.0], [4.8, 2.5, 2.2], 45.0, space),
        placed_box(50, [-0.3, 8.2, 0.0], [0.4, 0.4, 3.0], column),
        placed_box(60, [1.2, 11.25, 0.0], [0.4, 0.4, 3.0], column),
    ))
}

#[test]
fn with_geometry_a_size_bound_applies_only_to_the_bays_its_states_select() {
    let case = Case::new("geometry-parking-bay-filter");
    let check = |orientations: &[&str]| {
        case.geometry_rule(
            &car_park(),
            &[("space", "IfcSpace"), ("slab", "IfcSlab")],
            "axioval:capability.parking-bay",
            &registry_signature("axioval:capability.parking-bay"),
            entity("space"),
            json!({
                "min_length": {"type": "quantity", "value": 5, "unit": "m"},
                "applies_when": {"type": "string", "value": "filter"},
                "orientations": {"type": "stringList", "value": orientations},
                "aisles": {"type": "selector", "value": entity("slab")},
                "aisle_reach": {"type": "quantity", "value": 0.1, "unit": "m"},
                "angle_tolerance": {"type": "quantity", "value": 5, "unit": "deg"},
            }),
        )
    };
    // #40, at 45 degrees, is angled and 4.8 m long; #29 is perpendicular.
    let (output, result) = check(&["angled"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = finding_messages(&result);
    assert_eq!(found.len(), 1, "{result:#}");
    assert_eq!(found[0].0, "#40", "{result:#}");
    assert!(
        found[0].1.ends_with("(a bay with orientation angled)"),
        "{result:#}"
    );
    let (output, result) = check(&["perpendicular"]);
    assert_eq!(output.status.code(), Some(0), "{result:#}");
}

#[test]
fn with_geometry_parking_spaces_are_checked_along_their_own_axes() {
    let case = Case::new("geometry-parking-bay");
    let (output, result) = case.geometry_rule(
        &car_park(),
        &[
            ("space", "IfcSpace"),
            ("slab", "IfcSlab"),
            ("column", "IfcColumn"),
        ],
        "axioval:capability.parking-bay",
        &registry_signature("axioval:capability.parking-bay"),
        entity("space"),
        json!({
            "min_width": {"type": "quantity", "value": 2.4, "unit": "m"},
            "min_length": {"type": "quantity", "value": 5, "unit": "m"},
            "min_height": {"type": "quantity", "value": 2.1, "unit": "m"},
            "aisles": {"type": "selector", "value": entity("slab")},
            "aisle_reach": {"type": "quantity", "value": 0.1, "unit": "m"},
            "orientation": {"type": "string", "value": "perpendicular"},
            "angle_tolerance": {"type": "quantity", "value": 5, "unit": "deg"},
            "obstacles": {"type": "selector", "value": entity("column")},
            "obstruction_reach": {"type": "quantity", "value": 0.2, "unit": "m"},
            "end_obstructions": {"type": "string", "value": "none"},
            "side_obstructions": {"type": "string", "value": "one"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = finding_messages(&result);
    // #29 is long enough, but a column stands past its far end. #40's
    // bounding box is 5.16 m long; along its own axis it is 4.8 m, and it
    // stands at 45 degrees to the aisle.
    assert_eq!(found.len(), 3, "{result:#}");
    assert_eq!(
        found[0],
        (
            "#29".to_owned(),
            "1 of its ends obstructed by ifc-step:model.ifc/#69, none allowed".to_owned()
        ),
        "{result:#}"
    );
    assert_eq!(found[1].0, "#40");
    assert!(
        found[1]
            .1
            .starts_with("length along the bay's own axes is 4.8 m; at least 5 m"),
        "{found:#?}"
    );
    assert_eq!(found[2].0, "#40");
    assert!(
        found[2]
            .1
            .starts_with("the bay is not perpendicular to any aisle within 0.1 m"),
        "{found:#?}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Storey #101 over slab #19 (10 x 8 m), holding walls 0.2 m thick along
/// x: #29 at y 0..0.2, #39 at y 5..5.2 from x 2, #49 at y 5.6..5.8, and
/// #59 across them at x 0..0.2.
fn walls_on_a_storey() -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    model_with(&format!(
        "{}{}{}{}{}\
         #100=IFCBUILDING('0000000000000000000100',$,'B',$,$,#3,$,$,.ELEMENT.,$,$,$);\n\
         #101=IFCBUILDINGSTOREY('0000000000000000000101',$,'EG',$,$,#3,$,$,.ELEMENT.,0.);\n\
         #103=IFCRELAGGREGATES('0000000000000000000103',$,$,$,#100,(#101));\n\
         #106=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000106',$,$,$,(#19,#29,#39,#49,#59),#101);\n",
        placed_box(
            10,
            [5.0, 4.0, -0.3],
            [10.0, 8.0, 0.3],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.FLOOR.)"
        ),
        placed_box(20, [5.0, 0.1, 0.0], [10.0, 0.2, 3.0], wall),
        placed_box(30, [6.0, 5.1, 0.0], [8.0, 0.2, 3.0], wall),
        placed_box(40, [5.0, 5.7, 0.0], [10.0, 0.2, 3.0], wall),
        placed_box(50, [0.1, 2.9, 0.0], [0.2, 5.4, 3.0], wall),
    ))
}

#[test]
fn with_geometry_close_parallel_walls_and_an_uncovered_strip_are_found() {
    let case = Case::new("geometry-wall-spacing");
    let contained = json!({"type": "stringList",
                           "value": ["IfcRelContainedInSpatialStructure"]});
    let (output, result) = case.geometry_rule(
        &walls_on_a_storey(),
        &[("storey", "IfcBuildingStorey"), ("slab", "IfcSlab")],
        "axioval:capability.wall-spacing",
        &registry_signature("axioval:capability.wall-spacing"),
        entity("storey"),
        json!({
            "members": {"type": "selector", "value": entity("wall")},
            "member_path": contained,
            "angle_tolerance": {"type": "quantity", "value": 5, "unit": "deg"},
            "minimum": {"type": "quantity", "value": 1, "unit": "m"},
            "maximum": {"type": "quantity", "value": 6, "unit": "m"},
            "footprints": {"type": "selector", "value": entity("slab")},
            "footprint_path": contained,
            "uncovered_above": {"type": "quantity", "value": 1, "unit": "m2"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = finding_messages(&result);
    assert_eq!(found.len(), 2, "{result:#}");
    // The bands reach y 5.8 at most: the 2.2 m strip north of #49 is left.
    assert_eq!(found[0].0, "#101");
    assert!(
        found[0].1.starts_with(
            "22 m² of ifc-step:model.ifc/#19 lies outside every band between parallel \
             members at most 6 m apart"
        ),
        "{found:#?}"
    );
    assert!(
        found[1].1.starts_with(
            "ifc-step:model.ifc/#39 and ifc-step:model.ifc/#49 are parallel and 0.4 m apart \
             in plan; at least 1 m required"
        ),
        "{found:#?}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// The crossing walls share 0.2 m × 0.2 m × 3 m = 0.12 m³. A volume
/// tolerance below that keeps the clash; one above it hides it.
#[test]
fn with_geometry_clash_volume_tolerance_is_read_from_the_ifc_bodies() {
    let case = Case::new("clash-volume-tolerance");
    let (output, result) = case.wall_clash(
        &crossing_walls(),
        &json!({"volume_tolerance_cubic_metres": {"type": "number", "value": 0.1}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = sorted_findings(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert!(findings[0].1.contains("sharing 0.120000"), "{findings:?}");

    let (output, result) = case.wall_clash(
        &crossing_walls(),
        &json!({"volume_tolerance_cubic_metres": {"type": "number", "value": 0.13}}),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{result:#}",
        stderr(&output)
    );
    assert!(finding_ids(&result).is_empty(), "{result:#}");
}

/// A 4 m wall #19, 0.3 m thick and 3 m high, holding column #29 0.02 m from
/// its front face and column #39 0.05 m from both faces; column #49 stands
/// free beside it.
fn columns_in_a_wall() -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let column = "IFCCOLUMN('GID',$,$,$,$,PL,REP,$,$)";
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
         {}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 0.15, 0.0], [4.0, 0.3, 3.0], wall),
        placed_box(20, [1.0, 0.12, 0.4], [0.2, 0.2, 2.1], column),
        placed_box(30, [3.0, 0.15, 0.5], [0.2, 0.2, 2.0], column),
        placed_box(40, [6.0, 2.0, 0.0], [0.3, 0.3, 3.0], column),
    )
}

impl Case {
    /// Runs a containment rule of columns in walls with `parameters` added.
    fn containment(&self, parameters: &Value) -> (Output, Value) {
        let mut bound = json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "minimum_volume_ratio": {"type": "number", "value": 0.99},
        });
        for (name, value) in parameters.as_object().unwrap() {
            bound[name] = value.clone();
        }
        self.geometry_rule(
            &columns_in_a_wall(),
            &[("column", "IfcColumn")],
            "axioval:capability.containment",
            &registry_signature("axioval:capability.containment"),
            entity("column"),
            bound,
        )
    }
}

fn cover_row(faces: &str, minimum: f64) -> Value {
    json!({"faces": {"type": "string", "value": faces},
           "minimum_metres": {"type": "number", "value": minimum}})
}

/// The column 0.02 m from the wall face has too little side cover; the
/// other keeps 0.05 m. Both keep 0.5 m under the top, and only the first
/// sits 0.4 m over the bottom where 0.45 m is asked.
#[test]
fn with_geometry_a_column_with_too_little_side_cover_is_found() {
    let case = Case::new("containment-cover");
    let (output, result) = case.containment(&json!({
        "cover": {"type": "table", "value": [
            cover_row("side", 0.04),
            cover_row("top", 0.4),
            cover_row("bottom", 0.45),
        ]},
    }));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    let findings = sorted_findings(&result);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(findings.iter().all(|(id, _)| id == "#29"), "{findings:?}");
    assert!(
        findings.iter().any(
            |(_, message)| message.starts_with("side cover") && message.contains("is 0.0200 m")
        ),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|(_, message)| message.starts_with("bottom cover")
                && message.contains("is 0.4000 m")),
        "{findings:?}"
    );
}

/// Cover findings are nested by two properties of the column: its name,
/// then its object type.
#[test]
fn with_geometry_containment_findings_are_nested_by_two_properties() {
    let case = Case::new("containment-categories");
    let bcf = case.path("issues.bcfzip");
    let case = Case::new("containment-categories");
    let named = columns_in_a_wall().replace(
        "IFCCOLUMN('0000000000000000000029',$,$,$,$,",
        "IFCCOLUMN('0000000000000000000029',$,'C1',$,'Precast',",
    );
    assert_ne!(named, columns_in_a_wall(), "the column template changed");
    case.write("model.ifc", &named);
    let attribute = |id: &str| json!({"propertySet": "axioval:attributes", "property": format!("axioval:example.ifc.{id}")});
    let (output, result) = case.geometry_rule_with(
        &["model.ifc"],
        &[("column", "IfcColumn")],
        (
            "axioval:capability.containment",
            &registry_signature("axioval:capability.containment"),
        ),
        entity("column"),
        json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "minimum_volume_ratio": {"type": "number", "value": 0.99},
            "cover": {"type": "table", "value": [cover_row("side", 0.04)]},
        }),
        &json!({"categories": [attribute("name"), attribute("object-type")]}),
        &["--bcf", bcf.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = sorted_findings(&result);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].0, "#29", "{findings:?}");
    assert!(
        findings[0].1.starts_with("[C1] [Precast] side cover"),
        "{findings:?}"
    );
    // The levels are data too, and label the BCF topic.
    assert_eq!(
        result["report"]["findings"][0]["categories"],
        json!(["C1", "Precast"]),
        "{result:#}"
    );
    let archive = openbim_bcf::read_path(&bcf).unwrap();
    let labels = &archive.topics().next().unwrap().topic.labels;
    assert!(
        labels.contains(&"Category: C1 / Precast".to_owned()),
        "{labels:?}"
    );
}

/// The free-standing column is an orphan, and the wall holds two columns
/// where one is allowed.
#[test]
fn with_geometry_orphans_and_counts_are_found() {
    let case = Case::new("containment-counts");
    let (output, result) = case.containment(&json!({
        "report_orphans": {"type": "boolean", "value": true},
        "maximum_count": {"type": "integer", "value": 1},
    }));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = sorted_findings(&result);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert_eq!(findings[0].0, "#19", "{findings:?}");
    assert_eq!(
        findings[0].1,
        "holds 2 inner elements, more than the maximum 1"
    );
    assert_eq!(findings[1].0, "#49", "{findings:?}");
    assert!(
        findings[1].1.starts_with("lies in no outer element"),
        "{findings:?}"
    );

    let (output, result) = case.containment(&json!({
        "maximum_count": {"type": "integer", "value": 2},
    }));
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{result:#}",
        stderr(&output)
    );
}

#[test]
fn with_geometry_escape_routes_too_long_and_too_few_exits_are_found() {
    // #19 (20 x 10 m) and #29, whose doors lead outside, reach them through
    // the derived adjacency. #29 is sprinklered and needs three exits; the
    // farthest corner of #19 lies about 18.47 m from its nearer door.
    let case = Case::new("geometry-escape-route");
    let sprinklered = json!({"kind": "property",
        "propertySet": "axioval:example.ifc.pset-space-fire-safety",
        "property": "axioval:example.ifc.sprinkler-protection", "operator": "equals",
        "value": {"type": "boolean", "value": true}});
    let (output, result) = case.geometry_rule(
        &halls_with_exits(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"label": {"type": "string", "value": "assembly"},
                 "spaces": {"type": "selector", "value": sprinklered},
                 "exits": {"type": "integer", "value": 3}},
                {"label": {"type": "string", "value": "open plan"},
                 "spaces": {"type": "selector", "value": entity("space")},
                 "maximum_travel": {"type": "number", "value": 15.0},
                 "exits": {"type": "integer", "value": 2}},
            ]},
            "exit_path": {"type": "stringList",
                          "value": ["axioval:derived.adjacent-space:backward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert_eq!(findings[0].0, "#19", "{result:#}");
    assert!(
        findings[0]
            .1
            .starts_with("its farthest point, around (20.00, 0.00), lies ")
            && findings[0].1.contains("18.47")
            && findings[0].1.ends_with(
                " m from the nearest exit walking; use 1 (open plan) allows at most 15 m of \
                 travel"
            ),
        "{result:#}"
    );
    assert_eq!(
        findings[1],
        (
            "#29".to_owned(),
            "has 2 exit(s) via axioval:derived.adjacent-space; use 0 (assembly) requires at \
             least 3"
                .to_owned()
        ),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_travel_over_a_section_counts_by_its_factor() {
    // Plainly the farthest corner of #19 lies about 18.47 m from its nearer
    // door, within 30 m; walked over the hall itself, a section counting
    // twice, it costs about twice that.
    let case = Case::new("geometry-escape-route-weighted");
    let (output, result) = case.geometry_rule(
        &halls_with_exits(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"label": {"type": "string", "value": "open plan"},
                 "spaces": {"type": "selector", "value": entity("space")},
                 "maximum_travel": {"type": "number", "value": 30.0}},
            ]},
            "sections": {"type": "table", "value": [
                {"label": {"type": "string", "value": "slow floor"},
                 "objects": {"type": "selector", "value": entity("space")},
                 "factor": {"type": "number", "value": 2.0}},
            ]},
            "exit_path": {"type": "stringList",
                          "value": ["axioval:derived.adjacent-space:backward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert!(
        findings.iter().any(|(object, message)| object == "#19"
            && message.starts_with("its farthest point, around (20.00, 0.00), lies ")
            && message.contains(
                " m from the nearest exit walking, counting the walk on section 0 (slow floor) \
                 by its factors; use 0 (open plan) allows at most 30 m of travel"
            )),
        "{result:#}"
    );
}

#[test]
fn with_geometry_travel_over_a_section_counts_by_its_factor_while_climbing() {
    // As above, with stairs selected: the walk may climb (the model holds
    // none), and the weighted walk still decides what the plain walk times
    // the factor leaves open.
    let case = Case::new("geometry-escape-route-weighted-stairs");
    let (output, result) = case.geometry_rule(
        &halls_with_exits(),
        &[
            ("door", "IfcDoor"),
            ("space", "IfcSpace"),
            ("stair", "IfcStair"),
        ],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"label": {"type": "string", "value": "open plan"},
                 "spaces": {"type": "selector", "value": entity("space")},
                 "maximum_travel": {"type": "number", "value": 30.0}},
            ]},
            "sections": {"type": "table", "value": [
                {"label": {"type": "string", "value": "slow floor"},
                 "objects": {"type": "selector", "value": entity("space")},
                 "factor": {"type": "number", "value": 2.0}},
            ]},
            "exit_path": {"type": "stringList",
                          "value": ["axioval:derived.adjacent-space:backward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
            "stair_selector": {"type": "selector", "value": entity("stair")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert!(
        findings.iter().any(|(object, message)| object == "#19"
            && message.starts_with("its farthest point, around (20.00, 0.00), lies ")
            && message.contains(
                " m from the nearest exit walking, counting the walk on section 0 (slow floor) \
                 by its factors; use 0 (open plan) allows at most 30 m of travel"
            )),
        "{result:#}"
    );
}

#[test]
fn with_geometry_escape_routes_climb_only_the_stairs_the_rule_selects() {
    // The halls stand on one storey and the model holds no stair: a rule
    // climbing stairs walks the same routes, and the host's own connectors
    // are no way for it.
    let case = Case::new("geometry-escape-route-stairs");
    let (output, result) = case.geometry_rule(
        &halls_with_exits(),
        &[
            ("door", "IfcDoor"),
            ("space", "IfcSpace"),
            ("stair", "IfcStair"),
        ],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"spaces": {"type": "selector", "value": entity("space")},
                 "maximum_travel": {"type": "number", "value": 15.0}},
            ]},
            "exit_path": {"type": "stringList",
                          "value": ["axioval:derived.adjacent-space:backward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
            "stair_selector": {"type": "selector", "value": entity("stair")},
            "stair_length": {"type": "string", "value": "horizontal-plus-vertical"},
            "vertical_factor": {"type": "number", "value": 2.0},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#19", "{result:#}");
    assert!(
        findings[0].1.contains("18.47") && findings[0].1.contains("walking"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_passages_too_narrow_for_their_occupants_are_found() {
    // Both halls (200 m², 100 occupants each at 2 m²) are declared their own
    // passage; the rectangles enclosing them are 10 m wide where 12 m are
    // asked. Their 1 x 0.1 m doors are at most 1.005 m wide where 1.2 m are.
    let case = Case::new("geometry-escape-route-passages");
    let (output, result) = case.geometry_rule(
        &halls_with_exits(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"spaces": {"type": "selector", "value": entity("space")},
                 "area_per_occupant": {"type": "number", "value": 2.0}},
            ]},
            "widths": {"type": "table", "value": [
                {"occupants": {"type": "integer", "value": 500},
                 "width": {"type": "number", "value": 1.2},
                 "passage_width": {"type": "number", "value": 12.0}},
            ]},
            "exit_path": {"type": "stringList",
                          "value": ["axioval:derived.adjacent-space:backward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "passage_selector": {"type": "selector", "value": entity("space")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    let passages: Vec<&(String, String)> = findings
        .iter()
        .filter(|(_, message)| message.starts_with("passage "))
        .collect();
    assert_eq!(passages.len(), 2, "{result:#}");
    for ((object, message), hall) in passages.into_iter().zip(["#19", "#29"]) {
        assert_eq!(object, hall, "{result:#}");
        assert!(
            message.contains(
                " is at most 10 m wide (the shorter side of the rectangle enclosing its \
                 footprint); 100 occupant(s) relying on it (from "
            ) && message.ends_with(") require at least 12 m"),
            "{message}"
        );
    }
    assert_eq!(
        findings
            .iter()
            .filter(|(_, message)| message.starts_with("exit ")
                && message.contains("is at most 1.005 m wide"))
            .count(),
        4,
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// An IFC4 file in metres around `body`, instances `#1` to `#8` taken.
fn metre_model(body: &str) -> String {
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
         {body}\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

/// A prism over the plan polygon `points` (counter-clockwise), from `z` up
/// `height`, as instances `#first` to `#first + 8` and its corners from
/// `#first + 10`. `GID`, `PL` and `REP` in `product` become its `GlobalId`,
/// placement and shape; the product is `#first + 8`.
fn plan_prism(first: u32, points: &[[f64; 2]], z: f64, height: f64, product: &str) -> String {
    let [
        origin,
        frame,
        placement,
        polyline,
        profile,
        solid,
        shape,
        definition,
        object,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8].map(|offset| first + offset);
    let mut text = String::new();
    let mut corners = Vec::new();
    for (index, [x, y]) in points.iter().chain(points.first()).enumerate() {
        let corner = first + 10 + u32::try_from(index).unwrap();
        writeln!(text, "#{corner}=IFCCARTESIANPOINT(({x:.3},{y:.3}));").unwrap();
        corners.push(format!("#{corner}"));
    }
    write!(
        text,
        "#{origin}=IFCCARTESIANPOINT((0.,0.,{z:.3}));\n\
         #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
         #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
         #{polyline}=IFCPOLYLINE(({}));\n\
         #{profile}=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#{polyline});\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{height:.3});\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{object}={};\n",
        corners.join(","),
        product
            .replace("GID", &format!("{object:022}"))
            .replace("PL", &format!("#{placement}"))
            .replace("REP", &format!("#{definition}")),
    )
    .unwrap();
    text
}

/// A plan rectangle from `(x0, y0)` to `(x1, y1)`.
fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> [[f64; 2]; 4] {
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

const SPACE: &str = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";

/// Store room #18 (6 × 4 m, 3 m high) with door #78 (1 m wide) in its
/// south wall, and store room #48 (6 × 4 m, 1.5 m high) without a door.
fn store_rooms() -> String {
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
    metre_model(&format!(
        "{}{}{}",
        plan_prism(10, &rectangle(0.0, 0.0, 6.0, 4.0), 0.0, 3.0, SPACE),
        plan_prism(40, &rectangle(10.0, 0.0, 16.0, 4.0), 0.0, 1.5, SPACE),
        plan_prism(70, &rectangle(2.5, -0.2, 3.5, 0.0), 0.0, 2.1, door),
    ))
}

#[test]
fn with_geometry_shelf_capacity_lays_bands_around_the_door_clearance() {
    let case = Case::new("geometry-shelf-capacity");
    let number = |value: f64| json!({"type": "number", "value": value});
    let (output, result) = case.geometry_rule(
        &store_rooms(),
        &[("space", "IfcSpace"), ("door", "IfcDoor")],
        "axioval:capability.shelf-capacity",
        &registry_signature("axioval:capability.shelf-capacity"),
        entity("space"),
        json!({
            "minimum_running_metres": number(80.0),
            "shelf_depth_metres": number(0.5),
            "horizontal_spacing_metres": number(1.0),
            "vertical_spacing_metres": number(0.5),
            "bottom_elevation_metres": number(0.0),
            "top_elevation_metres": number(2.0),
            "door_clearance_metres": number(1.0),
            "access_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
            "door_selector": {"type": "selector", "value": entity("door")},
            "space_selector": {"type": "selector", "value": entity("space")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #18: four 6 m bands less 3 m beside the door, in four tiers: 84 m.
    // #48: four tiers do not fit under 1.5 m, and three hold 72 m.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert!(
        findings.iter().all(|(space, _)| space == "#48"),
        "{result:#}"
    );
    assert!(
        findings.iter().any(|(_, message)| message
            == "space too low for the shelving: clear height 1.5 m below the shelving's top \
                elevation 2 m"),
        "{result:#}"
    );
    assert!(
        findings
            .iter()
            .any(|(_, message)| message == "shelf running metres 72.000 below required 80.000"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Space #18 (10 × 6 m) with a niche 2 m wide and 1.5 m deep in its south
/// side; space #48 with one 1.5 m wide and 0.5 m deep.
fn spaces_with_niches() -> String {
    let niche = |x: f64, width: f64, depth: f64| {
        vec![
            [x, 0.0],
            [x + 4.0, 0.0],
            [x + 4.0, depth],
            [x + 4.0 + width, depth],
            [x + 4.0 + width, 0.0],
            [x + 10.0, 0.0],
            [x + 10.0, 6.0],
            [x, 6.0],
        ]
    };
    metre_model(&format!(
        "{}{}",
        plan_prism(10, &niche(0.0, 2.0, 1.5), 0.0, 3.0, SPACE),
        plan_prism(40, &niche(20.0, 1.5, 0.5), 0.0, 3.0, SPACE),
    ))
}

#[test]
fn with_geometry_a_recess_too_narrow_for_its_depth_is_found() {
    let case = Case::new("geometry-recess-width");
    let number = |value: f64| json!({"type": "number", "value": value});
    let (output, result) = case.geometry_rule(
        &spaces_with_niches(),
        &[("space", "IfcSpace")],
        "axioval:capability.recess-width",
        &registry_signature("axioval:capability.recess-width"),
        entity("space"),
        json!({
            "requirements": {"type": "table", "value": [
                {"maximum_depth_metres": number(1.0), "minimum_width_metres": number(1.0)},
                {"minimum_depth_metres": number(1.0), "minimum_width_per_depth": number(1.5)},
            ]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#18", "{result:#}");
    assert!(
        findings[0]
            .1
            .ends_with("is 2 m wide and 1.5 m deep; row 1 requires at least 2.25 m"),
        "{result:#}"
    );
}

/// Light wells as zones of stacked 3 m storeys: #300 groups #18, #48 and
/// #78 (3 × 3 m, the middle one shifted 0.5 m east), #310 groups #108 and
/// #138 with a 1 m gap between them, and #320 groups #168 and #198, only
/// 1.5 m wide.
fn light_wells() -> String {
    let space = |first: u32, x: f64, width: f64, z: f64| {
        plan_prism(first, &rectangle(x, 0.0, x + width, 3.0), z, 3.0, SPACE)
    };
    let zone = |id: u32, name: &str, members: &[u32]| {
        let members: Vec<String> = members.iter().map(|member| format!("#{member}")).collect();
        format!(
            "#{id}=IFCZONE('{id:022}',$,'{name}',$,$,$);\n\
             #{}=IFCRELASSIGNSTOGROUP('{:022}',$,$,$,({}),$,#{id});\n",
            id + 1,
            id + 1,
            members.join(",")
        )
    };
    metre_model(&format!(
        "{}{}{}{}{}{}{}{}{}{}{}",
        space(10, 0.0, 3.0, 0.0),
        space(40, 0.5, 3.0, 3.0),
        space(70, 0.0, 3.0, 6.0),
        space(100, 10.0, 3.0, 0.0),
        space(130, 10.0, 3.0, 4.0),
        space(160, 20.0, 1.5, 0.0),
        space(190, 20.0, 1.5, 3.0),
        zone(300, "W1", &[18, 48, 78]),
        zone(310, "W2", &[108, 138]),
        zone(320, "W3", &[168, 198]),
        "",
    ))
}

#[test]
fn with_geometry_light_wells_are_checked_for_contiguity_area_and_width() {
    let case = Case::new("geometry-light-well");
    let number = |value: f64| json!({"type": "number", "value": value});
    let (output, result) = case.geometry_rule(
        &light_wells(),
        &[("space", "IfcSpace"), ("zone", "IfcZone")],
        "axioval:capability.light-well",
        &registry_signature("axioval:capability.light-well"),
        entity("zone"),
        json!({
            "member_path": {"type": "stringList", "value": ["IfcRelAssignsToGroup:forward"]},
            "requirements": {"type": "table", "value": [
                {"maximum_height_metres": number(10.0),
                 "minimum_area_square_metres": number(6.0),
                 "minimum_width_metres": number(2.0)},
            ]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    let of = |zone: &str| -> Vec<&str> {
        findings
            .iter()
            .filter(|(object, _)| object == zone)
            .map(|(_, message)| message.as_str())
            .collect()
    };
    // #300 shares a 2.5 × 3 m section over 9 m: 7.5 m², 2.5 m wide.
    assert!(of("#300").is_empty(), "{result:#}");
    let gap = of("#310");
    assert_eq!(gap.len(), 1, "{result:#}");
    assert!(
        gap[0].contains("starts 1 m above the top of") && gap[0].ends_with("not contiguous"),
        "{result:#}"
    );
    let narrow = of("#320");
    assert_eq!(narrow.len(), 2, "{result:#}");
    assert!(
        narrow[0].starts_with("the well's section area is 4.5 m²; row 0 requires at least 6"),
        "{result:#}"
    );
    assert!(
        narrow[1].starts_with("the well's width is 1.5 m; row 0 requires at least 2"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// External wall #18 along the south edge and internal wall #48 along the
/// north, each declaring `IsExternal`; component #78 stands against the
/// external wall, component #108 against the internal one only.
fn components_along_walls() -> String {
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let proxy = "IFCBUILDINGELEMENTPROXY('GID',$,$,$,$,PL,REP,$,$)";
    let external = |wall: u32, value: &str| {
        let [single, set, rel] = [0, 1, 2].map(|offset| 400 + wall + offset);
        format!(
            "#{single}=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN({value}),$);\n\
             #{set}=IFCPROPERTYSET('{set:022}',$,'Pset_WallCommon',$,(#{single}));\n\
             #{rel}=IFCRELDEFINESBYPROPERTIES('{rel:022}',$,$,$,(#{wall}),#{set});\n"
        )
    };
    metre_model(&format!(
        "{}{}{}{}{}{}",
        plan_prism(10, &rectangle(0.0, -0.3, 10.0, 0.0), 0.0, 3.0, wall),
        plan_prism(40, &rectangle(0.0, 5.0, 10.0, 5.2), 0.0, 3.0, wall),
        plan_prism(70, &rectangle(1.0, 0.0, 2.0, 0.6), 0.0, 1.0, proxy),
        plan_prism(100, &rectangle(4.0, 4.4, 5.0, 5.0), 0.0, 1.0, proxy),
        external(18, ".T."),
        external(48, ".F."),
    ))
}

/// Envelope adjacency is a `distance` rule: each selected component has a
/// wall the model declares external within a tolerance in plan.
#[test]
fn with_geometry_a_component_away_from_every_external_wall_is_found() {
    let case = Case::new("geometry-envelope-adjacency");
    let external_walls = json!({"kind": "allOf", "operands": [
        entity("wall"),
        {"kind": "property", "propertySet": "axioval:example.ifc.pset-wall-common",
         "property": "axioval:example.ifc.is-external", "operator": "equals",
         "value": {"type": "boolean", "value": true}},
    ]});
    let (output, result) = case.geometry_rule(
        &components_along_walls(),
        &[("wall", "IfcWall"), ("proxy", "IfcBuildingElementProxy")],
        "axioval:capability.distance",
        &registry_signature("axioval:capability.distance"),
        entity("proxy"),
        json!({
            "counterparts": {"type": "selector", "value": external_walls},
            "mode": {"type": "string", "value": "nearest"},
            "maximum_metres": {"type": "number", "value": 0.05},
            "projection": {"type": "string", "value": "horizontal"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_ids(&result), ["#108"], "{result:#}");
}

/// The IFC4 header and project shared by the generated visibility and
/// coverage models, around `body`.
fn ifc4_model(body: &str) -> String {
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
         {body}\
         ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

/// Desk #19 at the origin faces door #39 4 m away behind wall #29; desk
/// #49 at x 20 faces door #69 4 m away past column #59, which hides only
/// the door's middle.
fn desks_and_doors() -> String {
    let desk = "IFCFURNISHINGELEMENT('GID',$,$,$,$,PL,REP,$)";
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.,1.,$,$,$)";
    let column = "IFCCOLUMN('GID',$,$,$,$,PL,REP,$,$)";
    ifc4_model(&format!(
        "{}{}{}{}{}{}",
        placed_box(10, [0.0, 0.0, 0.0], [1.0, 0.6, 0.8], desk),
        placed_box(20, [2.0, 0.0, 0.0], [0.2, 6.0, 3.0], wall),
        placed_box(30, [4.0, 0.0, 0.0], [0.1, 1.0, 2.0], door),
        placed_box(40, [20.0, 0.0, 0.0], [1.0, 0.6, 0.8], desk),
        placed_box(50, [22.0, 0.0, 0.0], [0.2, 0.2, 3.0], column),
        placed_box(60, [24.0, 0.0, 0.0], [0.1, 1.0, 2.0], door),
    ))
}

#[test]
fn with_geometry_a_door_hidden_by_a_wall_is_found_and_one_past_a_column_is_seen() {
    let case = Case::new("geometry-component-visibility");
    let (output, result) = case.geometry_rule(
        &desks_and_doors(),
        &[
            ("desk", "IfcFurnishingElement"),
            ("door", "IfcDoor"),
            ("column", "IfcColumn"),
        ],
        "axioval:capability.component-visibility",
        &registry_signature("axioval:capability.component-visibility"),
        entity("desk"),
        json!({
            "targets": {"type": "selector", "value": entity("door")},
            "blockers": {"type": "selector", "value": {"kind": "anyOf", "operands": [
                entity("wall"), entity("column"),
            ]}},
            "eye_height": {"type": "quantity", "value": 1.2, "unit": "m"},
            "radius": {"type": "quantity", "value": 6, "unit": "m"},
            "mode": {"type": "string", "value": "at-least"},
            "minimum": {"type": "integer", "value": 1},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#19".to_owned(),
            "0 target(s) within 6 m of the eye 1.2 m above the base of ifc-step:model.ifc/#19 \
             are in view; required at least 1; 1 hidden"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(
        result["report"]["findings"][0]["related"][0]["local_id"], "#39",
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Room #19 (x 0 to 10, y 0 to 4) with one sprinkler, #29 at (2.5, 2),
/// and wall #59 across it at x 5 to 5.2 up to y 3.5; room #39 (y 10 to 14)
/// with sprinklers #49 at (2.5, 12) and #69 at (7.5, 12).
fn sprinklered_rooms() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let sprinkler = "IFCFIRESUPPRESSIONTERMINAL('GID',$,$,$,$,PL,REP,$,.SPRINKLER.)";
    let wall = "IFCWALL('GID',$,$,$,$,PL,REP,$,$)";
    ifc4_model(&format!(
        "{}{}{}{}{}{}",
        placed_box(10, [5.0, 2.0, 0.0], [10.0, 4.0, 3.0], space),
        placed_box(20, [2.5, 2.0, 2.6], [0.2, 0.2, 0.1], sprinkler),
        placed_box(30, [5.0, 12.0, 0.0], [10.0, 4.0, 3.0], space),
        placed_box(40, [2.5, 12.0, 2.6], [0.2, 0.2, 0.1], sprinkler),
        placed_box(50, [5.1, 1.75, 0.0], [0.2, 3.5, 3.0], wall),
        placed_box(60, [7.5, 12.0, 2.6], [0.2, 0.2, 0.1], sprinkler),
    ))
}

#[test]
fn with_geometry_rooms_are_judged_by_how_much_of_them_their_sprinklers_reach() {
    let case = Case::new("geometry-effective-coverage");
    let types = [
        ("space", "IfcSpace"),
        ("sprinkler", "IfcFireSuppressionTerminal"),
    ];
    let signature = registry_signature("axioval:capability.effective-coverage");
    // Grown by 3 m, #29 reaches just over half of #19; #49 and #69 all of
    // #39 but its corners.
    let (output, result) = case.geometry_rule(
        &sprinklered_rooms(),
        &types,
        "axioval:capability.effective-coverage",
        &signature,
        entity("space"),
        json!({
            "sources": {"type": "selector", "value": entity("sprinkler")},
            "mode": {"type": "string", "value": "grown"},
            "range": {"type": "quantity", "value": 3, "unit": "m"},
            "minimum_ratio": {"type": "number", "value": 0.9},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = finding_messages(&result);
    assert_eq!(found.len(), 1, "{result:#}");
    assert_eq!(found[0].0, "#19");
    assert!(
        found[0].1.starts_with("between 0.5")
            && found[0]
                .1
                .ends_with("(grown by 3 m); required at least 0.9"),
        "{found:#?}"
    );
    // Seen from its centre, #29 sees the west half of #19 and a wedge
    // through the gap past the wall; the wall stands in the way.
    let (output, result) = case.geometry_rule(
        &sprinklered_rooms(),
        &types,
        "axioval:capability.effective-coverage",
        &signature,
        entity("space"),
        json!({
            "sources": {"type": "selector", "value": entity("sprinkler")},
            "mode": {"type": "string", "value": "visible"},
            "range": {"type": "quantity", "value": 20, "unit": "m"},
            "minimum_ratio": {"type": "number", "value": 0.9},
            "blockers": {"type": "selector", "value": entity("wall")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = finding_messages(&result);
    assert_eq!(found.len(), 1, "{result:#}");
    assert_eq!(found[0].0, "#19");
    assert!(found[0].1.contains("(visible by 20 m)"), "{found:#?}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Rooms #19 (x 0 to 4) and #29 (x 4.2 to 8.2), both 4 m deep, joined by
/// the bodiless opening #39 at y 3 to 4, with sprinkler #49 in #29 at
/// (6.2, 3.5).
fn rooms_with_a_sprinkler_next_door() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let opening = "IFCOPENINGELEMENT('GID',$,$,$,$,PL,REP,$,.OPENING.)";
    let sprinkler = "IFCFIRESUPPRESSIONTERMINAL('GID',$,$,$,$,PL,REP,$,.SPRINKLER.)";
    ifc4_model(&format!(
        "{}{}{}{}",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [6.2, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(30, [4.1, 3.5, 0.0], [0.2, 1.0, 2.1], opening),
        placed_box(40, [6.2, 3.5, 2.6], [0.2, 0.2, 0.1], sprinkler),
    ))
}

#[test]
fn with_geometry_an_effect_continues_through_an_opening_into_the_next_room() {
    let case = Case::new("geometry-effective-coverage-connected");
    let types = [
        ("space", "IfcSpace"),
        ("opening", "IfcOpeningElement"),
        ("sprinkler", "IfcFireSuppressionTerminal"),
    ];
    let signature = registry_signature("axioval:capability.effective-coverage");
    let check = |parameters: serde_json::Value| {
        case.geometry_rule(
            &rooms_with_a_sprinkler_next_door(),
            &types,
            "axioval:capability.effective-coverage",
            &signature,
            entity("space"),
            parameters,
        )
    };
    let mut parameters = json!({
        "sources": {"type": "selector", "value": entity("sprinkler")},
        "mode": {"type": "string", "value": "travel"},
        "range": {"type": "quantity", "value": 3, "unit": "m"},
        "minimum_ratio": {"type": "number", "value": 0.05},
    });
    // On its own, #19 holds no sprinkler.
    let (output, result) = check(parameters.clone());
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let found = finding_messages(&result);
    assert_eq!(found.len(), 1, "{result:#}");
    assert_eq!(found[0].0, "#19", "{result:#}");
    // Through the opening, #49's travel reaches more than a twentieth of it.
    parameters["access_path"] = json!({"type": "stringList",
                                      "value": ["axioval:derived.adjacent-space"]});
    parameters["opening_selector"] = json!({"type": "selector", "value": entity("opening")});
    let (output, result) = check(parameters);
    assert_eq!(output.status.code(), Some(0), "{result:#}");
}

/// STEP lines for space boundaries, numbered from 1000 up.
struct Boundaries {
    next: u32,
    data: String,
}

/// Three coordinates as STEP writes them.
fn triple(values: [f64; 3]) -> String {
    format!("({:.1},{:.1},{:.1})", values[0], values[1], values[2])
}

impl Boundaries {
    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// A closed 2D polyline through `points`.
    fn ring(&mut self, points: &[[f64; 2]]) -> u32 {
        let mut refs = Vec::new();
        for point in points {
            let entity = self.id();
            writeln!(
                self.data,
                "#{entity}=IFCCARTESIANPOINT(({:.1},{:.1}));",
                point[0], point[1]
            )
            .unwrap();
            refs.push(format!("#{entity}"));
        }
        let polyline = self.id();
        writeln!(
            self.data,
            "#{polyline}=IFCPOLYLINE(({},{}));",
            refs.join(","),
            refs[0]
        )
        .unwrap();
        polyline
    }

    /// A boundary of `space` against wall #99: an `IfcCurveBoundedPlane`
    /// over the plane at `location` with normal `axis` and first axis
    /// `reference`, bounded by the `size` rectangle from its origin, less
    /// `hole` (`[from u, from v, to u, to v]`).
    fn add(
        &mut self,
        space: u32,
        location: [f64; 3],
        axis: [f64; 3],
        reference: [f64; 3],
        size: [f64; 2],
        hole: Option<[f64; 4]>,
    ) {
        let [origin, normal, first, frame, plane] = [0; 5].map(|_| self.id());
        writeln!(
            self.data,
            "#{origin}=IFCCARTESIANPOINT({});",
            triple(location)
        )
        .unwrap();
        writeln!(self.data, "#{normal}=IFCDIRECTION({});", triple(axis)).unwrap();
        writeln!(self.data, "#{first}=IFCDIRECTION({});", triple(reference)).unwrap();
        writeln!(
            self.data,
            "#{frame}=IFCAXIS2PLACEMENT3D(#{origin},#{normal},#{first});"
        )
        .unwrap();
        writeln!(self.data, "#{plane}=IFCPLANE(#{frame});").unwrap();
        let [width, height] = size;
        let outer = self.ring(&[[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]);
        let inner = hole
            .map(|[left, bottom, right, top]| {
                let ring = self.ring(&[[left, bottom], [right, bottom], [right, top], [left, top]]);
                format!("#{ring}")
            })
            .unwrap_or_default();
        let [bounded, connection, relation] = [0; 3].map(|_| self.id());
        writeln!(
            self.data,
            "#{bounded}=IFCCURVEBOUNDEDPLANE(#{plane},#{outer},({inner}));"
        )
        .unwrap();
        writeln!(
            self.data,
            "#{connection}=IFCCONNECTIONSURFACEGEOMETRY(#{bounded},$);"
        )
        .unwrap();
        writeln!(
            self.data,
            "#{relation}=IFCRELSPACEBOUNDARY('{relation:022}',$,$,$,#{space},#99,\
             #{connection},.PHYSICAL.,.INTERNAL.);"
        )
        .unwrap();
    }
}

/// A 4 x 3 x 2.5 m space box whose entities start at `first`, placed at
/// (`x`, 5); the space itself is `first + 9`.
fn placed_space(first: u32, x: f64) -> String {
    let [
        origin,
        frame,
        placement,
        centre,
        position,
        profile,
        solid,
        shape,
        definition,
        space,
    ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map(|offset| first + offset);
    format!(
        "#{origin}=IFCCARTESIANPOINT(({x:.1},5.,0.));\n\
         #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
         #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
         #{centre}=IFCCARTESIANPOINT((2.,1.5));\n\
         #{position}=IFCAXIS2PLACEMENT2D(#{centre},$);\n\
         #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{position},4.,3.);\n\
         #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,2.5);\n\
         #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
         #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
         #{space}=IFCSPACE('{space:022}',$,$,$,$,#{placement},#{definition},$,.ELEMENT.,$,$);\n"
    )
}

/// Space #39, placed at (10, 5), bounded by `IfcCurveBoundedPlane`
/// connection surfaces stated in its own coordinates: a floor with a 1 m²
/// hole, the ceiling and three walls; its east wall has no boundary. Space
/// #69, placed at (20, 5), has one boundary given as a face surface: a
/// triangle of half its floor. Every boundary bounds against wall #99.
fn spaces_with_boundaries() -> String {
    let mut boundaries = Boundaries {
        next: 1000,
        data: String::new(),
    };
    let (up, south, west) = ([0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0]);
    let (east, north) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    let floor_hole = Some([1.0, 1.0, 2.0, 2.0]);
    boundaries.add(39, [0.0; 3], up, east, [4.0, 3.0], floor_hole);
    boundaries.add(39, [0.0, 0.0, 2.5], up, east, [4.0, 3.0], None);
    boundaries.add(39, [0.0; 3], south, east, [4.0, 2.5], None);
    boundaries.add(39, [0.0, 3.0, 0.0], south, east, [4.0, 2.5], None);
    boundaries.add(39, [0.0; 3], west, north, [3.0, 2.5], None);
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
         #80=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #81=IFCCARTESIANPOINT((4.,0.,0.));\n\
         #82=IFCCARTESIANPOINT((4.,3.,0.));\n\
         #83=IFCPOLYLOOP((#80,#81,#82));\n\
         #84=IFCFACEOUTERBOUND(#83,.T.);\n\
         #85=IFCPLANE(#2);\n\
         #86=IFCFACESURFACE((#84),#85,.T.);\n\
         #87=IFCCONNECTIONSURFACEGEOMETRY(#86,$);\n\
         #88=IFCRELSPACEBOUNDARY('0000000000000000000088',$,$,$,#69,#99,#87,.PHYSICAL.,.INTERNAL.);\n\
         {}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_space(30, 10.0),
        placed_space(60, 20.0),
        placed_box(
            90,
            [30.0, 0.0, 0.0],
            [4.0, 0.2, 3.0],
            "IFCWALL('GID',$,$,$,$,PL,REP,$,.STANDARD.)"
        ),
        boundaries.data,
    )
}

#[test]
fn with_geometry_space_boundaries_are_measured_against_the_space_surface() {
    let case = Case::new("geometry-space-boundary-coverage");
    let (output, result) = case.geometry_rule(
        &spaces_with_boundaries(),
        &[("space", "IfcSpace")],
        "axioval:capability.space-boundary-coverage",
        &registry_signature("axioval:capability.space-boundary-coverage"),
        entity("space"),
        json!({
            "minimum_covered_share": {"type": "number", "value": 0.9},
            "maximum_uncovered_area": {"type": "quantity", "value": 0.5, "unit": "m2"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The hole in the floor and the east wall stay uncovered: 1 + 7.5 m² of
    // the 59 m² surface, measured exactly in the space's own placement. The
    // face surface of #69 is measured too: its 6 m² triangle leaves 53 m².
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#39".to_owned(),
                "declared boundaries cover 85.59% of the 59 m² surface, leaving 8.5 m² \
                 uncovered; at least 90% required"
                    .to_owned()
            ),
            (
                "#39".to_owned(),
                "declared boundaries leave 8.5 m² of the 59 m² surface uncovered; at most \
                 0.5 m² allowed"
                    .to_owned()
            ),
            (
                "#69".to_owned(),
                "declared boundaries cover 10.17% of the 59 m² surface, leaving 53 m² \
                 uncovered; at least 90% required"
                    .to_owned()
            ),
            (
                "#69".to_owned(),
                "declared boundaries leave 53 m² of the 59 m² surface uncovered; at most \
                 0.5 m² allowed"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert!(
        result["report"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["evidence"][0]["exact"] == json!(true)),
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Metres. Columns #10 (an HEA300, 290 mm deep) and #20 (named HEA300 but
/// 295 mm deep) are I-sections extruded up; #30 is an arbitrary outline.
fn profiled_columns() -> String {
    let column = |id: u32, profile: &str| {
        format!(
            "#{a}={profile};\n#{b}=IFCEXTRUDEDAREASOLID(#{a},#2,#4,3.);\n\
             #{c}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{b}));\n\
             #{d}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{c}));\n\
             #{id}=IFCCOLUMN('00000000000000000000{id:02}',$,$,$,$,#3,#{d},$,.COLUMN.);\n",
            a = id + 1,
            b = id + 2,
            c = id + 3,
            d = id + 4,
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
         #9=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
         #7=IFCUNITASSIGNMENT((#6,#9));\n\
         #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
         {}{}\
         #40=IFCCARTESIANPOINT((0.,0.));\n\
         #41=IFCCARTESIANPOINT((0.3,0.));\n\
         #42=IFCCARTESIANPOINT((0.,0.3));\n\
         #43=IFCPOLYLINE((#40,#41,#42,#40));\n\
         {}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        column(
            10,
            "IFCISHAPEPROFILEDEF(.AREA.,'HEA300',$,0.3,0.29,0.0085,0.014,0.027,$,$)"
        ),
        column(
            20,
            "IFCISHAPEPROFILEDEF(.AREA.,'HEA300',$,0.3,0.295,0.0085,0.014,0.027,$,$)"
        ),
        column(30, "IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#43)"),
    )
}

#[test]
fn column_profiles_are_checked_against_a_table_of_allowed_profiles() {
    let case = Case::new("allowed-profile");
    let mut signature = registry_signature("axioval:capability.allowed-profile");
    for column in signature["profiles"]["columns"].as_array_mut().unwrap() {
        if column["kind"] == "quantity" {
            column["unitDimension"] = json!("length");
        }
    }
    let millimetres = |value: f64| json!({"type": "quantity", "value": value, "unit": "mm"});
    let (output, result) = case.geometry_rule(
        &profiled_columns(),
        &[("column", "IfcColumn")],
        "axioval:capability.allowed-profile",
        &signature,
        entity("column"),
        json!({
            "profiles": {"type": "table", "value": [
                {"type": {"type": "string", "value": "i-shape"},
                 "name": {"type": "string", "value": "HEA*"},
                 "width": millimetres(300.0), "depth": millimetres(290.0)},
            ]},
            "tolerance": millimetres(1.0),
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#20".to_owned(),
                "profile `i-shape` `HEA300` is not an allowed profile; nearest is row 1 \
                 (`i-shape` `HEA*`): depth 0.295 m, allowed 0.29 m within 0.001 m"
                    .to_owned()
            ),
            (
                "#30".to_owned(),
                "arbitrary profile `arbitrary-closed`: no allowed profile is of its type"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Metres. Wall #10, 5 m long along x from the origin, 0.2 m thick and 3 m
/// high, holds rectangular openings #100 (x 1.5 to 2.5), #200 (x 3.1 to
/// 4.1) and #300 (x 4.2 to 5.2, 0.1 m from #200 and past the wall's end), each 1.2 m high from
/// z 0.9. Beam #50, a 300 mm I-section with 20 mm flanges running 6 m along
/// x at z 3, has round holes #400 (mid-span), #500 (0.2 m from its start)
/// and #600 (reaching into the top flange).
fn hosts_with_openings() -> String {
    let opening = |id: u32, host: u32, profile: &str, at: [f64; 3], axis: &str, depth: f64| {
        format!(
            "#{a}={profile};\n#{b}=IFCCARTESIANPOINT(({},{},{}));\n\
             #{c}=IFCAXIS2PLACEMENT3D(#{b},{axis});\n\
             #{d}=IFCEXTRUDEDAREASOLID(#{a},#{c},#4,{depth});\n\
             #{e}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{d}));\n\
             #{f}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{e}));\n\
             #{id}=IFCOPENINGELEMENT('{id:022}',$,$,$,$,#3,#{f},$,.OPENING.);\n\
             #{g}=IFCRELVOIDSELEMENT('{g:022}',$,$,$,#{host},#{id});\n",
            at[0],
            at[1],
            at[2],
            a = id + 1,
            b = id + 2,
            c = id + 3,
            d = id + 4,
            e = id + 5,
            f = id + 6,
            g = id + 7,
        )
    };
    // Extruded along -y through the wall, the profile's Y axis up.
    let window = |id: u32, x: f64| {
        opening(
            id,
            10,
            "IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1.,1.2)",
            [x, 0.1, 1.5],
            "#6,#7",
            0.2,
        )
    };
    // Extruded along +y through the web.
    let hole = |id: u32, x: f64, z: f64| {
        opening(
            id,
            50,
            "IFCCIRCLEPROFILEDEF(.AREA.,$,$,0.05)",
            [x, -0.2, 3.0 + z],
            "#8,#7",
            0.4,
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCDIRECTION((0.,-1.,0.));\n\
         #7=IFCDIRECTION((1.,0.,0.));\n\
         #8=IFCDIRECTION((0.,1.,0.));\n\
         #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #21=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
         #22=IFCUNITASSIGNMENT((#20,#21));\n\
         #23=IFCPROJECT('0000000000000000000023',$,'P',$,$,$,$,(#5),#22);\n\
         #11=IFCCARTESIANPOINT((2.5,0.));\n\
         #12=IFCAXIS2PLACEMENT2D(#11,$);\n\
         #13=IFCRECTANGLEPROFILEDEF(.AREA.,$,#12,5.,0.2);\n\
         #14=IFCEXTRUDEDAREASOLID(#13,#2,#4,3.);\n\
         #15=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#14));\n\
         #16=IFCPRODUCTDEFINITIONSHAPE($,$,(#15));\n\
         #10=IFCWALL('0000000000000000000010',$,$,$,$,#3,#16,$,.STANDARD.);\n\
         #51=IFCISHAPEPROFILEDEF(.AREA.,'I300',$,0.3,0.3,0.01,0.02,$,$,$);\n\
         #52=IFCCARTESIANPOINT((0.,0.,3.));\n\
         #55=IFCAXIS2PLACEMENT3D(#52,#7,#8);\n\
         #56=IFCEXTRUDEDAREASOLID(#51,#55,#4,6.);\n\
         #57=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#56));\n\
         #58=IFCPRODUCTDEFINITIONSHAPE($,$,(#57));\n\
         #50=IFCBEAM('0000000000000000000050',$,$,$,$,#3,#58,$,.BEAM.);\n\
         {}{}{}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        window(100, 2.0),
        window(200, 3.6),
        window(300, 4.7),
        hole(400, 3.0, 0.0),
        hole(500, 0.2, 0.0),
        hole(600, 4.5, 0.1),
    )
}

fn opening_zone(name: &str, host: &str, parameters: Value) -> (Output, Value) {
    let case = Case::new(name);
    let mut parameters = parameters;
    parameters["host_path"] =
        json!({"type": "stringList", "value": ["IfcRelVoidsElement:backward"]});
    parameters["host_selector"] = json!({"type": "selector", "value": entity(host)});
    case.geometry_rule(
        &hosts_with_openings(),
        &[
            ("opening", "IfcOpeningElement"),
            ("wall", "IfcWall"),
            ("beam", "IfcBeam"),
        ],
        "axioval:capability.opening-zone",
        &registry_signature("axioval:capability.opening-zone"),
        entity("opening"),
        parameters,
    )
}

#[test]
fn wall_openings_are_checked_against_the_walls_outline_and_each_other() {
    let metres = |value: f64| json!({"type": "quantity", "value": value, "unit": "m"});
    let (output, result) = opening_zone(
        "opening-zone-wall",
        "wall",
        json!({
            "length_axis": {"type": "string", "value": "profile-x"},
            "height_axis": {"type": "string", "value": "extrusion"},
            "end_distance": metres(0.5),
            "edge_distance": metres(0.5),
            "opening_spacing": metres(0.8),
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#100".to_owned(),
                "opening is 0.6 m clear of another opening in its host #10; 0.8 m required"
                    .to_owned()
            ),
            (
                "#200".to_owned(),
                "opening is 0.1 m clear of another opening in its host #10; 0.8 m required"
                    .to_owned()
            ),
            (
                "#300".to_owned(),
                "opening is 0.1 m clear of another opening in its host #10; 0.8 m required"
                    .to_owned()
            ),
            (
                "#300".to_owned(),
                "opening lies partly outside its host #10: along its length it spans 1.7 m to \
                 2.7 m, the host -2.5 m to 2.5 m"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn wall_openings_are_checked_against_the_dimensioning_table() {
    let (output, result) = opening_zone(
        "opening-zone-dimensions",
        "wall",
        json!({
            "length_axis": {"type": "string", "value": "profile-x"},
            "height_axis": {"type": "string", "value": "extrusion"},
            "dimensions": {"type": "table", "value": [
                {"name": {"type": "string", "value": "window to side"},
                 "source": {"type": "selector", "value": entity("opening")},
                 "edge": {"type": "string", "value": "side"},
                 "minimum": {"type": "quantity", "value": 1.0, "unit": "m"}},
            ]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#200".to_owned(),
                "opening is 0.9 m from the side of its host #10 along its length; at least 1 m \
                 required (dimension `window to side`)"
                    .to_owned()
            ),
            (
                "#300".to_owned(),
                "opening lies partly outside its host #10: along its length it spans 1.7 m to \
                 2.7 m, the host -2.5 m to 2.5 m"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn beam_holes_are_checked_against_the_web_zone_and_the_beams_ends() {
    let (output, result) = opening_zone(
        "opening-zone-beam",
        "beam",
        json!({
            "length_axis": {"type": "string", "value": "extrusion"},
            "height_axis": {"type": "string", "value": "profile-y"},
            "end_distance": {"type": "quantity", "value": 300.0, "unit": "mm"},
            "zone": {"type": "string", "value": "web"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#500".to_owned(),
                "opening is 0.15 m from an end of its host #50; 0.3 m required".to_owned()
            ),
            (
                "#600".to_owned(),
                "opening reaches 0.02 m into the flanges of its host #50; 0 m clear required"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// A 300 mm I-beam #50 (10 mm web, 20 mm flanges) running 6 m along x at
/// 3 m, and 0.1 m square ducts extruded 2 m along +y through its web with
/// no void modelled: #100 at x = 3 m, #200 at x = 0.2 m, and #300 beside
/// the beam's end.
fn beam_with_ducts() -> String {
    let duct = |id: u32, x: f64| {
        format!(
            "#{a}=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,0.1,0.1);\n\
             #{b}=IFCCARTESIANPOINT(({x},-1.,3.));\n\
             #{c}=IFCAXIS2PLACEMENT3D(#{b},#8,#7);\n\
             #{d}=IFCEXTRUDEDAREASOLID(#{a},#{c},#4,2.);\n\
             #{e}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{d}));\n\
             #{f}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{e}));\n\
             #{id}=IFCDUCTSEGMENT('{id:022}',$,$,$,$,#3,#{f},$,.RIGIDSEGMENT.);\n",
            a = id + 1,
            b = id + 2,
            c = id + 3,
            d = id + 4,
            e = id + 5,
            f = id + 6,
        )
    };
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #7=IFCDIRECTION((1.,0.,0.));\n\
         #8=IFCDIRECTION((0.,1.,0.));\n\
         #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #21=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
         #22=IFCUNITASSIGNMENT((#20,#21));\n\
         #23=IFCPROJECT('0000000000000000000023',$,'P',$,$,$,$,(#5),#22);\n\
         #51=IFCISHAPEPROFILEDEF(.AREA.,'I300',$,0.3,0.3,0.01,0.02,$,$,$);\n\
         #52=IFCCARTESIANPOINT((0.,0.,3.));\n\
         #55=IFCAXIS2PLACEMENT3D(#52,#7,#8);\n\
         #56=IFCEXTRUDEDAREASOLID(#51,#55,#4,6.);\n\
         #57=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#56));\n\
         #58=IFCPRODUCTDEFINITIONSHAPE($,$,(#57));\n\
         #50=IFCBEAM('0000000000000000000050',$,$,$,$,#3,#58,$,.BEAM.);\n\
         {}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        duct(100, 3.0),
        duct(200, 0.2),
        duct(300, 6.5),
    )
}

#[test]
fn with_geometry_ducts_through_a_beam_without_voids_are_checked_in_the_beam() {
    let case = Case::new("opening-zone-penetration");
    let (output, result) = case.geometry_rule(
        &beam_with_ducts(),
        &[("duct", "IfcDuctSegment"), ("beam", "IfcBeam")],
        "axioval:capability.opening-zone",
        &registry_signature("axioval:capability.opening-zone"),
        entity("duct"),
        json!({
            "host_path": {"type": "stringList", "value": ["axioval:derived.intersects"]},
            "host_selector": {"type": "selector", "value": entity("beam")},
            "length_axis": {"type": "string", "value": "extrusion"},
            "height_axis": {"type": "string", "value": "profile-y"},
            "end_distance": {"type": "quantity", "value": 0.3, "unit": "m"},
            "edge_distance": {"type": "quantity", "value": 0.05, "unit": "m"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#200".to_owned(),
            "opening is 0.15 m from an end of its host #50; 0.3 m required".to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
    // The I-beam, without fillets, meshes exactly, like the ducts.
    assert_eq!(result["geometry"]["exact"], 4, "{result:#}");
    assert_eq!(result["geometry"]["unmeasured"], json!([]), "{result:#}");
    let cited = result.to_string();
    assert!(
        cited.contains("axioval:derived.intersects:"),
        "the derivation is cited: {result:#}"
    );
}

#[test]
fn with_geometry_a_duct_in_either_of_two_allowed_zones_passes() {
    let run = |name: &str, ends: f64| {
        let metres = |value: f64| json!({"type": "quantity", "value": value, "unit": "m"});
        Case::new(name).geometry_rule(
            &beam_with_ducts(),
            &[("duct", "IfcDuctSegment"), ("beam", "IfcBeam")],
            "axioval:capability.opening-zone",
            &registry_signature("axioval:capability.opening-zone"),
            entity("duct"),
            json!({
                "host_path": {"type": "stringList", "value": ["axioval:derived.intersects"]},
                "host_selector": {"type": "selector", "value": entity("beam")},
                "length_axis": {"type": "string", "value": "extrusion"},
                "height_axis": {"type": "string", "value": "profile-y"},
                "zones": {"type": "table", "value": [
                    {"name": {"type": "string", "value": "middle"},
                     "end_fraction": {"type": "number", "value": 0.4}},
                    {"name": {"type": "string", "value": "ends"},
                     "end_minimum": metres(ends),
                     "top_minimum": metres(0.05)},
                ]},
            }),
        )
    };
    let (output, result) = run("opening-zone-zones", 0.1);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(finding_messages(&result), [], "{result:#}");
    let (output, result) = run("opening-zone-zones-strict", 0.2);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#200".to_owned(),
            "opening lies outside any of the 2 allowed zones of its host #50: nearest is zone \
             `ends`, where it is 0.15 m from an end (0.2 m required)"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Walls #1 and #2 with an enumerated `Status`, a bounded `Span` in
/// millimetres and a table `Load` in `Pset_Kinds`, and a `Pset_Checks`
/// with `CheckA`/`CheckB`; wall #2 also has a `Pset_Draft`.
const PROPERTY_KINDS: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
     #90=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n\
     #91=IFCUNITASSIGNMENT((#90));\n\
     #92=IFCPROJECT('0000000000000000000092',$,'P',$,$,$,$,$,#91);\n\
     #1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
     #2=IFCWALL('0000000000000000000002',$,$,$,$,$,$,$,$);\n\
     #3=IFCPROPERTYENUMERATION('Status',(IFCLABEL('NEW'),IFCLABEL('EXISTING'),IFCLABEL('DEMOLISH')),$);\n\
     #4=IFCPROPERTYENUMERATEDVALUE('Status',$,(IFCLABEL('NEW')),#3);\n\
     #5=IFCPROPERTYENUMERATEDVALUE('Status',$,(IFCLABEL('EXISTING'),IFCLABEL('DEMOLISH')),#3);\n\
     #6=IFCPROPERTYBOUNDEDVALUE('Span',$,IFCLENGTHMEASURE(5000.),IFCLENGTHMEASURE(1000.),$,$);\n\
     #7=IFCPROPERTYBOUNDEDVALUE('Span',$,$,IFCLENGTHMEASURE(1000.),$,$);\n\
     #8=IFCPROPERTYTABLEVALUE('Load',$,(IFCREAL(1.),IFCREAL(2.)),(IFCREAL(10.),IFCREAL(20.)),$,$,$,$);\n\
     #10=IFCPROPERTYSET('0000000000000000000010',$,'Pset_Kinds',$,(#4,#6,#8));\n\
     #11=IFCPROPERTYSET('0000000000000000000011',$,'Pset_Kinds',$,(#5,#7));\n\
     #12=IFCRELDEFINESBYPROPERTIES('0000000000000000000012',$,$,$,(#1),#10);\n\
     #13=IFCRELDEFINESBYPROPERTIES('0000000000000000000013',$,$,$,(#2),#11);\n\
     #20=IFCPROPERTYSINGLEVALUE('CheckA',$,IFCLABEL('ok'),$);\n\
     #21=IFCPROPERTYSINGLEVALUE('CheckB',$,IFCLABEL(''),$);\n\
     #22=IFCPROPERTYSET('0000000000000000000022',$,'Pset_Checks',$,(#20,#21));\n\
     #23=IFCRELDEFINESBYPROPERTIES('0000000000000000000023',$,$,$,(#1,#2),#22);\n\
     #24=IFCPROPERTYSINGLEVALUE('Note',$,IFCTEXT('temporary'),$);\n\
     #25=IFCPROPERTYSET('0000000000000000000025',$,'Pset_Draft',$,(#24));\n\
     #26=IFCRELDEFINESBYPROPERTIES('0000000000000000000026',$,$,$,(#2),#25);\n\
     ENDSEC;\nEND-ISO-10303-21;\n";

/// Packages with one rule per `(id, capability, parameters)`, over walls.
/// `Status`, `Span` and `Load` in `Pset_Kinds` are concepts bound to IFC4.
fn property_packages(case: &Case, rules: &[(&str, &str, Value)]) -> (PathBuf, PathBuf) {
    let text = |value: &str| json!({"default": value, "translations": {}});
    let concept = |name: &str| {
        json!({"id": format!("axioval:example.ifc.{name}"), "name": text(name),
               "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
               "citations": []})
    };
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(case.definitions(true)).unwrap()).unwrap();
    definitions["propertySets"]["axioval:example.ifc.Pset_Kinds"] = concept("Pset_Kinds");
    for name in ["Status", "Span", "Load"] {
        let mut property = concept(name);
        property["valueKind"] = json!("string");
        definitions["properties"][format!("axioval:example.ifc.{name}")] = property;
    }
    for (id, capability, _) in rules {
        let capability = format!("axioval:capability.{capability}");
        definitions["definitions"][format!("axioval:example.{id}")] = json!({
            "id": format!("axioval:example.{id}"), "name": text(id), "description": text(id),
            "capability": capability, "parameters": registry_signature(&capability),
            "citations": [], "tags": [],
        });
    }
    let text_file = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text_file).unwrap();
    let template = ruleset["root"]["rules"][0].clone();
    ruleset["root"]["rules"] = rules
        .iter()
        .map(|(id, _, parameters)| {
            let mut rule = template.clone();
            rule["id"] = json!(id);
            rule["definitionId"] = json!(format!("axioval:example.{id}"));
            rule["parameters"] = parameters.clone();
            rule
        })
        .collect();
    (
        case.write("definitions.json", &definitions.to_string()),
        case.write("ruleset.json", &ruleset.to_string()),
    )
}

fn check_packages(case: &Case, model: &str, packages: (PathBuf, PathBuf)) -> (Output, Value) {
    let model = case.write("model.ifc", model);
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(packages.0)
        .arg("--ruleset")
        .arg(packages.1)
        .output()
        .unwrap();
    let result = json(&output);
    (output, result)
}

fn rule_findings(result: &Value) -> Vec<(String, String, String)> {
    let mut findings: Vec<(String, String, String)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["rule_id"].as_str().unwrap().to_owned(),
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                finding["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    findings.sort();
    findings
}

#[test]
fn enumerated_bounded_and_table_values_are_checked_value_by_value() {
    let case = Case::new("property-kinds");
    let reference = |name: &str| {
        json!({"type": "propertyReference", "property": format!("axioval:example.ifc.{name}"),
               "propertySet": "axioval:example.ifc.Pset_Kinds"})
    };
    let string = |value: &str| json!({"type": "string", "value": value});
    let packages = property_packages(
        &case,
        &[
            (
                "status-new",
                "property-value",
                json!({"property": reference("Status"),
                       "values": {"type": "stringList", "value": ["NEW"]},
                       "quantifier": string("any")}),
            ),
            (
                "span-within",
                "property-value",
                json!({"property": reference("Span"),
                       "min_inclusive": string("0.5"), "max_inclusive": string("6"),
                       "quantifier": string("all"),
                       "si_units": {"type": "boolean", "value": true}}),
            ),
            (
                "load-listed",
                "property-value",
                json!({"property": reference("Load"),
                       "values": {"type": "stringList", "value": ["20"]},
                       "quantifier": string("any"), "optional": {"type": "boolean", "value": true}}),
            ),
        ],
    );
    let (output, result) = check_packages(&case, PROPERTY_KINDS, packages);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        rule_findings(&result),
        [
            (
                "span-within".to_owned(),
                "#2".to_owned(),
                "property axioval:example.ifc.Span is [range from 1 m, open above], open above, \
                 so not all its values meet the upper bound"
                    .to_owned()
            ),
            (
                "status-new".to_owned(),
                "#2".to_owned(),
                "property axioval:example.ifc.Status is [`EXISTING`, `DEMOLISH`], and none of \
                 its values meets the constraints"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn a_table_whose_columns_differ_is_checked_by_the_cells_of_the_declared_type() {
    let case = Case::new("property-table-columns");
    let reference = json!({"type": "propertyReference", "property": "axioval:example.ifc.Load",
                           "propertySet": "axioval:example.ifc.Pset_Kinds"});
    let string = |value: &str| json!({"type": "string", "value": value});
    let optional = json!({"type": "boolean", "value": true});
    let packages = property_packages(
        &case,
        &[
            (
                "load-label",
                "property-value",
                json!({"property": reference, "data_type": string("IFCLABEL"),
                       "values": {"type": "stringList", "value": ["B"]},
                       "quantifier": string("any"), "optional": optional}),
            ),
            (
                "load-length",
                "property-value",
                json!({"property": reference, "data_type": string("IFCLENGTHMEASURE"),
                       "values": {"type": "stringList", "value": ["3"]},
                       "quantifier": string("any"), "optional": optional,
                       "si_units": {"type": "boolean", "value": true}}),
            ),
            (
                "load-real",
                "property-data-type",
                json!({"property": reference, "data_type": string("IFCREAL")}),
            ),
        ],
    );
    let model = PROPERTY_KINDS.replace(
        "#8=IFCPROPERTYTABLEVALUE('Load',$,(IFCREAL(1.),IFCREAL(2.)),(IFCREAL(10.),IFCREAL(20.)),$,$,$,$);",
        "#8=IFCPROPERTYTABLEVALUE('Load',$,(IFCLABEL('A'),IFCLABEL('B')),(IFCLENGTHMEASURE(1000.),IFCLENGTHMEASURE(2000.)),$,$,$,$);",
    );
    let (output, result) = check_packages(&case, &model, packages);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // `B` is a label of #1's table, 3 m none of its lengths, and no column
    // is a real; #2 has no table.
    assert_eq!(
        rule_findings(&result),
        [
            (
                "load-length".to_owned(),
                "#1".to_owned(),
                "property axioval:example.ifc.Load is [1 m, 2 m], and none of its values \
                 meets the constraints"
                    .to_owned()
            ),
            (
                "load-real".to_owned(),
                "#1".to_owned(),
                "property axioval:example.ifc.Load has no column of type IFCREAL".to_owned()
            ),
            (
                "load-real".to_owned(),
                "#2".to_owned(),
                "missing required property axioval:example.ifc.Load".to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn property_sets_and_properties_named_by_pattern_are_enumerated() {
    let case = Case::new("property-patterns");
    let string = |value: &str| json!({"type": "string", "value": value});
    let packages = property_packages(
        &case,
        &[
            (
                "requirements",
                "property-requirements",
                json!({"requirements": {"type": "table", "value": [
                    {"property_set_pattern": string("Pset_Ch.*"),
                     "property_pattern": string("Check[A-Z]"),
                     "requirement": string("required")},
                    {"property_set_pattern": string("Pset_Draft"),
                     "requirement": string("forbidden")},
                    {"property_set": string("axioval:example.ifc.Pset_Kinds"),
                     "property_pattern": string("Sta.*"),
                     "requirement": string("required"),
                     "one_of": string("NEW|EXISTING|DEMOLISH")},
                ]}}),
            ),
            (
                "checks-ok",
                "property-value",
                json!({"property_set_pattern": string("Pset_.*"),
                       "property_pattern": string("Check.*"),
                       "values": {"type": "stringList", "value": ["ok"]},
                       "optional": {"type": "boolean", "value": true}}),
            ),
        ],
    );
    let (output, result) = check_packages(&case, PROPERTY_KINDS, packages);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let expected: Vec<(String, String, String)> = [
        (
            "checks-ok",
            "#1",
            "property Pset_Checks.CheckB is \"\", not one of the required values",
        ),
        (
            "checks-ok",
            "#2",
            "property Pset_Checks.CheckB is \"\", not one of the required values",
        ),
        (
            "requirements",
            "#1",
            "missing value: Pset_Checks.CheckB is `` (requirement row 0)",
        ),
        (
            "requirements",
            "#2",
            "forbidden property set present: /Pset_Draft/ is present with 1 property \
             (requirement row 1)",
        ),
        (
            "requirements",
            "#2",
            "missing value: Pset_Checks.CheckB is `` (requirement row 0)",
        ),
    ]
    .iter()
    .map(|(rule, object, message)| {
        (
            (*rule).to_owned(),
            (*object).to_owned(),
            (*message).to_owned(),
        )
    })
    .collect();
    assert_eq!(rule_findings(&result), expected, "{result:#}");
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// A door with leaves as instances `#first` to `#first + 20`; the door is
/// `#first + 10`. It is placed at `origin` with its local x along `along`
/// (horizontal) and its body `width` wide, 0.1 m deep and 2.1 m high over
/// local x 0..`width`, y 0..0.1. It states `operation` and one
/// `IfcDoorPanelProperties` per `(PanelOperation, PanelPosition,
/// PanelWidth)`, 40 mm deep, and a lining 50 mm thick.
fn swinging_door(
    first: u32,
    [x, y, z]: [f64; 3],
    [ax, ay]: [f64; 2],
    width: f64,
    operation: &str,
    panels: &[(&str, &str, &str)],
) -> String {
    let at = |offset: u32| first + offset;
    let door = at(10);
    let mut records = format!(
        "#{p}=IFCCARTESIANPOINT(({x:?},{y:?},{z:?}));\n\
         #{d}=IFCDIRECTION(({ax:?},{ay:?},0.));\n\
         #{a}=IFCAXIS2PLACEMENT3D(#{p},#4,#{d});\n\
         #{l}=IFCLOCALPLACEMENT($,#{a});\n\
         #{c}=IFCCARTESIANPOINT(({half:?},0.05));\n\
         #{c2}=IFCAXIS2PLACEMENT2D(#{c},$);\n\
         #{r}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{c2},{width:?},0.1);\n\
         #{s}=IFCEXTRUDEDAREASOLID(#{r},#2,#4,2.1);\n\
         #{sh}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{s}));\n\
         #{pd}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{sh}));\n\
         #{door}=IFCDOOR('{door:022}',$,$,$,$,#{l},#{pd},$,2.1,{width:?},.DOOR.,.{operation}.,$);\n\
         #{lining}=IFCDOORLININGPROPERTIES('{lining:022}',$,$,$,0.1,0.05,$,0.02,$,$,$,$,$,$,$,$,$);\n\
         #{rel}=IFCRELDEFINESBYPROPERTIES('{rel:022}',$,$,$,(#{door}),#{lining});\n",
        p = at(0),
        d = at(1),
        a = at(2),
        l = at(3),
        c = at(4),
        c2 = at(5),
        r = at(6),
        s = at(7),
        sh = at(8),
        pd = at(9),
        half = width / 2.0,
        lining = at(11),
        rel = at(12),
    );
    for (index, (panel_operation, position, ratio)) in (0_u32..).zip(panels) {
        let (set, rel) = (at(13 + 2 * index), at(14 + 2 * index));
        let _ = write!(
            records,
            "#{set}=IFCDOORPANELPROPERTIES('{set:022}',$,$,$,0.04,.{panel_operation}.,{ratio},.{position}.,$);\n\
             #{rel}=IFCRELDEFINESBYPROPERTIES('{rel:022}',$,$,$,(#{door}),#{set});\n"
        );
    }
    records
}

/// Door #30 at the origin, 0.9 m wide, hinged left and opening north over
/// the quarter disc x, y >= 0 within 0.9 m; column #59 (x 0.2..0.4,
/// y 1.2..1.4) stands 0.32 m beyond its arc and column #69 (x 3..3.2)
/// 2.1 m from it. Sliding door #90 at x 5 sweeps nothing.
fn doors_and_columns() -> String {
    let column = "IFCCOLUMN('GID',$,$,$,$,PL,REP,$,$)";
    model_with(&format!(
        "{}{}{}{}",
        swinging_door(
            20,
            [0.0, 0.0, 0.0],
            [1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
        placed_box(50, [0.3, 1.3, 0.0], [0.2, 0.2, 2.0], column),
        placed_box(60, [3.1, 0.5, 0.0], [0.2, 0.2, 2.0], column),
        swinging_door(
            80,
            [5.0, 0.0, 0.0],
            [1.0, 0.0],
            0.9,
            "SLIDING_TO_LEFT",
            &[("SLIDING", "LEFT", "$")]
        ),
    ))
}

#[test]
fn with_geometry_a_column_within_a_door_swing_reach_is_found() {
    let case = Case::new("geometry-door-swing-distance");
    let (output, result) = case.geometry_rule(
        &doors_and_columns(),
        &[("door", "IfcDoor"), ("column", "IfcColumn")],
        "axioval:capability.distance",
        &registry_signature("axioval:capability.distance"),
        entity("door"),
        json!({
            "counterparts": {"type": "selector", "value": entity("column")},
            "subject_extent": {"type": "string", "value": "door_swing"},
            "mode": {"type": "string", "value": "none_closer_than"},
            "projection": {"type": "string", "value": "horizontal"},
            "minimum_metres": {"type": "number", "value": 0.5},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    let [(door, message)] = &findings[..] else {
        panic!("one finding: {result:#}")
    };
    assert_eq!(door, "#30");
    assert!(
        message.contains("/#59 at horizontal distance between 0.316"),
        "{message}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Door #30 at the origin, 0.9 m wide, hinged at x 0 and opening north,
/// with column #59 (x 0.65..0.75, y 0.35..0.45) on its swing side by the
/// handle; sliding door #90 at x 5.
fn door_with_a_column_by_its_handle() -> String {
    let column = "IFCCOLUMN('GID',$,$,$,$,PL,REP,$,$)";
    model_with(&format!(
        "{}{}{}",
        swinging_door(
            20,
            [0.0, 0.0, 0.0],
            [1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
        placed_box(50, [0.7, 0.4, 0.0], [0.1, 0.1, 2.0], column),
        swinging_door(
            80,
            [5.0, 0.0, 0.0],
            [1.0, 0.0],
            0.9,
            "SLIDING_TO_LEFT",
            &[("SLIDING", "LEFT", "$")]
        ),
    ))
}

fn door_clearance(case: &Case, front: &str, align: &str, both: bool) -> (Output, Value) {
    case.geometry_rule(
        &door_with_a_column_by_its_handle(),
        &[("door", "IfcDoor"), ("column", "IfcColumn")],
        "axioval:capability.component-clearance",
        &registry_signature("axioval:capability.component-clearance"),
        entity("door"),
        json!({
            "side": {"type": "string", "value": "front"},
            "both_sides": {"type": "boolean", "value": both},
            "front_axis": {"type": "string", "value": front},
            "width": {"type": "quantity", "value": 0.5, "unit": "m"},
            "depth": {"type": "quantity", "value": 0.5, "unit": "m"},
            "height": {"type": "quantity", "value": 2, "unit": "m"},
            "align": {"type": "string", "value": align},
            "height_reference": {"type": "string", "value": "bottom"},
            "obstacles": {"type": "selector", "value": entity("column")},
        }),
    )
}

#[test]
fn with_geometry_door_clearances_follow_the_swing_side_and_the_handle() {
    let case = Case::new("geometry-door-clearance");
    // On the swing side, flush with the handle edge (x 0.4..0.9), the box
    // holds the column; the push side is clear.
    let (output, result) = door_clearance(&case, "swing", "handle", true);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#30".to_owned(),
            "front clearance (0.5 m wide, 0.5 m deep, 2 m high) is obstructed by \
             ifc-step:model.ifc/#59"
                .to_owned()
        )],
        "{result:#}"
    );
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(
        open.len() == 2
            && open
                .iter()
                .all(|outcome| outcome["object_id"]["local_id"] == "#90"
                    && outcome["message"]
                        .as_str()
                        .unwrap()
                        .contains("has no hinged leaf")),
        "{result:#}"
    );
    // Flush with the hinge edge (x 0..0.5) the box misses the column, and
    // so does the box on the push side.
    for (front, align) in [("swing", "hinge"), ("-swing", "handle")] {
        let (_, result) = door_clearance(&case, front, align, false);
        assert!(
            finding_messages(&result).is_empty(),
            "{front} {align}: {result:#}"
        );
    }
}

/// The area on the swing side of door #30 is `max(0.2 m, total - clear
/// width)` deep, the clear width 0.76 m from its lining and leaf: a 1 m
/// total leaves 0.24 m, short of the column, a 1.2 m total 0.44 m, which
/// the column obstructs.
#[test]
fn with_geometry_a_door_clearance_depth_follows_the_clear_width() {
    let case = Case::new("geometry-door-clearance-depth");
    let check = |total: f64| {
        case.geometry_rule(
            &door_with_a_column_by_its_handle(),
            &[("door", "IfcDoor"), ("column", "IfcColumn")],
            "axioval:capability.component-clearance",
            &registry_signature("axioval:capability.component-clearance"),
            entity("door"),
            json!({
                "side": {"type": "string", "value": "front"},
                "front_axis": {"type": "string", "value": "swing"},
                "width": {"type": "quantity", "value": 0.5, "unit": "m"},
                "depth": {"type": "quantity", "value": total, "unit": "m"},
                "depth_mode": {"type": "string", "value": "less_clear_width"},
                "depth_minimum": {"type": "quantity", "value": 0.2, "unit": "m"},
                "clear_width_from_leaves": {"type": "string", "value": "passage"},
                "height": {"type": "quantity", "value": 2, "unit": "m"},
                "align": {"type": "string", "value": "handle"},
                "height_reference": {"type": "string", "value": "bottom"},
                "obstacles": {"type": "selector", "value": entity("column")},
            }),
        )
    };
    let (_, result) = check(1.0);
    assert!(finding_messages(&result).is_empty(), "{result:#}");
    let (output, result) = check(1.2);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    let [(door, message)] = &findings[..] else {
        panic!("one finding: {result:#}")
    };
    assert_eq!(door, "#30");
    assert!(
        message.starts_with("front clearance (0.5 m wide, 0.44 m deep, 2 m high) is obstructed")
            && message.ends_with(
                "; the depth is 1.2 m less the clear width (overall width 0.9 m less 2 × 0.05 m \
                 lining and 0.04 m of open leaf, as the door states them) 0.76 m, at least 0.2 m"
            ),
        "{message}"
    );
}

/// Office #19 (x -1..4, y 0..4) north of corridor #29 (y -2..0), both
/// 3 m high, bounding doors #50 and #80 in the line between them: #50,
/// hinged at the origin, swings north into the office; #80, hinged at
/// x 2.9 and turned half round, swings south into the corridor.
fn office_and_corridor_doors() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let boundary = |id: u32, space: u32, door: u32| {
        format!(
            "#{id}=IFCRELSPACEBOUNDARY('{id:022}',$,$,$,#{space},#{door},$,.PHYSICAL.,.INTERNAL.);\n"
        )
    };
    model_with(&format!(
        "{}{}{}{}{}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Corridor'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#29),#201);\n",
        placed_box(10, [1.5, 2.0, 0.0], [5.0, 4.0, 3.0], space),
        placed_box(20, [1.5, -1.0, 0.0], [5.0, 2.0, 3.0], space),
        swinging_door(
            40,
            [0.0, 0.0, 0.0],
            [1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
        swinging_door(
            70,
            [2.9, 0.0, 0.0],
            [-1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
        boundary(300, 19, 50),
        boundary(301, 29, 50),
        boundary(302, 19, 80),
        boundary(303, 29, 80),
    ))
}

fn corridor() -> Value {
    json!({"kind": "allOf", "operands": [
        entity("space"),
        {"kind": "property", "propertySet": "axioval:example.ifc.pset-space-common",
         "property": "axioval:example.ifc.reference", "operator": "equals",
         "value": {"type": "string", "value": "Corridor"}},
    ]})
}

#[test]
fn with_geometry_a_door_swinging_into_the_corridor_is_found() {
    let case = Case::new("geometry-door-swing");
    let (output, result) = case.geometry_rule(
        &office_and_corridor_doors(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.door-swing",
        &registry_signature("axioval:capability.door-swing"),
        entity("door"),
        json!({
            "space_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:backward"]},
            "swing_not_into": {"type": "selector", "value": corridor()},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#80".to_owned(),
            "swings into ifc-step:model.ifc/#29, which `swing_not_into` forbids".to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Office #19 (x 0..4, y 0..4) opens through door #39 (x 4..4.2) onto
/// corridor #29 (x 4.2..6.2, y 0..10), whose exit #49 (1 m clear) leads
/// out at its north end. The spaces are 0.2 m apart, so the office's only
/// way out is through the corridor.
fn office_behind_a_corridor() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
    let boundary = |id: u32, space: u32, door: u32| {
        format!(
            "#{id}=IFCRELSPACEBOUNDARY('{id:022}',$,$,$,#{space},#{door},$,.PHYSICAL.,.INTERNAL.);\n"
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
         {}{}{}{}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Corridor'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#29),#201);\n\
         #210=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Exit'),$);\n\
         #211=IFCPROPERTYSET('0000000000000000000211',$,'Pset_SpaceCommon',$,(#210));\n\
         #212=IFCRELDEFINESBYPROPERTIES('0000000000000000000212',$,$,$,(#49),#211);\n\
         #220=IFCPROPERTYSINGLEVALUE('ClearWidth',$,IFCPOSITIVELENGTHMEASURE(1.),$);\n\
         #221=IFCPROPERTYSET('0000000000000000000221',$,'Access',$,(#220));\n\
         #222=IFCRELDEFINESBYPROPERTIES('0000000000000000000222',$,$,$,(#49),#221);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [5.2, 5.0, 0.0], [2.0, 10.0, 3.0], space),
        placed_box(30, [4.1, 2.0, 0.0], [0.2, 1.0, 2.1], door),
        placed_box(40, [5.2, 10.1, 0.0], [1.0, 0.2, 2.1], door),
        boundary(300, 19, 39),
        boundary(301, 29, 39),
        boundary(302, 29, 49),
    )
}

#[test]
fn with_geometry_a_corridor_every_walk_crosses_carries_the_office_behind_it() {
    // The office's walk from its door crosses the corridor, and no walk
    // round it reaches the exit: the corridor carries the office's 8
    // occupants and its own 10. 18 need 3 m, and the corridor is at most
    // 2 m wide.
    let case = Case::new("geometry-escape-route-walked-passages");
    let reference = |value: &str| {
        json!({"kind": "property", "propertySet": "axioval:example.ifc.pset-space-common",
               "property": "axioval:example.ifc.reference", "operator": "equals",
               "value": {"type": "string", "value": value}})
    };
    let (output, result) = case.geometry_rule(
        &office_behind_a_corridor(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"spaces": {"type": "selector", "value": entity("space")},
                 "area_per_occupant": {"type": "number", "value": 2.0}},
            ]},
            "widths": {"type": "table", "value": [
                {"occupants": {"type": "integer", "value": 10},
                 "width": {"type": "number", "value": 0.5},
                 "passage_width": {"type": "number", "value": 1.0}},
                {"occupants": {"type": "integer", "value": 100},
                 "width": {"type": "number", "value": 0.5},
                 "passage_width": {"type": "number", "value": 3.0}},
            ]},
            "clear_width_property": {"type": "propertyReference",
                                     "property": "axioval:example.ifc.clear-width",
                                     "propertySet": "axioval:example.ifc.pset-access"},
            "exit_path": {"type": "stringList", "value": [
                "IfcRelSpaceBoundary:forward", "IfcRelSpaceBoundary:backward",
                "IfcRelSpaceBoundary:forward"]},
            "exit_selector": {"type": "selector", "value":
                {"kind": "allOf", "operands": [entity("door"), reference("Exit")]}},
            "door_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:forward"]},
            "door_selector": {"type": "selector", "value": entity("door")},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
            "passage_selector": {"type": "selector", "value":
                {"kind": "allOf", "operands": [entity("space"), reference("Corridor")]}},
            "walked_passages": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#29".to_owned(),
            "passage ifc-step:model.ifc/#29 is at most 2 m wide (the shorter side of the \
             rectangle enclosing its footprint); 18 occupant(s) relying on it (from \
             ifc-step:model.ifc/#19, ifc-step:model.ifc/#29) require at least 3 m"
                .to_owned()
        )],
        "{result:#}"
    );
    // The proof is the walk round the corridor, which reaches no exit.
    let text = result.to_string();
    assert!(
        text.contains("axiolid:metric-route:nearest:unreachable:")
            && text.contains(":avoided=[ifc-step:model.ifc/#29]:"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_escape_travel_ends_at_the_door_out_of_the_compartment() {
    // The office and the corridor are two fire compartments (zones #500
    // and #510). Travel then ends at door #39 between them: the office's
    // farthest corner lies about 4.6 m from it, and no point of the
    // corridor lies 6 m from both #39 and its exit #49. Without the
    // compartments, the office has no exit of its own and the corridor's
    // far end lies about 10 m from #49.
    let compartments = office_behind_a_corridor().replace(
        "ENDSEC;\nEND-ISO",
        "#500=IFCZONE('0000000000000000000500',$,'A',$,$,$);\n\
         #501=IFCRELASSIGNSTOGROUP('0000000000000000000501',$,$,$,(#19),$,#500);\n\
         #510=IFCZONE('0000000000000000000510',$,'B',$,$,$);\n\
         #511=IFCRELASSIGNSTOGROUP('0000000000000000000511',$,$,$,(#29),$,#510);\n\
         ENDSEC;\nEND-ISO",
    );
    let run = |name: &str, compartments_declared: bool| {
        let case = Case::new(name);
        let mut parameters = json!({
            "uses": {"type": "table", "value": [
                {"spaces": {"type": "selector", "value": entity("space")},
                 "maximum_travel": {"type": "number", "value": 6.0}},
            ]},
            "exit_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:forward"]},
            "exit_selector": {"type": "selector", "value":
                {"kind": "allOf", "operands": [entity("door"),
                    {"kind": "property",
                     "propertySet": "axioval:example.ifc.pset-space-common",
                     "property": "axioval:example.ifc.reference", "operator": "equals",
                     "value": {"type": "string", "value": "Exit"}}]}},
            "door_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:forward"]},
            "door_selector": {"type": "selector", "value": entity("door")},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
        });
        if compartments_declared {
            parameters["compartment_selector"] =
                json!({"type": "selector", "value": entity("zone")});
            parameters["compartment_path"] =
                json!({"type": "stringList", "value": ["IfcRelAssignsToGroup:backward"]});
        }
        case.geometry_rule(
            &compartments,
            &[
                ("door", "IfcDoor"),
                ("space", "IfcSpace"),
                ("zone", "IfcZone"),
            ],
            "axioval:capability.escape-route",
            &registry_signature("axioval:capability.escape-route"),
            entity("space"),
            parameters,
        )
    };
    let (output, result) = run("geometry-escape-route-no-compartments", false);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(
        findings
            .iter()
            .map(|(object, _)| object.as_str())
            .collect::<Vec<_>>(),
        ["#19", "#29"],
        "{result:#}"
    );
    assert_eq!(
        findings[0].1,
        "has no exit via IfcRelSpaceBoundary to walk to; use 0 allows at most 6 m of travel",
        "{result:#}"
    );
    assert!(
        findings[1].1.contains("m from the nearest exit walking"),
        "{result:#}"
    );

    let (output, result) = run("geometry-escape-route-compartments", true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{result:#}",
        stderr(&output)
    );
    assert!(finding_messages(&result).is_empty(), "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_an_office_behind_one_corridor_has_one_independent_route() {
    // The office's only way out runs through the corridor: the walk round
    // it reaches no exit, so its routes are one, where two are required.
    let case = Case::new("geometry-escape-route-independent-routes");
    let reference = |value: &str| {
        json!({"kind": "property", "propertySet": "axioval:example.ifc.pset-space-common",
               "property": "axioval:example.ifc.reference", "operator": "equals",
               "value": {"type": "string", "value": value}})
    };
    let (output, result) = case.geometry_rule(
        &office_behind_a_corridor(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        json!({"kind": "allOf", "operands": [
            entity("space"), {"kind": "not", "operand": reference("Corridor")}]}),
        json!({
            "uses": {"type": "table", "value": [
                {"spaces": {"type": "selector", "value": entity("space")},
                 "exits": {"type": "integer", "value": 2}},
            ]},
            "exit_path": {"type": "stringList", "value": [
                "IfcRelSpaceBoundary:forward", "IfcRelSpaceBoundary:backward",
                "IfcRelSpaceBoundary:forward"]},
            "exit_selector": {"type": "selector", "value":
                {"kind": "allOf", "operands": [entity("door"), reference("Exit")]}},
            "exit_count": {"type": "string", "value": "routes"},
            "walking_height": {"type": "number", "value": 2.0},
            "walking_step": {"type": "number", "value": 0.02},
            "passage_selector": {"type": "selector", "value":
                {"kind": "allOf", "operands": [entity("space"), reference("Corridor")]}},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#19".to_owned(),
            "every walk from it to an exit passes through ifc-step:model.ifc/#29, so it has at \
             most 1 independent route(s); use 0 requires at least 2"
                .to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn with_geometry_exit_doors_opening_against_the_escape_are_found() {
    let case = Case::new("geometry-exit-door-direction");
    let (output, result) = case.geometry_rule(
        &office_and_corridor_doors(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.escape-route",
        &registry_signature("axioval:capability.escape-route"),
        entity("space"),
        json!({
            "uses": {"type": "table", "value": [
                {"spaces": {"type": "selector", "value": entity("space")},
                 "exits": {"type": "integer", "value": 1}},
            ]},
            "exit_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:forward"]},
            "exit_selector": {"type": "selector", "value": entity("door")},
            "exit_door_direction": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Leaving the office, #50 swings against the escape; leaving the
    // corridor, #80 does.
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#19".to_owned(),
                "exit door ifc-step:model.ifc/#50 opens into the space, against the direction \
                 of escape"
                    .to_owned()
            ),
            (
                "#29".to_owned(),
                "exit door ifc-step:model.ifc/#80 opens into the space, against the direction \
                 of escape"
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Ramp #408 (as in [`handrails_and_ramp_ends`]) with its top landing at
/// x 12..13, y -1.2..0, 0.6 m up. Door #610 north of it, hinged at
/// (13, 0.15) and turned half round, swings south over the landing; door
/// #640, hinged at (12.1, 0.15), swings north away from it.
fn doors_at_a_ramp_landing() -> String {
    let ramp = "IFCRAMPFLIGHT('GID',$,$,$,$,PL,REP,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(400, [5.0, 0.0, 0.0], &ramp_profile(6.0), ramp),
        swinging_door(
            600,
            [13.0, 0.15, 0.6],
            [-1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
        swinging_door(
            630,
            [12.1, 0.15, 0.6],
            [1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
    )
}

#[test]
fn with_geometry_a_door_swinging_over_a_ramp_landing_is_found() {
    let case = Case::new("geometry-ramp-landing-door-swing");
    let (output, result) = case.geometry_rule(
        &doors_at_a_ramp_landing(),
        &[("ramp", "IfcRampFlight"), ("door", "IfcDoor")],
        "axioval:capability.ramp-geometry",
        &registry_signature("axioval:capability.ramp-geometry"),
        entity("ramp"),
        json!({
            "landing_objects": {"type": "selector", "value": entity("ramp")},
            "landing_depth_minimum": {"type": "quantity", "value": 0.5, "unit": "m"},
            "landing_doors": {"type": "selector", "value": entity("door")},
            "landing_door_height": {"type": "quantity", "value": 2, "unit": "m"},
            "landing_door_swing": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#408", "{result:#}");
    assert!(
        findings[0].1.ends_with(
            "door ifc-step:model.ifc/#610 swings over the landing at the top of run 1 of 1"
        ),
        "{result:#}"
    );
}

/// Declares the `Name` attribute as `axioval:example.ifc.name`.
fn declare_name(definitions: &mut Value) {
    definitions["properties"]["axioval:example.ifc.name"] = json!({
        "id": "axioval:example.ifc.name",
        "name": {"default": "Name", "translations": {}},
        "valueKind": "string",
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Name"}],
        "citations": [],
    });
}

/// Declares `Qto_WallBaseQuantities` with its `GrossSideArea` and
/// `NetSideArea`.
fn declare_wall_quantities(definitions: &mut Value) {
    definitions["propertySets"]["axioval:example.ifc.qto-wall"] = json!({
        "id": "axioval:example.ifc.qto-wall",
        "name": {"default": "Qto_WallBaseQuantities", "translations": {}},
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Qto_WallBaseQuantities"}],
        "citations": [],
    });
    for (id, name) in [
        ("gross-side-area", "GrossSideArea"),
        ("net-side-area", "NetSideArea"),
    ] {
        definitions["properties"][format!("axioval:example.ifc.{id}")] = json!({
            "id": format!("axioval:example.ifc.{id}"),
            "name": {"default": name, "translations": {}},
            "valueKind": "quantity",
            "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": name}],
            "citations": [],
        });
    }
}

/// One product with a single extruded body: `profile` placed at `at` with
/// the placement axes `axes` (`$,$` for the world's), extruded `depth`
/// along the placement's Z. Entities `#id` to `#id + 6`.
fn extruded(id: u32, entity: &str, profile: &str, at: [f64; 3], axes: &str, depth: f64) -> String {
    format!(
        "#{a}={profile};\n#{b}=IFCCARTESIANPOINT(({},{},{}));\n\
         #{c}=IFCAXIS2PLACEMENT3D(#{b},{axes});\n\
         #{d}=IFCEXTRUDEDAREASOLID(#{a},#{c},#4,{depth});\n\
         #{e}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{d}));\n\
         #{f}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{e}));\n\
         #{id}={};\n",
        at[0],
        at[1],
        at[2],
        entity
            .replace("GID", &format!("{id:022}"))
            .replace("REP", &format!("#{}", id + 6)),
        a = id + 1,
        b = id + 2,
        c = id + 3,
        d = id + 4,
        e = id + 5,
        f = id + 6,
    )
}

/// Wall `wall`'s stated gross and net side areas, entities `#id` to
/// `#id + 3`.
fn wall_areas(id: u32, wall: u32, gross: f64, net: f64) -> String {
    format!(
        "#{a}=IFCQUANTITYAREA('GrossSideArea',$,$,{gross:?},$);\n\
         #{b}=IFCQUANTITYAREA('NetSideArea',$,$,{net:?},$);\n\
         #{c}=IFCELEMENTQUANTITY('{c:022}',$,'Qto_WallBaseQuantities',$,$,(#{a},#{b}));\n\
         #{id}=IFCRELDEFINESBYPROPERTIES('{id:022}',$,$,$,(#{wall}),#{c});\n",
        a = id + 1,
        b = id + 2,
        c = id + 3,
    )
}

/// [`hosts_with_openings`] with the beam #50 resting on a 300 mm square
/// column #700 at x 0.5 (connected by `IfcRelConnectsElements`) and an HEB
/// column #800 at x 5.5 (by `IfcRelConnectsPathElements`), and a further
/// round hole #900 at x 5 through its web. Walls #1000 and #1100, 5 m long
/// at y 5 and y 10, state their gross and net side areas: #1000 holds two
/// 1 m x 1.2 m windows (#1030, #1060) and states net = gross - 2.4 m²;
/// #1100 holds one (#1130) and states net = gross. Wall #10 states areas
/// too, but its window #300 reaches past its end.
fn supported_beam_and_walls_with_areas() -> String {
    let column = |id: u32, x: f64, profile: &str| {
        extruded(
            id,
            "IFCCOLUMN('GID',$,$,$,$,#3,REP,$,.COLUMN.)",
            profile,
            [x, 0.0, 0.0],
            "$,$",
            2.85,
        )
    };
    let wall = |id: u32, y: f64| {
        format!(
            "#{p}=IFCCARTESIANPOINT((2.5,0.));\n#{q}=IFCAXIS2PLACEMENT2D(#{p},$);\n{}",
            extruded(
                id,
                "IFCWALL('GID',$,$,$,$,#3,REP,$,.STANDARD.)",
                &format!("IFCRECTANGLEPROFILEDEF(.AREA.,$,#{},5.,0.2)", id + 9),
                [0.0, y, 0.0],
                "$,$",
                3.0,
            ),
            p = id + 8,
            q = id + 9,
        )
    };
    let voids = |id: u32, host: u32, opening: u32| {
        format!("#{id}=IFCRELVOIDSELEMENT('{id:022}',$,$,$,#{host},#{opening});\n")
    };
    let window = |id: u32, host: u32, x: f64, y: f64| {
        format!(
            "{}{}",
            extruded(
                id,
                "IFCOPENINGELEMENT('GID',$,$,$,$,#3,REP,$,.OPENING.)",
                "IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1.,1.2)",
                [x, y + 0.1, 1.5],
                "#6,#7",
                0.2,
            ),
            voids(id + 7, host, id),
        )
    };
    let extra = [
        "#24=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);\n".to_owned(),
        column(700, 0.5, "IFCRECTANGLEPROFILEDEF(.AREA.,$,$,0.3,0.3)"),
        "#710=IFCRELCONNECTSELEMENTS('0000000000000000000710',$,$,$,$,#50,#700);\n".to_owned(),
        column(
            800,
            5.5,
            "IFCISHAPEPROFILEDEF(.AREA.,'HEB300',$,0.3,0.3,0.011,0.019,$,$,$)",
        ),
        "#810=IFCRELCONNECTSPATHELEMENTS('0000000000000000000810',$,$,$,$,#800,#50,(),(),\
         .ATSTART.,.ATEND.);\n"
            .to_owned(),
        extruded(
            900,
            "IFCOPENINGELEMENT('GID',$,$,$,$,#3,REP,$,.OPENING.)",
            "IFCCIRCLEPROFILEDEF(.AREA.,$,$,0.05)",
            [5.0, -0.2, 3.0],
            "#8,#7",
            0.4,
        ),
        voids(907, 50, 900),
        wall(1000, 5.0),
        window(1030, 1000, 1.0, 5.0),
        window(1060, 1000, 3.0, 5.0),
        wall_areas(1090, 1000, 15.0, 12.6),
        wall(1100, 10.0),
        window(1130, 1100, 2.0, 10.0),
        wall_areas(1190, 1100, 15.0, 15.0),
        wall_areas(1200, 10, 15.0, 11.4),
    ]
    .concat();
    hosts_with_openings()
        .replace(
            "#22=IFCUNITASSIGNMENT((#20,#21));",
            "#22=IFCUNITASSIGNMENT((#20,#21,#24));",
        )
        .replace("ENDSEC;\nEND-ISO", &format!("{extra}ENDSEC;\nEND-ISO"))
}

#[test]
fn beam_holes_are_checked_against_the_beams_supports() {
    let case = Case::new("opening-zone-supports");
    let (output, result) = case.geometry_rule(
        &supported_beam_and_walls_with_areas(),
        &[
            ("opening", "IfcOpeningElement"),
            ("beam", "IfcBeam"),
            ("column", "IfcColumn"),
        ],
        "axioval:capability.opening-zone",
        &registry_signature("axioval:capability.opening-zone"),
        entity("opening"),
        json!({
            "host_path": {"type": "stringList", "value": ["IfcRelVoidsElement:backward"]},
            "host_selector": {"type": "selector", "value": entity("beam")},
            "length_axis": {"type": "string", "value": "extrusion"},
            "height_axis": {"type": "string", "value": "profile-y"},
            "support_path": {"type": "stringList", "value": ["IfcRelConnectsElements:either"]},
            "support_selector": {"type": "selector", "value": entity("column")},
            "support_distance": {"type": "quantity", "value": 500.0, "unit": "mm"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#500".to_owned(),
                "opening is 0.1 m from support #700 along its host #50; 0.5 m required".to_owned()
            ),
            (
                "#900".to_owned(),
                "opening is 0.3 m from support #800 along its host #50; 0.5 m required".to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// The distance from supports as the depth of the 300 mm beam, at least
/// 0.25 m: #900, 0.3 m from its support, now meets it.
#[test]
fn beam_holes_keep_a_distance_from_supports_scaled_by_the_beam_depth() {
    let case = Case::new("opening-zone-supports-ratio");
    let (output, result) = case.geometry_rule(
        &supported_beam_and_walls_with_areas(),
        &[
            ("opening", "IfcOpeningElement"),
            ("beam", "IfcBeam"),
            ("column", "IfcColumn"),
        ],
        "axioval:capability.opening-zone",
        &registry_signature("axioval:capability.opening-zone"),
        entity("opening"),
        json!({
            "host_path": {"type": "stringList", "value": ["IfcRelVoidsElement:backward"]},
            "host_selector": {"type": "selector", "value": entity("beam")},
            "length_axis": {"type": "string", "value": "extrusion"},
            "height_axis": {"type": "string", "value": "profile-y"},
            "support_path": {"type": "stringList", "value": ["IfcRelConnectsElements:either"]},
            "support_selector": {"type": "selector", "value": entity("column")},
            "support_distance": {"type": "quantity", "value": 250.0, "unit": "mm"},
            "support_distance_ratio": {"type": "number", "value": 1.0},
            "support_distance_reference": {"type": "string", "value": "depth"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#500".to_owned(),
            "opening is 0.1 m from support #700 along its host #50; 0.3 m (the larger of \
             0.25 m and 1 × the host's depth 0.3 m) required"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn wall_openings_are_summed_against_gross_less_net_side_area() {
    let case = Case::new("opening-area-walls");
    let (output, result) = case.geometry_rule(
        &supported_beam_and_walls_with_areas(),
        &[("opening", "IfcOpeningElement"), ("wall", "IfcWall")],
        "axioval:capability.opening-area",
        &registry_signature("axioval:capability.opening-area"),
        entity("wall"),
        json!({
            "opening_path": {"type": "stringList", "value": ["IfcRelVoidsElement:forward"]},
            "opening_selector": {"type": "selector", "value": entity("opening")},
            "length_axis": {"type": "string", "value": "profile-x"},
            "height_axis": {"type": "string", "value": "extrusion"},
            "gross_area": {"type": "propertyReference",
                           "property": "axioval:example.ifc.gross-side-area",
                           "propertySet": "axioval:example.ifc.qto-wall"},
            "net_area": {"type": "propertyReference",
                         "property": "axioval:example.ifc.net-side-area",
                         "propertySet": "axioval:example.ifc.qto-wall"},
            "area_tolerance": {"type": "quantity", "value": 0.01, "unit": "m2"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#1100".to_owned(),
            "its openings (#1130) cover 1.2 m² of its face, but its gross side area 15 m² \
             less its net side area 15 m² is 0 m²; they must agree within 0.01 m²"
                .to_owned()
        )],
        "{result:#}"
    );
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 1, "{result:#}");
    assert_eq!(
        not_evaluated[0]["object_id"]["local_id"],
        json!("#10"),
        "{result:#}"
    );
}

#[test]
fn a_wall_with_a_window_is_not_empty() {
    let run = |name: &str, minimum: f64| {
        Case::new(name).geometry_rule(
            &supported_beam_and_walls_with_areas(),
            &[("opening", "IfcOpeningElement"), ("wall", "IfcWall")],
            "axioval:capability.empty-host",
            &registry_signature("axioval:capability.empty-host"),
            entity("wall"),
            json!({
                "opening_path": {"type": "stringList", "value": ["IfcRelVoidsElement:forward"]},
                "opening_selector": {"type": "selector", "value": entity("opening")},
                "length_axis": {"type": "string", "value": "profile-x"},
                "height_axis": {"type": "string", "value": "extrusion"},
                "minimum_opening_area": {"type": "quantity", "value": minimum, "unit": "m2"},
            }),
        )
    };
    let (output, result) = run("empty-host-walls", 0.0);
    assert_ne!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(finding_messages(&result), [], "{result:#}");
    let walls = |result: &Value| -> Vec<Value> {
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .map(|outcome| outcome["object_id"]["local_id"].clone())
            .collect()
    };
    assert!(!walls(&result).contains(&json!("#1100")), "{result:#}");
}

#[test]
fn beam_supports_are_found_by_contact_with_geometry() {
    // The I-beam rests on both columns at distance 0: the square #700 and
    // the HEB #800 are supports by contact alone. Hole #600 is tangent to
    // the top flange's outer face.
    let model = supported_beam_and_walls_with_areas();
    let case = Case::new("opening-zone-contact");
    let (output, result) = case.geometry_rule(
        &model,
        &[
            ("opening", "IfcOpeningElement"),
            ("beam", "IfcBeam"),
            ("column", "IfcColumn"),
        ],
        "axioval:capability.opening-zone",
        &registry_signature("axioval:capability.opening-zone"),
        entity("opening"),
        json!({
            "host_path": {"type": "stringList", "value": ["IfcRelVoidsElement:backward"]},
            "host_selector": {"type": "selector", "value": entity("beam")},
            "length_axis": {"type": "string", "value": "extrusion"},
            "height_axis": {"type": "string", "value": "profile-y"},
            "support_selector": {"type": "selector", "value": entity("column")},
            "support_gap": {"type": "quantity", "value": 1.0, "unit": "mm"},
            "support_distance": {"type": "quantity", "value": 500.0, "unit": "mm"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Every body meshes, the tangent hole's host included. The HEB has no
    // fillets, so its mesh is exact; only the beam, cut by round holes, is
    // a tessellation.
    let geometry = &result["geometry"];
    assert_eq!(geometry["unmeasured"], json!([]), "{geometry:#}");
    assert_eq!(geometry["tessellated"], 1, "{geometry:#}");
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#500".to_owned(),
                "opening is 0.1 m from support #700 along its host #50; 0.5 m required".to_owned()
            ),
            (
                "#900".to_owned(),
                "opening is 0.3 m from support #800 along its host #50; 0.5 m required".to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Stair flight #108 (four 0.17 m risers, x 0 to 1.12, y -1.2 to 0) with
/// landing slab #209 beyond its top tread (x 1.12 to 1.92, top at 0.68 m).
/// Door #310 north of the landing, hinged at (1.92, 0.15) and turned half
/// round, swings south over it; door #340, hinged at (1.22, 0.15), swings
/// north away from it.
fn doors_at_a_stair_landing() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        placed_box(
            200,
            [1.52, -0.6, 0.48],
            [0.8, 1.2, 0.2],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.LANDING.)"
        ),
        swinging_door(
            300,
            [1.92, 0.15, 0.68],
            [-1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
        swinging_door(
            330,
            [1.22, 0.15, 0.68],
            [1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
    )
}

#[test]
fn with_geometry_a_door_swinging_over_a_stair_landing_is_found() {
    let case = Case::new("geometry-stair-landing-door-swing");
    let (output, result) = case.geometry_rule(
        &doors_at_a_stair_landing(),
        &[
            ("flight", "IfcStairFlight"),
            ("slab", "IfcSlab"),
            ("door", "IfcDoor"),
        ],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "landing_objects": {"type": "selector", "value": entity("slab")},
            "landing_depth_minimum": {"type": "quantity", "value": 0.5, "unit": "m"},
            "landing_doors": {"type": "selector", "value": entity("door")},
            "landing_door_height": {"type": "quantity", "value": 2, "unit": "m"},
            "landing_door_swing": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Neither door stands on the landing; #310 swings over it.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#108", "{result:#}");
    assert!(
        findings[0].1.ends_with(
            "door ifc-step:model.ifc/#310 swings over the landing at the top of the flight"
        ),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Stair flight #108 (four 0.17 m risers from x 0 along x, 1.2 m wide in
/// y -1.2 to 0) and cupboard #209, 1 m to 1.2 m before its first riser.
fn a_cupboard_before_a_flight() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        placed_box(
            200,
            [-1.1, -0.6, 0.0],
            [0.2, 0.6, 1.0],
            "IFCFURNISHINGELEMENT('GID',$,$,$,$,PL,REP,$)"
        ),
    )
}

#[test]
fn with_geometry_a_cupboard_in_a_flights_end_space_is_found() {
    let case = Case::new("geometry-stair-end-space");
    let (output, result) = case.geometry_rule(
        &a_cupboard_before_a_flight(),
        &[
            ("flight", "IfcStairFlight"),
            ("furniture", "IfcFurnishingElement"),
        ],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "end_space_depth": {"type": "quantity", "value": 1.5, "unit": "m"},
            "end_space_width": {"type": "quantity", "value": 1.2, "unit": "m"},
            "end_space_height": {"type": "quantity", "value": 2, "unit": "m"},
            "end_space_obstacles": {"type": "selector", "value": entity("furniture")},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The cupboard stands 1 m before the first riser; nothing beyond the
    // top.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#108", "{result:#}");
    assert_eq!(
        findings[0].1,
        "ifc-step:model.ifc/#209 obstructs the free space at the bottom of the flight (1.5 m \
         deep, 1.2 m wide)",
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Stair #700 of two flights in one line, aggregated by #701: #108 (four
/// 0.17 m risers, x 0 to 1.12, y -1.2 to 0) and #408 (the same from x 2.12
/// and 0.68 m up), landing #609 between them. Each flight has a handrail
/// along its left side (y 0.05 to 0.1, seen climbing), #208 and #508,
/// reaching 0.3 m past its ends; nothing joins them across the landing.
fn a_stair_whose_rail_stops_at_its_landing() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}{}{}\
         #700=IFCSTAIR('0000000000000000000700',$,$,$,$,#3,$,$,.STRAIGHT_RUN_STAIR.);\n\
         #701=IFCRELAGGREGATES('0000000000000000000701',$,$,$,#700,(#108,#408,#609));\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        swept(200, [0.0, 0.05, 0.0], &rail_profile(0.9), 0.05, rail),
        profiled(400, [2.12, 0.0, 0.68], &stair_profile(&[0.17; 4]), flight),
        swept(500, [2.12, 0.05, 0.68], &rail_profile(0.9), 0.05, rail),
        placed_box(
            600,
            [1.62, -0.6, 0.48],
            [1.0, 1.2, 0.2],
            "IFCSLAB('GID',$,$,$,$,PL,REP,$,.LANDING.)"
        ),
    )
}

#[test]
fn with_geometry_a_stairs_rail_stopping_at_its_landing_is_found() {
    let case = Case::new("geometry-whole-stair");
    let (output, result) = case.geometry_rule(
        &a_stair_whose_rail_stops_at_its_landing(),
        &[
            ("stair", "IfcStair"),
            ("flight", "IfcStairFlight"),
            ("rail", "IfcRailing"),
        ],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("stair"),
        json!({
            "stair_path": {"type": "stringList", "value": ["IfcRelAggregates"]},
            "stair_flights": {"type": "selector", "value": entity("flight")},
            "maximum_total_rise": {"type": "quantity", "value": 1.2, "unit": "m"},
            "handrail_objects": {"type": "selector", "value": entity("rail")},
            "handrail_reach_across": {"type": "quantity", "value": 0.2, "unit": "m"},
            "handrail_reach_above": {"type": "quantity", "value": 1.5, "unit": "m"},
            "handrail_continuous_across_landings": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The stair rises 1.36 m; its left handrail breaks off at the landing.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert!(
        findings.iter().all(|(object, _)| object == "#700"),
        "{result:#}"
    );
    assert!(
        findings.iter().any(|(_, message)| message
            == "the stair rises 1.36 m from its lowest flight's base to its highest flight's top; \
                at most 1.2 m allowed"),
        "{result:#}"
    );
    assert!(
        findings.iter().any(|(_, message)| message
            .starts_with("the handrail along the left side stops at the landing between ")
            && message.contains("#208 and ")
            && message.ends_with("#508 are not joined by selected rails within 0 m of each other")),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Stair flight #108 (four 0.17 m risers from x 0 along x, 1.2 m wide in
/// y -1.2 to 0) and tactile flooring #209 before its first riser, x -0.6 to
/// -0.3 across its width: only half the depth a 0.6 m strip needs.
fn a_narrow_tactile_strip_before_a_flight() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        placed_box(
            200,
            [-0.45, -0.6, 0.0],
            [0.3, 1.2, 0.01],
            "IFCCOVERING('GID',$,$,$,$,PL,REP,$,.FLOORING.)"
        ),
    )
}

#[test]
fn with_geometry_a_narrow_or_missing_tactile_strip_is_found() {
    let case = Case::new("geometry-stair-tactile");
    let (output, result) = case.geometry_rule(
        &a_narrow_tactile_strip_before_a_flight(),
        &[("flight", "IfcStairFlight"), ("covering", "IfcCovering")],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "tactile_objects": {"type": "selector", "value": entity("covering")},
            "tactile_offset": {"type": "quantity", "value": 0.3, "unit": "m"},
            "tactile_depth": {"type": "quantity", "value": 0.6, "unit": "m"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #209 covers only half the strip before the first riser; nothing lies
    // beyond the last.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert!(
        findings.iter().all(|(object, _)| object == "#108"),
        "{result:#}"
    );
    assert!(
        findings.iter().any(|(_, message)| message.starts_with(
            "the tactile strip at the bottom of the flight (0.6 m deep, 0.3 m before the first \
             riser, across the flight) is not covered: "
        ) && message.ends_with("#209 leave part of it bare")),
        "{result:#}"
    );
    assert!(
        findings.iter().any(|(_, message)| message
            == "no selected tactile surface lies in the tactile strip at the top of the flight \
                (0.6 m deep, 0.3 m beyond the last riser, across the flight)"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Stair flight #108 (four 0.17 m risers, x 0 to 1.12, 1.2 m wide in y
/// -1.2 to 0) with 0.1 m wide handrails #208 (y -0.1 to 0) and #308 (y -1.2
/// to -1.1) inside both its sides, 0.9 m above its nosing line.
fn a_flight_narrowed_by_its_rails() -> String {
    let flight = "IFCSTAIRFLIGHT('GID',$,$,$,$,PL,REP,$,$,$,$,$,$)";
    let rail = "IFCRAILING('GID',$,$,$,$,PL,REP,$,.HANDRAIL.)";
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
         #9=IFCDIRECTION((0.,-1.,0.));\n\
         #10=IFCDIRECTION((1.,0.,0.));\n\
         {}{}{}ENDSEC;\nEND-ISO-10303-21;\n",
        profiled(100, [0.0, 0.0, 0.0], &stair_profile(&[0.17; 4]), flight),
        swept(200, [0.0, 0.0, 0.0], &rail_profile(0.9), 0.1, rail),
        swept(300, [0.0, -1.1, 0.0], &rail_profile(0.9), 0.1, rail),
    )
}

#[test]
fn with_geometry_a_flight_narrowed_by_its_rails_is_found() {
    let case = Case::new("geometry-stair-clear-width");
    let (output, result) = case.geometry_rule(
        &a_flight_narrowed_by_its_rails(),
        &[("flight", "IfcStairFlight"), ("rail", "IfcRailing")],
        "axioval:capability.stair-geometry",
        &registry_signature("axioval:capability.stair-geometry"),
        entity("flight"),
        json!({
            "clear_width_minimum": {"type": "quantity", "value": 1.1, "unit": "m"},
            "clear_width_obstacles": {"type": "selector", "value": entity("rail")},
            "clear_width_band_from": {"type": "quantity", "value": 0.5, "unit": "m"},
            "clear_width_band_to": {"type": "quantity", "value": 1.5, "unit": "m"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // 1.2 m less 0.1 m on each side leaves 1 m.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#108", "{result:#}");
    assert!(
        findings[0].1.starts_with(
            "the clear width of the flight 0.5 m to 1.5 m above its pitch line is 1 m beside "
        ) && findings[0].1.ends_with("#308; at least 1.1 m required"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Metres. Wall #10, 0.2 m thick and 3 m high, is extruded up from a plan
/// polyline mitred at its far end: 5 m long on its face y = 0, 5.2 m on
/// y = 0.2. Windows of 1 m x 1.2 m run through it along -y at x 2 to 3
/// (#100), 3.5 to 4.5 (#200, 0.5 m from the short face's end) and 4.1 to
/// 5.1 (#300, through the mitre). #400 is an L-shaped opening, a 1 m x
/// 0.5 m foot with a 0.5 m x 1 m leg, spanning x 0.7 to 1.7 and z 0.5 to 2.
fn mitred_wall_with_openings() -> String {
    let opening = |id: u32, profile: &str, x: f64, z: f64| {
        format!(
            "#{a}={profile};\n#{b}=IFCCARTESIANPOINT(({x},0.3,{z}));\n\
             #{c}=IFCAXIS2PLACEMENT3D(#{b},#6,#7);\n\
             #{d}=IFCEXTRUDEDAREASOLID(#{a},#{c},#4,0.4);\n\
             #{e}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{d}));\n\
             #{f}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{e}));\n\
             #{id}=IFCOPENINGELEMENT('{id:022}',$,$,$,$,#3,#{f},$,.OPENING.);\n\
             #{g}=IFCRELVOIDSELEMENT('{g:022}',$,$,$,#10,#{id});\n",
            a = id + 1,
            b = id + 2,
            c = id + 3,
            d = id + 4,
            e = id + 5,
            f = id + 6,
            g = id + 7,
        )
    };
    let window = |id: u32, x: f64| opening(id, "IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1.,1.2)", x, 1.5);
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCDIRECTION((0.,-1.,0.));\n\
         #7=IFCDIRECTION((1.,0.,0.));\n\
         #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #21=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
         #22=IFCUNITASSIGNMENT((#20,#21));\n\
         #23=IFCPROJECT('0000000000000000000023',$,'P',$,$,$,$,(#5),#22);\n\
         #30=IFCCARTESIANPOINT((0.,0.));\n\
         #31=IFCCARTESIANPOINT((5.,0.));\n\
         #32=IFCCARTESIANPOINT((5.2,0.2));\n\
         #33=IFCCARTESIANPOINT((0.,0.2));\n\
         #34=IFCPOLYLINE((#30,#31,#32,#33,#30));\n\
         #13=IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#34);\n\
         #14=IFCEXTRUDEDAREASOLID(#13,#2,#4,3.);\n\
         #15=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#14));\n\
         #16=IFCPRODUCTDEFINITIONSHAPE($,$,(#15));\n\
         #10=IFCWALL('0000000000000000000010',$,$,$,$,#3,#16,$,.STANDARD.);\n\
         #40=IFCCARTESIANPOINTLIST2D(((0.,0.),(1.,0.),(1.,0.5),(0.5,0.5),(0.5,1.5),(0.,1.5)));\n\
         #41=IFCINDEXEDPOLYCURVE(#40,$,$);\n\
         {}{}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        window(100, 2.5),
        window(200, 4.0),
        window(300, 4.6),
        opening(400, "IFCARBITRARYCLOSEDPROFILEDEF(.AREA.,$,#41)", 0.7, 0.5),
    )
}

#[test]
fn openings_near_a_mitred_wall_end_are_checked_against_its_plan_outline() {
    let case = Case::new("opening-zone-mitre");
    let metres = |value: f64| json!({"type": "quantity", "value": value, "unit": "m"});
    let (output, result) = case.geometry_rule(
        &mitred_wall_with_openings(),
        &[("opening", "IfcOpeningElement"), ("wall", "IfcWall")],
        "axioval:capability.opening-zone",
        &registry_signature("axioval:capability.opening-zone"),
        entity("opening"),
        json!({
            "host_path": {"type": "stringList", "value": ["IfcRelVoidsElement:backward"]},
            "host_selector": {"type": "selector", "value": entity("wall")},
            "length_axis": {"type": "string", "value": "profile-x"},
            "height_axis": {"type": "string", "value": "extrusion"},
            "end_distance": metres(0.6),
            "edge_distance": metres(0.6),
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [
            (
                "#200".to_owned(),
                "opening is 0.5 m from an end of its host #10; 0.6 m required".to_owned()
            ),
            (
                "#300".to_owned(),
                "opening lies partly outside its host #10: it crosses the edge of the host's \
                 outline"
                    .to_owned()
            ),
            (
                "#400".to_owned(),
                "opening is 0.5 m from an edge of its host #10; 0.6 m clear required".to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Room #19 (x 0..2.4, y 0..1.6, 3 m high) with door #50 in its north
/// wall: hinged at (1.2, 1.7), its 0.9 m leaf closed westward and opening
/// south over the quarter disc south-west of its hinge.
fn room_with_a_door_swinging_in() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    model_with(&format!(
        "{}{}",
        placed_box(10, [1.2, 0.8, 0.0], [2.4, 1.6, 3.0], space),
        swinging_door(
            40,
            [1.2, 1.7, 0.0],
            [-1.0, 0.0],
            0.9,
            "SINGLE_SWING_LEFT",
            &[("SWINGING", "LEFT", "$")]
        ),
    ))
}

#[test]
fn with_geometry_a_turning_circle_needs_the_floor_a_door_swings_over() {
    let case = Case::new("geometry-free-floor-door-swing");
    let check = |extra: Value| {
        let mut parameters = json!({
            "diameter_metres": {"type": "number", "value": 1.5},
            "height_metres": {"type": "number", "value": 2.0},
        });
        for (name, value) in extra.as_object().unwrap() {
            parameters[name] = value.clone();
        }
        case.geometry_rule(
            &room_with_a_door_swinging_in(),
            &[("door", "IfcDoor"), ("space", "IfcSpace")],
            "axioval:capability.free-floor-circle",
            &registry_signature("axioval:capability.free-floor-circle"),
            entity("space"),
            parameters,
        )
    };
    let (output, result) = check(json!({}));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(finding_messages(&result), [], "{result:#}");
    let (output, result) = check(json!({
        "subtract_door_swings": {"type": "selector", "value": entity("door")},
    }));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#19".to_owned(),
            "NO_FREE_FLOOR_SPACE_FOR_CIRCLE".to_owned()
        )],
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Text of every viewpoint file in a written archive.
fn bcf_viewpoints(path: &Path) -> Vec<String> {
    use std::io::Read as _;
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut texts = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if Path::new(entry.name())
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bcfv"))
        {
            let mut text = String::new();
            entry.read_to_string(&mut text).unwrap();
            texts.push(text);
        }
    }
    texts
}

#[test]
fn with_geometry_bcf_viewpoints_frame_the_clashing_pair() {
    for (version, expected) in [
        ("2.1", openbim_bcf::BcfVersion::V2_1),
        ("3.0", openbim_bcf::BcfVersion::V3_0),
    ] {
        let case = Case::new(&format!("geometry-bcf-{version}"));
        let bcf = case.path("issues.bcfzip");
        let output = case.clash_check(&[
            "--geometry",
            "--summary",
            "--bcf",
            bcf.to_str().unwrap(),
            "--bcf-date",
            "2026-09-26T10:00:00Z",
            "--bcf-version",
            version,
        ]);
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

        let archive = openbim_bcf::read_path(&bcf).unwrap();
        assert_eq!(archive.version().resolved(), Some(expected));
        assert!(
            archive.diagnostics().is_empty(),
            "{:?}",
            archive.diagnostics()
        );
        let topic = &archive.topics().next().unwrap().topic;
        assert_eq!(topic.priority.as_deref(), Some("High"));

        // A perspective and an orthogonal view of both walls, looking down
        // at the centre of the pair: x 0..4, y -2..2, z 0..3.
        let views = bcf_viewpoints(&bcf);
        assert_eq!(views.len(), 2, "{views:?}");
        for view in &views {
            assert!(
                view.contains("0000000000000000000016") && view.contains("0000000000000000000026"),
                "{view}"
            );
            // The subject in red, the wall it clashes with in blue.
            let red = view.find("Color=\"FFFF0000\"").expect(view);
            let blue = view.find("Color=\"FF0000FF\"").expect(view);
            let coloring = view.find("</Coloring>").expect(view);
            assert!(red < blue, "{view}");
            let (subject, related) = (&view[red..blue], &view[blue..coloring]);
            let walls = ["0000000000000000000016", "0000000000000000000026"];
            let in_subject: Vec<_> = walls.iter().filter(|w| subject.contains(*w)).collect();
            let in_related: Vec<_> = walls.iter().filter(|w| related.contains(*w)).collect();
            assert!(
                in_subject.len() == 1 && in_related.len() == 1 && in_subject != in_related,
                "{view}"
            );
        }
        let perspective = views
            .iter()
            .find(|view| view.contains("<PerspectiveCamera>"))
            .unwrap();
        assert!(
            views.iter().any(|view| view.contains("<OrthogonalCamera>")),
            "{views:?}"
        );
        let number = |element: &str| -> f64 {
            let start = perspective.find(&format!("<{element}>")).unwrap() + element.len() + 2;
            let end = start + perspective[start..].find('<').unwrap();
            perspective[start..end].parse().unwrap()
        };
        let viewpoint = perspective.find("<CameraViewPoint>").unwrap();
        let (x, z) = {
            let tail = &perspective[viewpoint..];
            let read = |axis: &str| -> f64 {
                let start = tail.find(&format!("<{axis}>")).unwrap() + 3;
                let end = start + tail[start..].find('<').unwrap();
                tail[start..end].parse().unwrap()
            };
            (read("X"), read("Z"))
        };
        assert!(x > 2.0 && z > 1.5, "{perspective}");
        assert!((number("FieldOfView") - 60.0).abs() < 1e-9, "{perspective}");
    }
}

#[test]
fn bcf_colours_are_configurable_and_can_be_left_out() {
    let case = Case::new("bcf-colours");
    let views = |name: &str, extra: &[&str]| {
        let bcf = case.path(name);
        let mut args = vec!["--bcf", bcf.to_str().unwrap()];
        args.extend(extra);
        let output = case.check(&ifc("0000000000000000000002", false), true, &args);
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        let views = bcf_viewpoints(&bcf);
        assert!(!views.is_empty());
        views
    };
    // Without geometry nothing is coloured unless a colour is given.
    let plain = views("plain.bcfzip", &[]);
    assert!(
        plain.iter().all(|view| !view.contains("Coloring")),
        "{plain:?}"
    );
    let colored = views("colored.bcfzip", &["--bcf-subject-color", "00ff00"]);
    assert!(
        colored
            .iter()
            .all(|view| view.contains("Color=\"FF00FF00\"")),
        "{colored:?}"
    );

    let bcf = case.path("uncolored.bcfzip");
    let output = case.clash_check(&[
        "--geometry",
        "--bcf",
        bcf.to_str().unwrap(),
        "--bcf-no-color",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let uncolored = bcf_viewpoints(&bcf);
    assert!(
        !uncolored.is_empty() && uncolored.iter().all(|view| !view.contains("Coloring")),
        "{uncolored:?}"
    );

    let bcf = case.path("bad.bcfzip");
    let output = case.check(
        &ifc("0000000000000000000002", false),
        true,
        &["--bcf", bcf.to_str().unwrap(), "--bcf-related-color", "red"],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

#[test]
fn bcf_isolate_hides_everything_but_the_clashing_pair() {
    let case = Case::new("bcf-isolate");
    let bcf = case.path("issues.bcfzip");
    let output = case.clash_check(&[
        "--geometry",
        "--bcf",
        bcf.to_str().unwrap(),
        "--bcf-isolate",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let views = bcf_viewpoints(&bcf);
    assert_eq!(views.len(), 2, "{views:?}");
    for view in &views {
        let exceptions = &view[view.find("<Exceptions>").expect(view)..];
        assert!(
            view.contains("DefaultVisibility=\"false\"")
                && exceptions.contains("0000000000000000000016")
                && exceptions.contains("0000000000000000000026"),
            "{view}"
        );
    }
}

#[test]
fn bcf_section_box_cuts_framed_viewpoints_only() {
    let case = Case::new("bcf-section-box");
    let bcf = case.path("boxed.bcfzip");
    let output = case.clash_check(&[
        "--geometry",
        "--bcf",
        bcf.to_str().unwrap(),
        "--bcf-version",
        "3.0",
        "--bcf-section-box",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let views = bcf_viewpoints(&bcf);
    assert_eq!(views.len(), 2, "{views:?}");
    for view in &views {
        assert_eq!(view.matches("<ClippingPlane>").count(), 6, "{view}");
    }

    let bcf = case.path("unmeasured.bcfzip");
    let output = case.check(
        &ifc("0000000000000000000002", false),
        true,
        &["--bcf", bcf.to_str().unwrap(), "--bcf-section-box"],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let views = bcf_viewpoints(&bcf);
    assert!(
        !views.is_empty() && views.iter().all(|view| !view.contains("ClippingPlane")),
        "{views:?}"
    );
}

#[test]
fn bcf_3_without_geometry_writes_nothing_and_fails_with_status_1() {
    let case = Case::new("bcf-3-without-bounds");
    let bcf = case.path("issues.bcfzip");
    let output = case.check(
        &ifc("0000000000000000000002", false),
        true,
        &["--bcf", bcf.to_str().unwrap(), "--bcf-version", "3.0"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("BCF 3.0 needs a camera"),
        "{}",
        stderr(&output)
    );
    assert!(!bcf.exists(), "nothing is written when 3.0 is refused");
}

/// Bedroom #19 (x 0..6, y 0..3, 3 m high) entered through door #29 in its
/// south wall at x 0.3..1.2, bounding it. Bed #39, 0.5 m high, stands from
/// the south wall at x 1.4..3.4 up to y 2.2, leaving 0.8 m north of it; a
/// turning circle fits east of it only, the strip west of it being 1.4 m
/// wide.
fn bedroom_behind_a_bed() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,0.9,$,$,$)";
    let bed = "IFCFURNITURE('GID',$,$,$,$,PL,REP,$,$)";
    model_with(&format!(
        "{}{}{}\
         #300=IFCRELSPACEBOUNDARY('0000000000000000000300',$,$,$,#19,#29,$,.PHYSICAL.,.INTERNAL.);\n",
        placed_box(10, [3.0, 1.5, 0.0], [6.0, 3.0, 3.0], space),
        placed_box(20, [0.75, -0.1, 0.0], [0.9, 0.2, 2.1], door),
        placed_box(30, [2.4, 1.1, 0.0], [2.0, 2.2, 0.5], bed),
    ))
}

#[test]
fn with_geometry_a_turning_circle_behind_a_bed_is_not_reached_from_the_door() {
    let case = Case::new("geometry-free-floor-entrance-path");
    let check = |width: f64| {
        case.geometry_rule(
            &bedroom_behind_a_bed(),
            &[("door", "IfcDoor"), ("space", "IfcSpace")],
            "axioval:capability.free-floor-circle",
            &registry_signature("axioval:capability.free-floor-circle"),
            entity("space"),
            json!({
                "diameter_metres": {"type": "number", "value": 1.5},
                "height_metres": {"type": "number", "value": 2.0},
                "entrance_path_width": {"type": "number", "value": width},
                "access_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:backward"]},
                "door_selector": {"type": "selector", "value": entity("door")},
            }),
        )
    };
    let (output, result) = check(1.2);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#19".to_owned(),
            "NO_FREE_FLOOR_SPACE_FOR_CIRCLE: the shape fits only where no path 1.2 m wide \
             from an entrance reaches it"
                .to_owned()
        )],
        "{result:#}"
    );
    // A 0.7 m path passes north of the bed.
    let (output, result) = check(0.7);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(finding_messages(&result), [], "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Room #19 (x 0..6, y 0..4, 3 m high) entered through door #29 in its
/// south wall at x 0.5..1.5; WC #39 in its south-east corner; skirting #49,
/// 0.1 m high, runs across it at x 3.0..3.1.
fn room_with_a_skirting() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
    let wc = "IFCFURNITURE('GID',$,$,$,$,PL,REP,$,$)";
    let skirting = "IFCBUILDINGELEMENTPROXY('GID',$,$,$,$,PL,REP,$,$)";
    model_with(&format!(
        "{}{}{}{}\
         #300=IFCRELSPACEBOUNDARY('0000000000000000000300',$,$,$,#19,#29,$,.PHYSICAL.,.INTERNAL.);\n",
        placed_box(10, [3.0, 2.0, 0.0], [6.0, 4.0, 3.0], space),
        placed_box(20, [1.0, -0.1, 0.0], [1.0, 0.2, 2.1], door),
        placed_box(30, [5.6, 0.55, 0.0], [0.6, 0.7, 0.8], wc),
        placed_box(40, [3.05, 2.0, 0.0], [0.1, 4.0, 0.1], skirting),
    ))
}

#[test]
fn with_geometry_local_circulation_ignores_a_skirting_below_the_band() {
    let case = Case::new("geometry-local-circulation-band");
    let check = |extra: Value| {
        let mut parameters = json!({
            "component_selector": {"type": "selector", "value": entity("furniture")},
            "space_path": {"type": "stringList", "value": ["axioval:derived.contained-in-space"]},
            "access_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:backward"]},
            "door_selector": {"type": "selector", "value": entity("door")},
            "width_metres": {"type": "number", "value": 0.9},
            "clear_height_metres": {"type": "number", "value": 2.0},
        });
        for (name, value) in extra.as_object().unwrap() {
            parameters[name] = value.clone();
        }
        case.geometry_rule(
            &room_with_a_skirting(),
            &[
                ("door", "IfcDoor"),
                ("space", "IfcSpace"),
                ("furniture", "IfcFurniture"),
            ],
            "axioval:capability.local-circulation",
            &registry_signature("axioval:capability.local-circulation"),
            entity("space"),
            parameters,
        )
    };
    let (output, result) = check(json!({}));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert_eq!(
        finding_messages(&result),
        [(
            "#39".to_owned(),
            "no entrance of ifc-step:model.ifc/#19 reaches it on a path 0.9 m wide".to_owned()
        )],
        "{result:#}"
    );
    let (output, result) = check(json!({
        "band_from_metres": {"type": "number", "value": 0.2},
    }));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(finding_messages(&result), [], "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// The room with a skirting, its door #29 stating a clear width of 0.8 m
/// in `Access.ClearWidth`, and store #59 (x 10..14) reached by no door.
fn rooms_with_a_narrow_door_and_a_store() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
    let wc = "IFCFURNITURE('GID',$,$,$,$,PL,REP,$,$)";
    let skirting = "IFCBUILDINGELEMENTPROXY('GID',$,$,$,$,PL,REP,$,$)";
    model_with(&format!(
        "{}{}{}{}{}\
         #300=IFCRELSPACEBOUNDARY('0000000000000000000300',$,$,$,#19,#29,$,.PHYSICAL.,.INTERNAL.);\n\
         #310=IFCPROPERTYSINGLEVALUE('ClearWidth',$,IFCPOSITIVELENGTHMEASURE(0.8),$);\n\
         #311=IFCPROPERTYSET('0000000000000000000311',$,'Access',$,(#310));\n\
         #312=IFCRELDEFINESBYPROPERTIES('0000000000000000000312',$,$,$,(#29),#311);\n",
        placed_box(10, [3.0, 2.0, 0.0], [6.0, 4.0, 3.0], space),
        placed_box(20, [1.0, -0.1, 0.0], [1.0, 0.2, 2.1], door),
        placed_box(30, [5.6, 0.55, 0.0], [0.6, 0.7, 0.8], wc),
        placed_box(40, [3.05, 2.0, 0.0], [0.1, 4.0, 0.1], skirting),
        placed_box(50, [12.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
    ))
}

#[test]
fn with_geometry_local_circulation_requires_entrances_as_wide_as_the_path() {
    let case = Case::new("geometry-local-circulation-entrances");
    let check = |extra: Value| {
        let mut parameters = json!({
            "component_selector": {"type": "selector", "value": entity("furniture")},
            "space_path": {"type": "stringList", "value": ["axioval:derived.contained-in-space"]},
            "access_path": {"type": "stringList", "value": ["IfcRelSpaceBoundary:backward"]},
            "door_selector": {"type": "selector", "value": entity("door")},
            "width_metres": {"type": "number", "value": 0.9},
            "clear_height_metres": {"type": "number", "value": 2.0},
            "band_from_metres": {"type": "number", "value": 0.2},
        });
        for (name, value) in extra.as_object().unwrap() {
            parameters[name] = value.clone();
        }
        case.geometry_rule(
            &rooms_with_a_narrow_door_and_a_store(),
            &[
                ("door", "IfcDoor"),
                ("space", "IfcSpace"),
                ("furniture", "IfcFurniture"),
            ],
            "axioval:capability.local-circulation",
            &registry_signature("axioval:capability.local-circulation"),
            entity("space"),
            parameters,
        )
    };
    // Off by default: the WC is reached and the empty store has nothing to
    // judge.
    let (output, result) = check(json!({}));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(finding_messages(&result), [], "{result:#}");
    let (output, result) = check(json!({
        "require_entrances": {"type": "boolean", "value": true},
        "check_entrance_width": {"type": "boolean", "value": true},
        "clear_width_property": {"type": "propertyReference",
                                 "property": "axioval:example.ifc.clear-width",
                                 "propertySet": "axioval:example.ifc.pset-access"},
    }));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 2, "{result:#}");
    assert_eq!(findings[0].0, "#19", "{result:#}");
    assert!(
        findings[0].1.starts_with(
            "entrance ifc-step:model.ifc/#29 is narrower than the path: clear width ("
        ) && findings[0]
            .1
            .ends_with("is 0.8 m; required at least 0.9 m, the path's width"),
        "{result:#}"
    );
    assert_eq!(findings[1].0, "#59", "{result:#}");
    assert!(
        findings[1]
            .1
            .starts_with("ifc-step:model.ifc/#59 has no entrance: no door or opening reaches it"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// Rooms #19 (x 0..4) and #29 (x 4.2..8.2), both with their floor at 0 m,
/// 3 m high and 4 m deep. Door #39 between them, at y 0.5..1.5,
/// has its bottom 4 cm above the floors; door #49, at y 2.5..3.5, stands on
/// them. Both are 2.1 m high and 0.1 m thick, and state no threshold.
/// `Pset_SpaceCommon.Reference` gives each room's use.
fn doors_on_sills() -> String {
    let space = "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)";
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
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
         {}{}{}{}\
         #200=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('Office'),$);\n\
         #201=IFCPROPERTYSET('0000000000000000000201',$,'Pset_SpaceCommon',$,(#200));\n\
         #202=IFCRELDEFINESBYPROPERTIES('0000000000000000000202',$,$,$,(#19,#29),#201);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        placed_box(10, [2.0, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(20, [6.2, 2.0, 0.0], [4.0, 4.0, 3.0], space),
        placed_box(30, [4.1, 1.0, 0.04], [0.1, 1.0, 2.1], door),
        placed_box(40, [4.1, 3.0, 0.0], [0.1, 1.0, 2.1], door),
    )
}

#[test]
fn with_geometry_a_door_sill_above_the_floor_is_found() {
    let case = Case::new("geometry-threshold-step");
    let (output, result) = case.geometry_rule(
        &doors_on_sills(),
        &[("door", "IfcDoor"), ("space", "IfcSpace")],
        "axioval:capability.keyed-limit",
        &registry_signature("axioval:capability.keyed-limit"),
        entity("door"),
        json!({
            "limits": {"type": "table", "value": [
                {"key_1": {"type": "string", "value": "*"},
                 "maximum": {"type": "number", "value": 0.02}},
            ]},
            "quantity": {"type": "string", "value": "threshold-step"},
            "floor_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
            "key_1": {"type": "propertyReference",
                      "property": "axioval:example.ifc.reference",
                      "propertySet": "axioval:example.ifc.pset-space-common"},
            "key_1_path": {"type": "stringList", "value": ["axioval:derived.adjacent-space"]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // #39's bottom lies 4 cm above both floors, #49's on them.
    let findings = finding_messages(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#39", "{result:#}");
    assert!(
        findings[0].1.starts_with("the step from the floor of ")
            && findings[0]
                .1
                .contains("to the door's bottom is 0.04 m; required at most 0.02 m"),
        "{result:#}"
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

#[test]
fn bcf_topics_are_labelled_by_folder_and_tags_of_each_ruleset() {
    let case = Case::new("bcf-rule-labels");
    let nested = |package: &str, folder: &str| {
        let path = case.ruleset_as(package);
        let mut ruleset: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let rule = ruleset["root"]["rules"][0].take();
        ruleset["root"]["rules"] = json!([]);
        ruleset["root"]["folders"] = json!([{
            "id": "f", "name": {"default": folder, "translations": {}},
            "rules": [rule], "folders": [],
        }]);
        case.write(&format!("{package}.json"), &ruleset.to_string())
    };
    let rulesets = [
        nested("org.example.client", "Handover"),
        nested("org.example.discipline", "Walls"),
    ];
    let model = case.write("model.ifc", &ifc("0000000000000000000002", false));
    let bcf = case.path("issues.bcfzip");
    let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
    command
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(case.definitions(true))
        .args(["--bcf", bcf.to_str().unwrap()])
        .env("SOURCE_DATE_EPOCH", "1790416800");
    for ruleset in &rulesets {
        command.arg("--ruleset").arg(ruleset);
    }
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

    let archive = openbim_bcf::read_path(&bcf).unwrap();
    let mut labels: Vec<Vec<String>> = archive
        .topics()
        .map(|markup| markup.topic.labels.clone())
        .collect();
    labels.sort();
    assert_eq!(
        labels,
        [
            ["org.example.client/r1", "Folder: Handover", "example"],
            ["org.example.discipline/r1", "Folder: Walls", "example"],
        ]
    );
}

/// Space #16 (x 0..4) inside furniture #26 (x -1..5), and wall #36 far
/// away at x 20..24; every body 4 m deep in y.
fn space_in_furniture() -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         {}{}{}\
         ENDSEC;\nEND-ISO-10303-21;\n",
        body(
            10,
            2.0,
            4.0,
            3.0,
            "IFCSPACE('0000000000000000000016',$,$,$,$,#3,REP,$,.ELEMENT.,$,$)"
        ),
        body(
            20,
            2.0,
            6.0,
            1.0,
            "IFCFURNITURE('0000000000000000000026',$,$,$,$,#3,REP,$,$)"
        ),
        body(
            30,
            22.0,
            4.0,
            3.0,
            "IFCWALL('0000000000000000000036',$,$,$,$,#3,REP,$,$)"
        ),
    )
}

/// The rule chooses the elements bounding a space: bounded only by
/// furniture, its boundary is uncovered when walls bound it and covered once
/// furniture is selected.
#[test]
fn with_geometry_space_validation_selects_its_bounding_elements() {
    let run = |name: &str, boundary: Value| {
        Case::new(name).geometry_rule(
            &space_in_furniture(),
            &[("space", "IfcSpace"), ("furniture", "IfcFurniture")],
            "axioval:capability.space-validation",
            &registry_signature("axioval:capability.space-validation"),
            entity("space"),
            json!({
                "required_height_metres": {"type": "number", "value": 2.5},
                "uncovered_segment_length_metres": {"type": "number", "value": 0.5},
                "check_top_cap": {"type": "boolean", "value": false},
                "check_bottom_cap": {"type": "boolean", "value": false},
                "check_unallocated_area": {"type": "boolean", "value": false},
                "maximum_unallocated_area_square_metres": {"type": "number", "value": 1.0},
                "intersection_elements": {"type": "selector", "value": entity("wall")},
                "boundary_elements": {"type": "selector", "value": boundary},
            }),
        )
    };
    let (output, result) = run("geometry-space-walls", entity("wall"));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = sorted_findings(&result);
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0].0, "#16");
    assert!(
        findings[0].1.starts_with("uncovered_boundary"),
        "{result:#}"
    );

    let furniture = json!({"kind": "anyOf", "operands": [entity("wall"), entity("furniture")]});
    let (output, result) = run("geometry-space-furniture", furniture);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(sorted_findings(&result).is_empty(), "{result:#}");
}

/// Every wall of the envelope model declared internal, #56 included.
fn envelope_model_all_internal() -> String {
    envelope_model()
        .replace("IFCBOOLEAN(.T.)", "IFCBOOLEAN(.F.)")
        .replace(
            "#90=IFCZONE",
            "#100=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.F.),$);\n\
             #101=IFCPROPERTYSET('0000000000000000000101',$,'Pset_WallCommon',$,(#100));\n\
             #102=IFCRELDEFINESBYPROPERTIES('0000000000000000000102',$,$,$,(#56),#101);\n\
             #90=IFCZONE",
        )
}

/// A model declaring no wall external is one major finding against the
/// model, never one per wall on the envelope.
#[test]
fn with_geometry_a_model_declaring_nothing_external_is_one_source_finding() {
    let case = Case::new("geometry-envelope-all-internal");
    let (output, result) = case.geometry_rule(
        &envelope_model_all_internal(),
        &[("space", "IfcSpace"), ("zone", "IfcZone")],
        "axioval:capability.external-wall-validation",
        &registry_signature("axioval:capability.external-wall-validation"),
        entity("wall"),
        json!({
            "derivations": {"type": "stringList", "value": ["all-spaces", "gross-area-groups"]},
            "bounding_selector": {"type": "selector", "value": entity("space")},
            "gross_area_group_selector": {"type": "selector", "value": entity("zone")},
            "gross_area_group_path": {"type": "stringList",
                                      "value": ["IfcRelAssignsToGroup:forward"]},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert!(findings[0]["object_id"].is_null(), "{result:#}");
    assert_eq!(findings[0]["severity"], "error", "{result:#}");
    assert!(
        findings[0]["message"]
            .as_str()
            .unwrap()
            .contains("no selected object is declared external"),
        "{result:#}"
    );
}

/// Every wall opening's head is 0.9 m below the wall top: a 0.5 m maximum
/// from the top edge finds each, a 1 m one none.
#[test]
fn wall_opening_heads_too_far_below_the_wall_top_are_found() {
    let metres = |value: f64| json!({"type": "quantity", "value": value, "unit": "m"});
    let run = |name: &str, maximum: f64| {
        opening_zone(
            name,
            "wall",
            json!({
                "length_axis": {"type": "string", "value": "profile-x"},
                "height_axis": {"type": "string", "value": "extrusion"},
                "edge_distance_maximum": metres(maximum),
                "maximum_edges": {"type": "string", "value": "top"},
            }),
        )
    };
    let (output, result) = run("opening-zone-head", 0.5);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let messages = finding_messages(&result);
    for opening in ["#100", "#200", "#300"] {
        assert!(
            messages.contains(&(
                opening.to_owned(),
                "opening is 0.9 m from the top edge of its host #10; at most 0.5 m allowed"
                    .to_owned()
            )),
            "{result:#}"
        );
    }
    let (_, result) = run("opening-zone-head-lenient", 1.0);
    assert!(
        finding_messages(&result)
            .iter()
            .all(|(_, message)| !message.contains("top edge")),
        "{result:#}"
    );
}

/// Walls #10 (5 m) and #20 (4 m), 3 m high, both stating a net side area
/// of 15 m², in a project measured in metres and square metres.
fn walls_stating_side_areas() -> String {
    let mut data = String::new();
    for (first, y, length) in [(10u32, 0.0, 5.0), (20, 5.0, 4.0)] {
        let [p, pos, profile, solid, shape, product, wall] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        let _ = write!(
            data,
            "#{p}=IFCCARTESIANPOINT(({x},{y}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length:?},0.2);\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,3.);\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{product}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{wall}=IFCWALL('{wall:022}',$,$,$,$,#3,#{product},$,$);\n{}",
            wall_areas(first + 100, wall, 15.0, 15.0),
            x = length / 2.0,
        );
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.);\n\
         #8=IFCUNITASSIGNMENT((#6,#7));\n\
         #9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,(#5),#8);\n\
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn with_geometry_a_stated_side_area_is_divided_by_the_measured_face() {
    let case = Case::new("property-requirements-face-area");
    let (output, result) = case.geometry_rule(
        &walls_stating_side_areas(),
        &[],
        "axioval:capability.property-requirements",
        &registry_signature("axioval:capability.property-requirements"),
        entity("wall"),
        json!({"requirements": {"type": "table", "value": [{
            "property_set": {"type": "string", "value": "axioval:example.ifc.qto-wall"},
            "property": {"type": "string", "value": "axioval:example.ifc.net-side-area"},
            "requirement": {"type": "string", "value": "required"},
            "minimum": {"type": "number", "value": 0.99},
            "maximum": {"type": "number", "value": 1.01},
            "unit": {"type": "string", "value": "m2"},
            "per": {"type": "string", "value": "measured-face-area"},
        }]}}),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // The 5 m wall's side is 15 m², the 4 m wall's only 12 m².
    assert_eq!(
        finding_messages(&result),
        [(
            "#26".to_owned(),
            "wrong value: axioval:example.ifc.qto-wall.axioval:example.ifc.net-side-area is \
             15 m² (1.25 m² per m² of measured face area); required between 0.99 and 1.01 m2 per m² of measured face \
             area (requirement row 0)"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Runs one rule of `capability` (its registry signature) over
/// [`storeys_with_facades`], applied to `applies_to`, with the storey
/// metric definitions and the `Name` attribute bound.
fn storey_rule(name: &str, capability: &str, applies_to: &str, parameters: Value) -> Value {
    storey_rule_with(name, capability, applies_to, parameters, &[], Value::Null)
}

/// [`storey_rule`] with `files` written beside the ruleset, by name, and
/// the ruleset's `relations` unless null.
fn storey_rule_with(
    name: &str,
    capability: &str,
    applies_to: &str,
    parameters: Value,
    files: &[(&str, &str)],
    relations: Value,
) -> Value {
    let (output, result) = storey_run(
        name,
        (capability, applies_to, parameters),
        files,
        relations,
        &[],
    );
    let result = result.unwrap_or_else(|| panic!("{}", stderr(&output)));
    assert_eq!(
        output.status.code(),
        Some(
            if !result["report"]["findings"]
                .as_array()
                .is_none_or(Vec::is_empty)
            {
                3
            } else if !result["report"]["not_evaluated"]
                .as_array()
                .is_none_or(Vec::is_empty)
            {
                4
            } else {
                0
            }
        ),
        "{}",
        stderr(&output)
    );
    result
}

/// Runs [`storey_rule_with`]'s check with `extra` arguments, `{dir}`
/// standing for the case's directory, and returns its output and the
/// result it saved, if any.
fn storey_run(
    name: &str,
    (capability, applies_to, parameters): (&str, &str, Value),
    files: &[(&str, &str)],
    relations: Value,
    extra: &[&str],
) -> (Output, Option<Value>) {
    let case = Case::new(name);
    for (file, contents) in files {
        case.write(file, contents);
    }
    let text = |value: &str| json!({"default": value, "translations": {}});
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(storey_metric_definitions(&case)).unwrap())
            .unwrap();
    definitions["properties"]["axioval:example.ifc.Name"] = json!({
        "id": "axioval:example.ifc.Name", "name": text("Name"), "valueKind": "string",
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Name"}], "citations": [],
    });
    let capability = format!("axioval:capability.{capability}");
    definitions["definitions"]["axioval:example.under-test"] = json!({
        "id": "axioval:example.under-test", "name": text("under test"),
        "description": text("under test"), "capability": capability,
        "parameters": registry_signature(&capability), "citations": [], "tags": [],
    });
    let text_file = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text_file).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("under-test");
    rule["definitionId"] = json!("axioval:example.under-test");
    rule["parameters"] = parameters;
    rule["applicability"]["groups"]["walls"]["selector"] = entity(applies_to);
    if !relations.is_null() {
        ruleset["relations"] = relations;
    }
    let model = case.write("model.ifc", &storeys_with_facades());
    let definitions = case.write("definitions.json", &definitions.to_string());
    let ruleset = case.write("ruleset.json", &ruleset.to_string());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .args(extra.iter().map(|arg| {
            arg.replace(
                "{dir}",
                case.path("").to_str().unwrap().trim_end_matches('/'),
            )
        }))
        .output()
        .unwrap();
    let result = std::fs::read_to_string(&saved)
        .ok()
        .map(|text| serde_json::from_str(&text).unwrap());
    (output, result)
}

fn name_attribute() -> Value {
    json!({"type": "propertyReference", "property": "axioval:example.ifc.Name",
           "propertySet": "axioval:attributes"})
}

#[test]
fn table_allocation_rows_are_keyed_per_storey() {
    let row = |anchor: &str, count: i64| {
        json!({"anchor": {"type": "string", "value": anchor},
               "count": {"type": "integer", "value": count}})
    };
    let result = storey_rule(
        "table-allocation-per-storey",
        "table-allocation",
        "IfcSpace",
        json!({
            "rows": {"type": "table", "value": [row("EG", 2), row("OG", 1)]},
            "anchor_key": name_attribute(),
            "anchor_selector": {"type": "selector", "value": entity("IfcBuildingStorey")},
            "relationship": {"type": "string", "value": "IfcRelAggregates"},
        }),
    );
    // Each storey holds one space: the ground storey's row asks for two.
    assert_eq!(
        finding_messages(&result),
        [(
            "#101".to_owned(),
            "row 1 (any object) in anchors like `EG` has 1 object(s); required exactly 2"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn table_allocation_rows_read_from_a_csv_file_beside_the_ruleset() {
    use sha2::Digest as _;
    let programme = "storey,spaces\r\nEG,2\r\nOG,1\r\n";
    let digest =
        sha2::Sha256::digest(programme.as_bytes())
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    let result = storey_rule_with(
        "table-allocation-from-csv",
        "table-allocation",
        "IfcSpace",
        json!({
            "rows": {"type": "tableFile", "path": "programme.csv", "sha256": digest, "columns": [
                {"id": "anchor", "header": "storey", "kind": "textPattern"},
                {"id": "count", "header": "spaces", "kind": "integer"},
            ]},
            "anchor_key": name_attribute(),
            "anchor_selector": {"type": "selector", "value": entity("IfcBuildingStorey")},
            "relationship": {"type": "string", "value": "IfcRelAggregates"},
        }),
        &[("programme.csv", programme)],
        Value::Null,
    );
    // As `table_allocation_rows_are_keyed_per_storey` with the rows inline.
    assert_eq!(
        finding_messages(&result),
        [(
            "#101".to_owned(),
            "row 1 (any object) in anchors like `EG` has 1 object(s); required exactly 2"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn a_relation_listed_in_a_csv_file_is_followed_and_an_unknown_object_reported() {
    use sha2::Digest as _;
    let pairs = "storey,space\n\
                 ifc-step:model.ifc/#101,ifc-step:model.ifc/#49\n\
                 ifc-step:model.ifc/#102,ifc-step:model.ifc/#999\n";
    let digest =
        sha2::Sha256::digest(pairs.as_bytes())
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    let row = |anchor: &str| {
        json!({"anchor": {"type": "string", "value": anchor},
               "count": {"type": "integer", "value": 1}})
    };
    let result = storey_rule_with(
        "relation-from-csv",
        "table-allocation",
        "IfcSpace",
        json!({
            "rows": {"type": "table", "value": [row("EG"), row("OG")]},
            "anchor_key": name_attribute(),
            "anchor_selector": {"type": "selector", "value": entity("IfcBuildingStorey")},
            "relationship": {"type": "string", "value": "axioval:derived.relation;id=holds"},
        }),
        &[("holds.csv", pairs)],
        json!({"holds": {
            "id": "holds",
            "name": {"default": "holds", "translations": {}},
            "from": entity("IfcBuildingStorey"),
            "to": entity("IfcSpace"),
            "by": {"kind": "pairs", "pairs": {
                "type": "tableFile", "path": "holds.csv", "sha256": digest, "columns": [
                    {"id": "from", "header": "storey", "kind": "string"},
                    {"id": "to", "header": "space", "kind": "string"},
                ]}},
        }}),
    );
    // The ground storey holds its space through the relation; the upper
    // storey's pair names no object, so it is reported and left undecided.
    let records = result["integrity"].as_array().unwrap();
    assert!(
        records
            .iter()
            .any(|record| record["code"] == "relation-object-unknown"
                && record["message"]
                    .as_str()
                    .unwrap()
                    .contains("`ifc-step:model.ifc/#999` is no object of the model")),
        "{result:#}"
    );
    assert!(
        !finding_messages(&result)
            .iter()
            .any(|(subject, _)| subject == "#101"),
        "{result:#}"
    );
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(
        open.iter()
            .any(|outcome| outcome["message"].as_str().unwrap().contains("#999")),
        "{result:#}"
    );
}

/// The storeys holding their spaces, as
/// [`a_relation_listed_in_a_csv_file_is_followed_and_an_unknown_object_reported`]
/// checks it, with the pairs `holds` supplied at check time by `extra`.
fn supplied_holds(name: &str, files: &[(&str, &str)], extra: &[&str]) -> (Output, Option<Value>) {
    let row = |anchor: &str| {
        json!({"anchor": {"type": "string", "value": anchor},
               "count": {"type": "integer", "value": 1}})
    };
    storey_run(
        name,
        (
            "table-allocation",
            "IfcSpace",
            json!({
                "rows": {"type": "table", "value": [row("EG"), row("OG")]},
                "anchor_key": name_attribute(),
                "anchor_selector": {"type": "selector", "value": entity("IfcBuildingStorey")},
                "relationship": {"type": "string", "value": "axioval:derived.relation;id=holds"},
            }),
        ),
        files,
        json!({"holds": {
            "id": "holds",
            "name": {"default": "holds", "translations": {}},
            "from": entity("IfcBuildingStorey"),
            "to": entity("IfcSpace"),
            "by": {"kind": "supplied", "columns": [
                {"id": "from", "header": "storey", "kind": "string"},
                {"id": "to", "header": "space", "kind": "string"},
            ]},
        }}),
        extra,
    )
}

#[test]
fn a_relation_supplied_beside_the_model_is_followed_and_its_file_recorded() {
    use sha2::Digest as _;
    let pairs = "storey,space\n\
                 ifc-step:model.ifc/#101,ifc-step:model.ifc/#49\n\
                 ifc-step:model.ifc/#102,ifc-step:model.ifc/#999\n";
    let digest =
        sha2::Sha256::digest(pairs.as_bytes())
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    let (output, result) = supplied_holds(
        "relation-supplied",
        &[("holds.csv", pairs)],
        &["--relations", "holds={dir}/holds.csv"],
    );
    let result = result.unwrap_or_else(|| panic!("{}", stderr(&output)));
    // As the same pairs listed in the package: the ground storey holds its
    // space, the upper storey's pair names no object and is reported.
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        result["integrity"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["code"] == "relation-object-unknown"
                && record["message"]
                    .as_str()
                    .unwrap()
                    .contains("`ifc-step:model.ifc/#999` is no object of the model")),
        "{result:#}"
    );
    assert!(
        !finding_messages(&result)
            .iter()
            .any(|(subject, _)| subject == "#101"),
        "{result:#}"
    );
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(
        open.iter()
            .any(|outcome| outcome["message"].as_str().unwrap().contains("#999")),
        "{result:#}"
    );
    assert_eq!(
        result["relation_files"],
        json!([{"relation": "holds", "file": "holds.csv", "sha256": digest, "pairs": 2}]),
        "{result:#}"
    );
}

#[test]
fn a_supplied_relation_given_no_file_leaves_its_rules_not_evaluated() {
    let (output, result) = supplied_holds("relation-unsupplied", &[], &[]);
    let result = result.unwrap_or_else(|| panic!("{}", stderr(&output)));
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(result["report"]["findings"], json!([]), "{result:#}");
    let open = result["report"]["not_evaluated"].as_array().unwrap();
    assert!(!open.is_empty(), "{result:#}");
    assert!(
        open.iter().any(|outcome| outcome["message"]
            .as_str()
            .unwrap()
            .contains("none were supplied")),
        "{result:#}"
    );
    assert!(
        result["integrity"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["code"] == "relation-pairs-not-supplied"),
        "{result:#}"
    );
    assert!(result.get("relation_files").is_none(), "{result:#}");
}

#[test]
fn a_relation_file_for_an_undeclared_relation_or_of_other_columns_fails_before_checking() {
    let pairs = "storey,space\nifc-step:model.ifc/#101,ifc-step:model.ifc/#49\n";
    let (output, result) = supplied_holds(
        "relation-undeclared",
        &[("feeds.csv", pairs)],
        &["--relations", "feeds={dir}/feeds.csv"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(result.is_none());
    assert!(
        stderr(&output).contains("no ruleset declares the relation `feeds`"),
        "{}",
        stderr(&output)
    );
    let (output, result) = supplied_holds(
        "relation-wrong-columns",
        &[(
            "holds.csv",
            "from,to\nifc-step:model.ifc/#101,ifc-step:model.ifc/#49\n",
        )],
        &["--relations", "holds={dir}/holds.csv"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(result.is_none());
    assert!(
        stderr(&output).contains("the header names column `from`, which is not declared"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn group_composition_reports_a_required_storey_missing_from_the_model() {
    let row = |group: &str| {
        json!({"group": {"type": "string", "value": group},
               "count": {"type": "integer", "value": 1}})
    };
    let result = storey_rule(
        "group-composition-absent-storey",
        "group-composition",
        "IfcBuildingStorey",
        json!({
            "requirements": {"type": "table", "value": [row("EG"), row("OG"), row("UG")]},
            "group_key_1": name_attribute(),
            "member_selector": {"type": "selector", "value": entity("IfcSpace")},
            "relationship": {"type": "string", "value": "IfcRelAggregates"},
            "report_absent_groups": {"type": "boolean", "value": true},
        }),
    );
    let findings: Vec<&str> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        findings,
        [
            "not in model: no group matches row 3 (axioval:attributes.axioval:example.ifc.Name \
          like `UG`)"
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn keyed_limits_bound_each_storeys_summed_space_area() {
    let limit = |pattern: &str, minimum: f64, maximum: f64| {
        json!({"key_1": {"type": "string", "value": pattern},
               "minimum": {"type": "number", "value": minimum},
               "maximum": {"type": "number", "value": maximum}})
    };
    let result = storey_rule(
        "keyed-limit-storey-areas",
        "keyed-limit",
        "IfcBuildingStorey",
        json!({
            "limits": {"type": "table", "value": [limit("EG*", 30.0, 50.0), limit("OG*", 50.0, 60.0)]},
            "quantity": {"type": "string", "value": "member-plan-area"},
            "key_1": name_attribute(),
            "member_selector": {"type": "selector", "value": entity("IfcSpace")},
            "relationship": {"type": "string", "value": "IfcRelAggregates"},
        }),
    );
    // Each storey holds one 10 m × 4 m space.
    assert_eq!(
        finding_messages(&result),
        [(
            "#102".to_owned(),
            "summed plan area of the members via IfcRelAggregates is 40 m²; required at least \
             50 m² (limit row 1: axioval:attributes.axioval:example.ifc.Name `OG`)"
                .to_owned()
        )],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// [`storeys_with_facades`] with a 4 m × 4 m atrium #89, 6 m high, in the
/// ground storey: it rises through the upper storey too. The upper storey
/// is placed 3 m up, where its elevation says.
fn storeys_with_an_atrium() -> String {
    storeys_with_facades()
        .replace(
            "#100=IFCBUILDING(",
            &format!(
                "{}#100=IFCBUILDING(",
                placed_box(
                    80,
                    [20.0, 2.0, 0.0],
                    [4.0, 4.0, 6.0],
                    "IFCSPACE('GID',$,$,$,$,PL,REP,$,.ELEMENT.,$,$)"
                )
            ),
        )
        .replace("#101,(#49));", "#101,(#49,#89));")
        // The upper storey is placed at its elevation, as its band needs.
        .replace("$,'OG',$,$,#3,", "$,'OG',$,$,#122,")
        .replace(
            "#100=IFCBUILDING(",
            "#120=IFCCARTESIANPOINT((0.,0.,3.));\n\
             #121=IFCAXIS2PLACEMENT3D(#120,$,$);\n\
             #122=IFCLOCALPLACEMENT($,#121);\n\
             #100=IFCBUILDING(",
        )
}

#[test]
fn an_atrium_counts_in_every_storey_its_height_spans() {
    let case = Case::new("spans-level-atrium");
    let (output, result) = case.geometry_rule(
        &storeys_with_an_atrium(),
        &[("storey", "IfcBuildingStorey"), ("space", "IfcSpace")],
        "axioval:capability.plan-area",
        &registry_signature("axioval:capability.plan-area"),
        entity("storey"),
        json!({
            "maximum": {"type": "number", "value": 50.0},
            "member_selector": {"type": "selector", "value": entity("space")},
            "relationship": {"type": "string", "value": "axioval:derived.spans-level;overlap=1"},
            "direction": {"type": "string", "value": "backward"},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    // Each storey's 40 m² space and the 16 m² atrium.
    let expected = |storey: &str| {
        (
            storey.to_owned(),
            "summed plan area of the members is 56 m²; required at most 50 m²".to_owned(),
        )
    };
    assert_eq!(
        finding_messages(&result),
        [expected("#101"), expected("#102")],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

#[test]
fn area_ratio_measures_its_numerator_and_denominator_differently() {
    let result = storey_rule(
        "area-ratio-separate-measures",
        "area-ratio",
        "IfcBuildingStorey",
        json!({
            "numerator_measure": {"type": "string", "value": "facade"},
            "denominator_measure": {"type": "string", "value": "footprint"},
            "numerator_selector": {"type": "selector", "value": entity("wall")},
            "denominator_selector": {"type": "selector", "value": entity("wall")},
            "maximum": {"type": "number", "value": 5.0},
            "relationship": {"type": "string", "value": "IfcRelContainedInSpatialStructure"},
        }),
    );
    // The ground storey's wall: its 27 m² outer face net of the window and
    // two 0.9 m² free ends, over its 10 m × 0.3 m footprint.
    let findings = finding_messages(&result);
    assert!(
        findings.contains(&(
            "#101".to_owned(),
            "facade area to plan area ratio is 9.6 (28.8 m² of 3 m²); required at most 5"
                .to_owned()
        )),
        "{result:#}"
    );
}

/// Walls `…11` and `…12` and a slab, numbered from `first`. The wall
/// `GlobalId`s in `referenced` state a `Reference`; the others are findings.
fn revision(first: u64, referenced: &[&str]) -> String {
    let n = |offset: u64| first + offset;
    let walls = [
        ("0000000000000000000011", n(0)),
        ("0000000000000000000012", n(1)),
    ];
    let mut related: Vec<String> = walls
        .iter()
        .filter(|(global_id, _)| referenced.contains(global_id))
        .map(|(_, number)| format!("#{number}"))
        .collect();
    // The relationship needs an object; the slab is never checked.
    related.push(format!("#{}", n(2)));
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #{}=IFCWALL('{}',$,$,$,$,$,$,$,$);\n\
         #{}=IFCWALL('{}',$,$,$,$,$,$,$,$);\n\
         #{}=IFCSLAB('0000000000000000000013',$,$,$,$,$,$,$,$);\n\
         #{}=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W-1'),$);\n\
         #{}=IFCPROPERTYSET('0000000000000000000015',$,'Pset_WallCommon',$,(#{}));\n\
         #{}=IFCRELDEFINESBYPROPERTIES('0000000000000000000016',$,$,$,({}),#{});\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        walls[0].1,
        walls[0].0,
        walls[1].1,
        walls[1].0,
        n(2),
        n(3),
        n(4),
        n(3),
        n(5),
        related.join(","),
        n(4),
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn revision_two_carries_revision_ones_decisions_and_lists_stale_ones() {
    let case = Case::new("decisions");
    let axioval = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(case.path("."))
            .args(args)
            .output()
            .unwrap()
    };
    let read = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(case.path(name)).unwrap()).unwrap()
    };
    let text = |output: Output| String::from_utf8(output.stdout).unwrap();

    // Revision 1: both walls lack their reference.
    let report = case.path("r1.json");
    let output = case.check(
        &revision(1, &[]),
        true,
        &["--report", report.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let first = read("r1.json");
    let findings = first["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2, "{first:#}");
    let id = |finding: &Value| finding["id"].as_str().unwrap().to_owned();
    let (kept, fixed) = (id(&findings[0]), id(&findings[1]));

    // The reviewer accepts one and rejects the other.
    let decide = |finding: &str, status: &str, comment: &str| {
        axioval(&[
            "decide",
            "r1.json",
            "--decisions",
            "decisions.json",
            "--finding",
            finding,
            "--status",
            status,
            "--author",
            "A. Reviewer",
            "--comment",
            comment,
            "--date",
            "2026-09-27T08:00:00Z",
        ])
    };
    let output = decide(&kept, "accepted", "agreed with the architect");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = decide(&fixed, "rejected", "");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        read("decisions.json")["decisions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // An unknown finding is refused and nothing is written.
    let before = std::fs::read(case.path("decisions.json")).unwrap();
    let output = decide("00000000-0000-5000-8000-000000000000", "accepted", "");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no finding has id"),
        "{}",
        stderr(&output)
    );
    assert_eq!(std::fs::read(case.path("decisions.json")).unwrap(), before);

    // Revision 2 renumbers every entity and fixes the second wall.
    let bcf = case.path("r2.bcfzip");
    let output = case.check(
        &revision(101, &["0000000000000000000012"]),
        true,
        &[
            "--report",
            case.path("r2.json").to_str().unwrap(),
            "--decisions",
            case.path("decisions.json").to_str().unwrap(),
            "--bcf",
            bcf.to_str().unwrap(),
        ],
    );
    // Deciding a finding never changes the exit status.
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let second = read("r2.json");
    let findings = second["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{second:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#101");
    assert_eq!(id(&findings[0]), kept);
    assert_eq!(
        findings[0]["decision"],
        json!({"status": "accepted", "author": "A. Reviewer", "date": "2026-09-27T08:00:00Z",
               "comment": "agreed with the architect", "evidence": "unchanged"})
    );
    let stale = second["report"]["stale_decisions"].as_array().unwrap();
    assert_eq!(stale.len(), 1, "{second:#}");
    assert_eq!(stale[0]["finding"], json!(fixed));
    assert_eq!(stale[0]["status"], "rejected");

    // The BCF topic is the finding, accepted, with the decision's comment.
    let archive = openbim_bcf::read_path(&bcf).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );
    let markup = archive.topics().next().unwrap();
    assert_eq!(markup.topic.guid.as_deref(), Some(kept.as_str()));
    assert_eq!(markup.topic.topic_status.as_deref(), Some("Accepted"));
    assert_eq!(
        markup.comments[0].comment.as_deref(),
        Some("Accepted: agreed with the architect")
    );

    // The summary counts decisions and points at the stale one.
    let summary = text(axioval(&["report", "r2.json"]));
    assert!(
        summary.contains(
            "decisions: 1 accepted · 0 rejected · 0 open · 0 undecided · 0 changed · 1 stale"
        ),
        "{summary}"
    );
    assert!(
        summary.contains("axioval report r2.json --section stale-decisions"),
        "{summary}"
    );
    let listing = text(axioval(&[
        "report",
        "r2.json",
        "--section",
        "stale-decisions",
    ]));
    assert!(listing.contains(&format!("id: {fixed}")), "{listing}");
    assert!(
        listing.contains("rejected by A. Reviewer on 2026-09-27T08:00:00Z"),
        "{listing}"
    );
    let listing = text(axioval(&["report", "r2.json", "--decision", "accepted"]));
    assert!(listing.contains("showing 1–1 of 1"), "{listing}");
    assert!(
        listing.contains("decision: accepted by A. Reviewer"),
        "{listing}"
    );
    let listing = text(axioval(&["report", "r2.json", "--decision", "undecided"]));
    assert!(listing.contains("no matching entries"), "{listing}");

    // Without decisions the same check exits 3 and decides nothing.
    let output = case.check(&revision(101, &["0000000000000000000012"]), true, &[]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(id(&result["report"]["findings"][0]), kept);
    assert!(result["report"]["findings"][0].get("decision").is_none());
    assert!(result["report"].get("stale_decisions").is_none());
}

/// The topics of a BCF archive as the writer takes them, without
/// viewpoints: what another BCF tool writes back after a review.
fn reviewed_topics(path: &Path) -> Vec<openbim_bcf::write::Topic> {
    let archive = openbim_bcf::read_path(path).unwrap();
    archive
        .topics()
        .map(|markup| {
            let topic = &markup.topic;
            openbim_bcf::write::Topic {
                guid: topic.guid.clone().unwrap(),
                title: topic.title.clone().unwrap(),
                description: topic.description.clone(),
                topic_type: topic.topic_type.clone(),
                topic_status: topic.topic_status.clone(),
                priority: topic.priority.clone(),
                labels: topic.labels.clone(),
                creation_date: topic.creation_date.clone().unwrap(),
                creation_author: topic.creation_author.clone().unwrap(),
                ..openbim_bcf::write::Topic::default()
            }
        })
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_bcf_reviewed_elsewhere_decides_the_next_check() {
    let case = Case::new("decisions-from-bcf");
    let read = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(case.path(name)).unwrap()).unwrap()
    };
    let bcf = case.path("r1.bcfzip");
    let r1 = case.path("r1.json");
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            r1.to_str().unwrap(),
            "--bcf",
            bcf.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let first = read("r1.json");
    let ids: Vec<String> = first["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2, "{first:#}");

    // Another tool closes the first topic, comments on the second, and adds
    // a topic of its own.
    let mut topics = reviewed_topics(&bcf);
    let position = |id: &str| topics.iter().position(|t| t.guid == id).unwrap();
    let (closed, commented) = (position(&ids[0]), position(&ids[1]));
    topics[closed].topic_status = Some("Closed".into());
    topics[commented]
        .comments
        .push(openbim_bcf::write::Comment {
            guid: "7e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b".into(),
            date: "2026-09-28T09:30:00Z".into(),
            author: "B. Reviewer".into(),
            comment: "the reference is on the drawing".into(),
            viewpoint: None,
        });
    let mut foreign = topics[commented].clone();
    foreign.guid = "3f2504e0-4f89-41d3-9a0c-0305e82c3301".into();
    foreign.title = "Duct clashes with beam".into();
    foreign.comments.clear();
    topics.push(foreign);
    let reviewed = case.path("reviewed.bcfzip");
    openbim_bcf::write::to_path(
        &openbim_bcf::write::Document {
            version: openbim_bcf::write::TargetVersion::V2_1,
            extensions: None,
            topics,
        },
        &reviewed,
    )
    .unwrap();

    let report = case.path("r2.json");
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            report.to_str().unwrap(),
            "--decisions-from",
            reviewed.to_str().unwrap(),
        ],
    );
    // Deciding a finding never changes the exit status.
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("1 BCF topic(s) decided no current finding"),
        "{}",
        stderr(&output)
    );
    let second = read("r2.json");
    let findings = second["report"]["findings"].as_array().unwrap();
    let decision =
        |id: &str| findings.iter().find(|finding| finding["id"] == id).unwrap()["decision"].clone();
    assert_eq!(
        decision(&ids[0]),
        json!({"status": "accepted", "author": "axioval", "date": "2026-09-26T10:00:00Z",
               "evidence": "unknown"})
    );
    assert_eq!(
        decision(&ids[1]),
        json!({"status": "open", "author": "B. Reviewer", "date": "2026-09-28T09:30:00Z",
               "comments": [{"author": "B. Reviewer", "date": "2026-09-28T09:30:00Z",
                             "text": "the reference is on the drawing",
                             "id": "7e1d2c3b-4a59-4687-9a1b-2c3d4e5f6a7b"}],
               "evidence": "unknown"})
    );
    assert_eq!(
        second["unmatched_topics"],
        json!([{"guid": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
                "title": "Duct clashes with beam", "status": "Open", "reason": "no-finding"}])
    );

    // The summary points at the unmatched topic, and the listing names it.
    let axioval = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(case.path("."))
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    };
    let summary = axioval(&["report", "r2.json"]);
    assert!(
        summary.contains("axioval report r2.json --section unmatched-topics"),
        "{summary}"
    );
    let listing = axioval(&["report", "r2.json", "--section", "unmatched-topics"]);
    assert!(listing.contains("Duct clashes with beam"), "{listing}");
    assert!(
        listing.contains("id: 3f2504e0-4f89-41d3-9a0c-0305e82c3301"),
        "{listing}"
    );

    // A decisions file and a BCF archive are two sources of one thing.
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--decisions",
            "decisions.json",
            "--decisions-from",
            reviewed.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    // What is not a BCF archive fails the run, which writes nothing.
    let bogus = case.write("bogus.bcfzip", "not an archive");
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            case.path("r3.json").to_str().unwrap(),
            "--decisions-from",
            bogus.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("bogus.bcfzip"),
        "{}",
        stderr(&output)
    );
    assert!(!case.path("r3.json").exists());
}

/// Doors, each with a `Pset_DoorCommon` holding its number (`Reference`) and
/// fire rating, and a wall no rule selects. `first` offsets every entity
/// number, so each export numbers its entities and generates its
/// `GlobalId`s afresh.
fn door_export(first: u32, doors: &[(&str, &str)]) -> String {
    door_export_at(first, doors, "t")
}

/// As [`door_export`], its header stamped `stamp`.
fn door_export_at(first: u32, doors: &[(&str, &str)], stamp: &str) -> String {
    let door = "IFCDOOR('GID',$,$,$,$,PL,REP,$,2.1,1.,$,$,$)";
    let mut data = placed_box(
        first,
        [0.0, -1.0, 0.0],
        [8.0, 0.2, 3.0],
        "IFCWALL('GID',$,$,$,$,PL,REP,$,$)",
    );
    for (index, (number, rating)) in doors.iter().enumerate() {
        let index = u32::try_from(index).unwrap();
        let base = first + 20 * (index + 1);
        data.push_str(&placed_box(
            base,
            [2.0 * f64::from(index), 0.0, 0.0],
            [1.0, 0.1, 2.1],
            door,
        ));
        let (object, reference, fire, set, relation) =
            (base + 9, base + 10, base + 11, base + 12, base + 13);
        let _ = write!(
            data,
            "#{reference}=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('{number}'),$);\n\
             #{fire}=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('{rating}'),$);\n\
             #{set}=IFCPROPERTYSET('{set:022}',$,'Pset_DoorCommon',$,(#{reference},#{fire}));\n\
             #{relation}=IFCRELDEFINESBYPROPERTIES('{relation:022}',$,$,$,(#{object}),#{set});\n"
        );
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','{stamp}',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCLOCALPLACEMENT($,#2);\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
         #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #7=IFCUNITASSIGNMENT((#6));\n\
         #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn a_model_comparison_rule_matches_regenerated_doors_by_number() {
    let case = Case::new("model-comparison-rule");
    case.write(
        "base.ifc",
        &door_export(100, &[("D1", "EI30"), ("D2", "EI30"), ("D3", "EI30")]),
    );
    // Every GlobalId is new; D1's fire rating changed, D3 is gone, D4 new.
    case.write(
        "revised.ifc",
        &door_export(500, &[("D1", "EI60"), ("D2", "EI30"), ("D4", "EI30")]),
    );
    let (output, result) = case.geometry_rule_over(
        &["base.ifc:base", "revised.ifc:revised"],
        &[("door", "IfcDoor")],
        "axioval:capability.model-comparison",
        &registry_signature("axioval:capability.model-comparison"),
        entity("door"),
        json!({
            "base": {"type": "string", "value": "base"},
            "revised": {"type": "string", "value": "revised"},
            "identity_property": {"type": "propertyReference",
                                  "property": "axioval:example.ifc.reference"},
            "all_property_sets": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let mut findings: Vec<(String, String)> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["object_id"]["source"]["document"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                finding["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    findings.sort();
    assert_eq!(
        findings,
        vec![
            (
                "base.ifc".to_owned(),
                "removed (property axioval:example.ifc.reference:D3)".to_owned()
            ),
            (
                "revised.ifc".to_owned(),
                "added (property axioval:example.ifc.reference:D4)".to_owned()
            ),
            (
                "revised.ifc".to_owned(),
                "property changed: property Pset_DoorCommon.FireRating \"EI30\" -> \"EI60\""
                    .to_owned()
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Runs `model-comparison` over `base.ifc:base` and `revised.ifc:revised`
/// with `parameters` beside the two disciplines.
fn door_comparison(case: &Case, parameters: Value) -> (Output, Value) {
    let mut parameters = parameters;
    parameters["base"] = json!({"type": "string", "value": "base"});
    parameters["revised"] = json!({"type": "string", "value": "revised"});
    case.geometry_rule_over(
        &["base.ifc:base", "revised.ifc:revised"],
        &[("door", "IfcDoor")],
        "axioval:capability.model-comparison",
        &registry_signature("axioval:capability.model-comparison"),
        entity("door"),
        parameters,
    )
}

#[test]
fn with_geometry_doors_with_fresh_identities_match_by_their_bodies() {
    let case = Case::new("model-comparison-geometry");
    case.write(
        "base.ifc",
        &door_export(100, &[("D1", "EI30"), ("D2", "EI30"), ("D3", "EI30")]),
    );
    // The third door is renumbered where the old one stood.
    case.write(
        "revised.ifc",
        &door_export(500, &[("D1", "EI60"), ("D2", "EI30"), ("D4", "EI30")]),
    );
    let (output, result) = door_comparison(
        &case,
        json!({
            "match_by": {"type": "stringList", "value": ["geometry"]},
            "all_property_sets": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let messages: Vec<String> = finding_messages(&result)
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    assert_eq!(
        messages,
        vec![
            "property changed: property Pset_DoorCommon.FireRating \"EI30\" -> \"EI60\"",
            "property changed: property Pset_DoorCommon.Reference \"D3\" -> \"D4\"",
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// [`door_export`] with two storeys of stable `GlobalId`s, the first door
/// contained in storey `storey` (`1` or `2`).
fn doors_on_storeys(first: u32, doors: &[(&str, &str)], storey: u32) -> String {
    let (one, two, relation, door) = (first + 900, first + 901, first + 902, first + 29);
    let contained = if storey == 1 { one } else { two };
    door_export(first, doors).replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        &format!(
            "#{one}=IFCBUILDINGSTOREY('0000000000000000000S01',$,'EG',$,$,$,$,$,$,$);\n\
             #{two}=IFCBUILDINGSTOREY('0000000000000000000S02',$,'OG',$,$,$,$,$,$,$);\n\
             #{relation}=IFCRELCONTAINEDINSPATIALSTRUCTURE('{relation:022}',$,$,$,(#{door}),#{contained});\n\
             ENDSEC;\nEND-ISO-10303-21;"
        ),
    )
}

#[test]
fn a_model_comparison_rule_reports_a_door_moved_to_another_storey() {
    let case = Case::new("model-comparison-relationships");
    let doors = [("D1", "EI30"), ("D2", "EI30")];
    case.write("base.ifc", &doors_on_storeys(100, &doors, 1));
    case.write("revised.ifc", &doors_on_storeys(500, &doors, 2));
    let parameters = json!({
        "identity_scheme": {"type": "string", "value": "ifc-globalid"},
        "identity_property": {"type": "propertyReference",
                              "property": "axioval:example.ifc.reference"},
        "compare_relationships": {"type": "boolean", "value": true},
    });
    let (output, result) = door_comparison(&case, parameters);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let messages: Vec<String> = finding_messages(&result)
        .into_iter()
        .map(|(_, message)| message)
        .collect();
    assert_eq!(
        messages,
        vec![
            "relationship changed: containment -[0000000000000000000S01] \
             +[0000000000000000000S02]"
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");

    // The same storey in both: nothing changed.
    case.write("revised.ifc", &doors_on_storeys(500, &doors, 1));
    let (output, result) = door_comparison(
        &case,
        json!({
            "identity_scheme": {"type": "string", "value": "ifc-globalid"},
            "identity_property": {"type": "propertyReference",
                                  "property": "axioval:example.ifc.reference"},
            "compare_relationships": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(result["report"]["findings"], json!([]), "{result:#}");
}

#[test]
fn a_revised_model_older_than_its_base_is_an_error_finding() {
    let case = Case::new("model-comparison-timestamps");
    let doors = [("D1", "EI30")];
    case.write(
        "base.ifc",
        &door_export_at(100, &doors, "2024-05-01T10:00:00"),
    );
    case.write(
        "revised.ifc",
        &door_export_at(500, &doors, "2024-01-01T10:00:00"),
    );
    let (output, result) = door_comparison(
        &case,
        json!({
            "identity_property": {"type": "propertyReference",
                                  "property": "axioval:example.ifc.reference"},
            "compare_timestamps": {"type": "boolean", "value": true},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["severity"], "error", "{result:#}");
    assert!(
        findings[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("timestamp changed from `ifc-step:base.ifc`: the revised file is older"),
        "{result:#}"
    );
}

/// Walls 4 m long, 0.2 m thick and 3 m high, each with a 1 m wide, 2 m
/// high opening voiding it. A wall is `(GlobalId, y, x)`: placed at `y`,
/// starting at `x`, its opening `opening` metres along it. `first` offsets
/// every entity number, so each export numbers its entities afresh while
/// the walls keep their `GlobalId`s.
fn voided_wall_export(first: u32, walls: &[(&str, f64, f64, f64)]) -> String {
    let mut data = String::new();
    for (index, (global_id, y, x, opening)) in walls.iter().enumerate() {
        let base = first + 30 * u32::try_from(index).unwrap();
        data.push_str(&placed_box(
            base,
            [x + 2.0, y + 0.1, 0.0],
            [4.0, 0.2, 3.0],
            &format!("IFCWALL('{global_id}',$,$,$,$,PL,REP,$,$)"),
        ));
        data.push_str(&placed_box(
            base + 10,
            [x + opening + 0.5, y + 0.1, 0.5],
            [1.0, 0.4, 2.0],
            "IFCOPENINGELEMENT('GID',$,$,$,$,PL,REP,$,.OPENING.)",
        ));
        let _ = writeln!(
            data,
            "#{v}=IFCRELVOIDSELEMENT('{v:022}',$,$,$,#{wall},#{hole});",
            v = base + 20,
            wall = base + 9,
            hole = base + 19,
        );
    }
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
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

const MOVED_OPENING: &str = "1WallMovedOpening00001";
const RE_EXPORTED: &str = "1WallReExported0000001";
const SHIFTED: &str = "1WallShifted0000000001";

/// The base of [`voided_wall_export`] and its revision: the first wall's
/// opening moved 0.5 m within unchanged bounds, the second re-exported as
/// it was, the third moved 0.25 m along itself, exactly the tolerance.
fn wall_revisions(case: &Case) {
    case.write(
        "base.ifc",
        &voided_wall_export(
            100,
            &[
                (MOVED_OPENING, 0.0, 0.0, 1.0),
                (RE_EXPORTED, 5.0, 0.0, 1.0),
                (SHIFTED, 10.0, 0.0, 1.0),
            ],
        ),
    );
    case.write(
        "revised.ifc",
        &voided_wall_export(
            700,
            &[
                (SHIFTED, 10.0, 0.25, 1.0),
                (RE_EXPORTED, 5.0, 0.0, 1.0),
                (MOVED_OPENING, 0.0, 0.0, 1.5),
            ],
        ),
    );
}

#[test]
fn a_mesh_comparison_finds_a_moved_opening_inside_unchanged_bounds() {
    let case = Case::new("model-comparison-mesh");
    wall_revisions(&case);
    let (output, result) = case.geometry_rule_over(
        &["base.ifc:base", "revised.ifc:revised"],
        &[("wall", "IfcWall")],
        "axioval:capability.model-comparison",
        &registry_signature("axioval:capability.model-comparison"),
        entity("wall"),
        json!({
            "base": {"type": "string", "value": "base"},
            "revised": {"type": "string", "value": "revised"},
            "identity_scheme": {"type": "string", "value": "ifc-globalid"},
            "geometry": {"type": "string", "value": "mesh"},
            "tolerance_metres": {"type": "number", "value": 0.25},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    // Only the moved opening: the re-export is unchanged, the shift by the
    // tolerance undecided.
    assert_eq!(findings.len(), 1, "{result:#}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("geometry changed: geometry mesh differs by 0.5000 m"),
        "{result:#}"
    );
    let witness = findings[0]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|evidence| evidence["locator"].as_str())
        .find(|locator| locator.starts_with("comparison:witness:mesh:"));
    assert!(witness.is_some(), "{result:#}");
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 1, "{result:#}");
    assert!(
        not_evaluated[0]["message"]
            .as_str()
            .unwrap()
            .contains("geometry mesh differs by"),
        "{result:#}"
    );
    assert!(
        not_evaluated[0]["message"]
            .as_str()
            .unwrap()
            .ends_with("undetermined"),
        "{result:#}"
    );

    // Bounds alone see the shift, never the moved opening.
    let (output, result) = case.geometry_rule_over(
        &["base.ifc:base", "revised.ifc:revised"],
        &[("wall", "IfcWall")],
        "axioval:capability.model-comparison",
        &registry_signature("axioval:capability.model-comparison"),
        entity("wall"),
        json!({
            "base": {"type": "string", "value": "base"},
            "revised": {"type": "string", "value": "revised"},
            "identity_scheme": {"type": "string", "value": "ifc-globalid"},
            "compare_geometry": {"type": "boolean", "value": true},
            "length_tolerance": {"type": "number", "value": 0.1},
        }),
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert!(
        findings[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("geometry changed: geometry bounds differs by 0.2500 m"),
        "{result:#}"
    );
}

#[test]
fn an_auxiliary_rule_chooses_the_walls_another_checks_and_reports_nothing() {
    let case = Case::new("auxiliary-rule");
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let mut parent = ruleset["root"]["rules"][0].clone();
    parent["auxiliary"] = json!(true);
    let mut passed = parent.clone();
    passed["id"] = json!("a-recheck-passed-walls");
    passed["auxiliary"] = json!(false);
    passed["gate"] = json!({"rule": "wall-reference-required", "condition": "passedObjects"});
    ruleset["root"]["rules"] = json!([parent, passed]);
    let ruleset = case.write("auxiliary.json", &ruleset.to_string());
    let model = case.write("model.ifc", &ten_walls_two_without_reference());
    let definitions = case.definitions(true);
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .arg("--rule-status")
        .output()
        .unwrap();
    // The two walls without a reference fail only the auxiliary rule,
    // which reports nothing.
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(
        result["report"]["rules"],
        json!([{"rule_id": "a-recheck-passed-walls", "checked": 8, "failed": 0,
                "not_evaluated": 0, "status": "passed"}]),
        "{result:#}"
    );
    assert_eq!(result["report"]["findings"], json!([]), "{result:#}");
}

/// The storey-metric definitions with `quantity-takeoff` as
/// `axioval:example.takeoff` and the concept `Name`.
fn takeoff_definitions(case: &Case) -> PathBuf {
    let text = |value: &str| json!({"default": value, "translations": {}});
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(storey_metric_definitions(case)).unwrap())
            .unwrap();
    definitions["properties"]["axioval:example.ifc.Name"] = json!({
        "id": "axioval:example.ifc.Name", "name": text("Name"), "valueKind": "string",
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "Name"}], "citations": []});
    let parameters = registry_signature("axioval:capability.quantity-takeoff");
    definitions["definitions"]["axioval:example.takeoff"] = json!({
        "id": "axioval:example.takeoff", "name": text("takeoff"), "description": text("takeoff"),
        "capability": "axioval:capability.quantity-takeoff", "parameters": parameters,
        "citations": [], "tags": []});
    case.write("definitions.json", &definitions.to_string())
}

/// One rule taking off walls by storey (the name of the storey containing
/// them) with their summed footprints.
fn takeoff_ruleset(case: &Case) -> PathBuf {
    let text_file = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text_file).unwrap();
    let mut rule = ruleset["root"]["rules"][0].clone();
    rule["id"] = json!("wall-takeoff");
    rule["definitionId"] = json!("axioval:example.takeoff");
    rule["applicability"]["groups"]["walls"]["selector"] = json!({
        "kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true});
    rule["parameters"] = json!({
        "group_1": {"type": "propertyReference", "propertySet": "axioval:attributes",
                    "property": "axioval:example.ifc.Name"},
        "group_1_path": {"type": "stringList",
                         "value": ["IfcRelContainedInSpatialStructure:backward"]},
        "group_1_name": {"type": "string", "value": "storey"},
        "measure_1": {"type": "propertyReference", "propertySet": "axioval:measured",
                      "property": "area"},
        "measure_1_name": {"type": "string", "value": "footprint"},
    });
    ruleset["root"]["rules"] = json!([rule]);
    case.write("ruleset.json", &ruleset.to_string())
}

/// Walls taken off per storey with their measured footprints: the grouped
/// table is saved, listed with its groups and exported as CSV.
#[test]
fn a_quantity_takeoff_is_listed_by_group_and_exported_as_csv() {
    let case = Case::new("quantity-takeoff");
    let definitions = takeoff_definitions(&case);

    let ruleset = takeoff_ruleset(&case);
    let model = case.write("model.ifc", &storeys_with_facades());
    let saved = case.path("result.json");
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("check")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(["--geometry", "--report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let table = &result["report"]["tables"][0];
    assert_eq!(table["name"], "takeoff", "{result:#}");
    assert_eq!(table["group_by"], json!(["storey"]), "{result:#}");
    let groups: Vec<&Value> = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| &row["group"])
        .collect();
    assert_eq!(groups, [&json!(["EG"]), &json!(["OG"])], "{result:#}");
    // One 10 m by 0.3 m wall per storey (the file's single-precision
    // placement rounds its area within a micrometre square or so).
    for row in table["rows"].as_array().unwrap() {
        assert_eq!(row["values"][0], json!({"type": "exact", "value": 1.0}));
        let footprint = &row["values"][1];
        let (lower, upper) = match footprint["type"].as_str().unwrap() {
            "exact" => (footprint["value"].as_f64(), footprint["value"].as_f64()),
            _ => (footprint["lower"].as_f64(), footprint["upper"].as_f64()),
        };
        assert!(
            lower.unwrap() <= 3.0 + 1e-6 && upper.unwrap() >= 3.0 - 1e-6,
            "{row}"
        );
    }

    let saved = saved.to_str().unwrap();
    let summary = stdout(&report(&[saved]));
    assert!(
        summary.contains("grouped by storey; columns: count, sum_footprint (m²)"),
        "{summary}"
    );
    let listing = stdout(&report(&[
        saved,
        "--section",
        "tables",
        "--rule",
        "wall-takeoff",
    ]));
    assert!(
        listing.contains("[EG] count 1 · sum_footprint 3"),
        "{listing}"
    );
    assert!(
        listing.contains("[OG] count 1 · sum_footprint 3"),
        "{listing}"
    );

    let csv = stdout(&report(&[
        saved,
        "--csv",
        "--rule",
        "wall-takeoff",
        "--table",
        "takeoff",
    ]));
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "scope,storey,count_lower,count_upper,sum_footprint_lower [m²],sum_footprint_upper [m²]",
        "{csv}"
    );
    assert_eq!(lines.len(), 3, "{csv}");
    assert!(
        lines[1].starts_with("source ") && lines[1].contains(",EG,1,1,"),
        "{csv}"
    );
    assert!(lines[2].contains(",OG,1,1,"), "{csv}");
    let missing = report(&[saved, "--csv", "--table", "levels"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stderr(&missing).contains("--rule wall-takeoff --table takeoff"),
        "{}",
        stderr(&missing)
    );
}

/// The text of `name` in the zip archive `bytes`.
fn zip_entry(bytes: &[u8], name: &str) -> String {
    use std::io::Read as _;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

/// Two takeoffs of the walls per storey, one with their footprints, written
/// as a workbook: a findings sheet and one sheet per table, whose counts
/// and areas are numeric cells.
#[test]
fn two_takeoff_tables_are_written_as_a_workbook_of_three_sheets() {
    let case = Case::new("takeoff-xlsx");
    let definitions = takeoff_definitions(&case);
    let ruleset = takeoff_ruleset(&case);
    let mut packages: Value =
        serde_json::from_str(&std::fs::read_to_string(&ruleset).unwrap()).unwrap();
    let mut count = packages["root"]["rules"][0].clone();
    count["id"] = json!("wall-count");
    for parameter in ["measure_1", "measure_1_name"] {
        count["parameters"]
            .as_object_mut()
            .unwrap()
            .remove(parameter);
    }
    packages["root"]["rules"]
        .as_array_mut()
        .unwrap()
        .push(count);
    let ruleset = case.write("ruleset.json", &packages.to_string());
    let model = case.write("model.ifc", &storeys_with_facades());
    let workbook = case.path("report.xlsx");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .arg("check")
            .arg("--model")
            .arg(&model)
            .arg("--definitions")
            .arg(&definitions)
            .arg("--ruleset")
            .arg(&ruleset)
            .args(["--geometry", "--report"])
            .arg(case.path("result.json"))
            .arg("--xlsx")
            .arg(&workbook)
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap()
    };
    let output = run();
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let bytes = std::fs::read(&workbook).unwrap();
    let names = zip_entry(&bytes, "xl/workbook.xml");
    assert_eq!(names.matches("<sheet ").count(), 3, "{names}");
    for name in ["Findings", "wall-count takeoff", "wall-takeoff takeoff"] {
        assert!(names.contains(&format!("name=\"{name}\"")), "{names}");
    }
    let strings = zip_entry(&bytes, "xl/sharedStrings.xml");
    assert!(strings.contains("sum_footprint lower [m²]"), "{strings}");
    // Row 3 is storey EG: scope, kind, GlobalId, storey, then the count's
    // bounds as numbers and the footprint's.
    let takeoff = zip_entry(&bytes, "xl/worksheets/sheet3.xml");
    assert!(takeoff.contains("<c r=\"E3\"><v>1</v></c>"), "{takeoff}");
    assert!(takeoff.contains("<c r=\"H3\"><v>"), "{takeoff}");
    assert!(takeoff.contains("<c r=\"I3\"><v>"), "{takeoff}");
    let count = zip_entry(&bytes, "xl/worksheets/sheet2.xml");
    assert!(count.contains("<c r=\"E3\"><v>1</v></c>"), "{count}");
    // Created at SOURCE_DATE_EPOCH: a second run writes the same bytes.
    assert_eq!(run().status.code(), Some(0));
    assert_eq!(std::fs::read(&workbook).unwrap(), bytes);
}

/// The `takeoff` table of the rule under test.
fn takeoff_table(result: &Value) -> &Value {
    result["report"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|table| table["rule_id"] == "under-test" && table["name"] == "takeoff")
        .unwrap_or_else(|| panic!("no takeoff table: {result:#}"))
}

/// Storeys `EG` and `OG`, wall `W1` and door `D1` in `EG`, door `D2` in
/// `OG`; only the wall states `Pset_WallCommon`.
fn walls_and_doors_on_storeys() -> String {
    "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
     #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
     #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
     #3=IFCLOCALPLACEMENT($,#2);\n\
     #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#2,$);\n\
     #6=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
     #7=IFCUNITASSIGNMENT((#6));\n\
     #8=IFCPROJECT('0000000000000000000008',$,'P',$,$,$,$,(#5),#7);\n\
     #10=IFCBUILDINGSTOREY('0000000000000000000010',$,'EG',$,$,#3,$,$,.ELEMENT.,0.);\n\
     #11=IFCBUILDINGSTOREY('0000000000000000000011',$,'OG',$,$,#3,$,$,.ELEMENT.,3.);\n\
     #20=IFCWALL('0000000000000000000020',$,'W1',$,$,#3,$,$,.STANDARD.);\n\
     #21=IFCDOOR('0000000000000000000021',$,'D1',$,$,#3,$,$,2.1,0.9,.DOOR.,.SINGLE_SWING.,$);\n\
     #22=IFCDOOR('0000000000000000000022',$,'D2',$,$,#3,$,$,2.1,0.9,.DOOR.,.SINGLE_SWING.,$);\n\
     #30=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000030',$,$,$,(#20,#21),#10);\n\
     #31=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000031',$,$,$,(#22),#11);\n\
     #40=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('F90'),$);\n\
     #41=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);\n\
     #42=IFCPROPERTYSET('0000000000000000000042',$,'Pset_WallCommon',$,(#40,#41));\n\
     #43=IFCRELDEFINESBYPROPERTIES('0000000000000000000043',$,$,$,(#20),#42);\n\
     ENDSEC;\nEND-ISO-10303-21;\n"
        .to_owned()
}

/// An itemised takeoff of walls and doors: each element's storey through
/// a `related` column, and the wall's `Pset_WallCommon` expanded into one
/// column per property.
#[test]
fn a_takeoff_lists_related_storeys_and_expands_a_property_set() {
    let case = Case::new("takeoff-related-property-set");
    let name = json!({"type": "propertyReference", "propertySet": "axioval:attributes",
                      "property": "axioval:example.ifc.name"});
    let (output, result) = case.geometry_rule(
        &walls_and_doors_on_storeys(),
        &[("door", "IfcDoor")],
        "axioval:capability.quantity-takeoff",
        &registry_signature("axioval:capability.quantity-takeoff"),
        json!({"kind": "anyOf", "operands": [entity("wall"), entity("door")]}),
        json!({
            "group_1": name,
            "group_1_name": {"type": "string", "value": "element"},
            "measure_1_kind": {"type": "string", "value": "related"},
            "measure_1": name,
            "measure_1_path": {"type": "stringList",
                               "value": ["IfcRelContainedInSpatialStructure:backward"]},
            "measure_1_name": {"type": "string", "value": "storey"},
            "measure_2_kind": {"type": "string", "value": "property_set"},
            "measure_2_property_set": {"type": "string",
                                       "value": "axioval:example.ifc.pset-wall-common"},
        }),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let table = takeoff_table(&result);
    assert_eq!(
        table["columns"],
        json!([
            {"id": "count", "kind": "number", "exactness": "exact"},
            {"id": "values_storey", "kind": "text", "exactness": "exact"},
            {"id": "values_fire_rating", "kind": "text", "exactness": "exact"},
            {"id": "values_is_external", "kind": "text", "exactness": "exact"},
        ]),
        "{result:#}"
    );
    let rows: Vec<(Value, Vec<Value>)> = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let values = row["values"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1)
                .map(|value| value["value"].clone())
                .collect();
            (row["group"].clone(), values)
        })
        .collect();
    assert_eq!(
        rows,
        [
            (json!(["D1"]), vec![json!("EG"), json!("-"), json!("-")]),
            (json!(["D2"]), vec![json!("OG"), json!("-"), json!("-")]),
            (
                json!(["W1"]),
                vec![json!("EG"), json!("F90"), json!("true")]
            ),
        ],
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// Spaces #39 and #69 take off the area of their boundaries against walls,
/// measured from the connection surfaces the model declares.
#[test]
fn with_geometry_a_space_takeoff_sums_its_wall_boundary_areas() {
    let case = Case::new("takeoff-boundary-areas");
    let (output, result) = case.geometry_rule(
        &spaces_with_boundaries(),
        &[("space", "IfcSpace")],
        "axioval:capability.quantity-takeoff",
        &registry_signature("axioval:capability.quantity-takeoff"),
        entity("space"),
        json!({
            "measure_1_kind": {"type": "string", "value": "boundary_area"},
            "measure_1_bounding": {"type": "selector", "value": entity("wall")},
            "measure_1_name": {"type": "string", "value": "wall_area"},
        }),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let table = takeoff_table(&result);
    assert_eq!(
        table["columns"][1],
        json!({"id": "sum_wall_area", "kind": "quantity", "dimension": "area",
               "exactness": "exact"}),
        "{result:#}"
    );
    // #39: the floor less its 1 m² hole, the ceiling and three walls
    // (11 + 12 + 10 + 10 + 7.5 m²); #69: a 6 m² triangle.
    assert_eq!(
        table["rows"][0]["values"],
        json!([{"type": "exact", "value": 2.0}, {"type": "exact", "value": 56.5}]),
        "{result:#}"
    );
    assert_eq!(result["report"]["not_evaluated"], json!([]), "{result:#}");
}

/// The wall lining of spaces #39 and #69 priced per square metre: a
/// computed column over the boundary area, reported in its currency and
/// exported with it.
#[test]
fn with_geometry_a_computed_column_prices_the_wall_lining() {
    let case = Case::new("takeoff-computed");
    let (output, result) = case.geometry_rule(
        &spaces_with_boundaries(),
        &[("space", "IfcSpace")],
        "axioval:capability.quantity-takeoff",
        &registry_signature("axioval:capability.quantity-takeoff"),
        entity("space"),
        json!({
            "measure_1_kind": {"type": "string", "value": "boundary_area"},
            "measure_1_bounding": {"type": "selector", "value": entity("wall")},
            "measure_1_name": {"type": "string", "value": "wall_area"},
            "measure_2_kind": {"type": "string", "value": "computed"},
            "measure_2_expression": {"type": "string", "value": "wall_area × 30 EUR/m²"},
            "measure_2_name": {"type": "string", "value": "lining"},
        }),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let table = takeoff_table(&result);
    assert_eq!(
        table["columns"][2],
        json!({"id": "sum_lining", "kind": "number", "unit": "EUR", "exactness": "exact"}),
        "{result:#}"
    );
    // 50.5 m² of #39 and 6 m² of #69 at 30 EUR/m².
    assert_eq!(
        table["rows"][0]["values"][2],
        json!({"type": "exact", "value": 1695.0}),
        "{result:#}"
    );
    let saved = case.path("result.json");
    let saved = saved.to_str().unwrap();
    let csv = stdout(&report(&[
        saved,
        "--csv",
        "--rule",
        "under-test",
        "--table",
        "takeoff",
    ]));
    assert!(
        csv.lines()
            .next()
            .unwrap()
            .ends_with("sum_lining_lower [EUR],sum_lining_upper [EUR]"),
        "{csv}"
    );
    let summary = stdout(&report(&[saved]));
    assert!(summary.contains("sum_lining (EUR)"), "{summary}");
}

#[test]
fn a_decision_assigns_a_finding_with_due_date_priority_and_labels() {
    let case = Case::new("decisions-review");
    let axioval = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(case.path("."))
            .args(args)
            .output()
            .unwrap()
    };
    let read = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(case.path(name)).unwrap()).unwrap()
    };
    let r1 = case.path("r1.json");
    let output = case.check(&revision(1, &[]), true, &["--report", r1.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let id = read("r1.json")["report"]["findings"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let decide = |extra: &[&str]| {
        let mut args = vec![
            "decide",
            "r1.json",
            "--decisions",
            "decisions.json",
            "--finding",
            &id,
            "--author",
            "A. Reviewer",
            "--date",
            "2026-09-27T08:00:00Z",
        ];
        args.extend_from_slice(extra);
        axioval(&args)
    };
    let output = decide(&[
        "--status",
        "open",
        "--assign-to",
        "C. Engineer",
        "--due",
        "2026-10-15T17:00:00+02:00",
        "--priority",
        "Critical",
        "--label",
        "site visit",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    // A later decision that does not restate them keeps them.
    let output = decide(&["--status", "accepted", "--comment", "fixed on site"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let decision = read("decisions.json")["decisions"][0].clone();
    assert_eq!(decision["status"], "accepted");
    assert_eq!(decision["assigned_to"], "C. Engineer");
    assert_eq!(decision["due_date"], "2026-10-15T17:00:00+02:00");
    assert_eq!(decision["priority"], "Critical");
    assert_eq!(decision["labels"], json!(["site visit"]));

    let bcf = case.path("r2.bcfzip");
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            case.path("r2.json").to_str().unwrap(),
            "--decisions",
            case.path("decisions.json").to_str().unwrap(),
            "--bcf",
            bcf.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let carried = read("r2.json")["report"]["findings"][0]["decision"].clone();
    assert_eq!(carried["assigned_to"], "C. Engineer");
    assert_eq!(carried["priority"], "Critical");
    // Priority, labels, assignee and due date are in the topic.
    let archive = openbim_bcf::read_path(&bcf).unwrap();
    let topic = &archive
        .topics()
        .find(|markup| markup.topic.guid.as_deref() == Some(id.as_str()))
        .unwrap()
        .topic;
    assert_eq!(topic.priority.as_deref(), Some("Critical"));
    assert!(topic.labels.contains(&"site visit".to_owned()), "{topic:?}");
    assert_eq!(topic.assigned_to.as_deref(), Some("C. Engineer"));
    assert_eq!(topic.due_date.as_deref(), Some("2026-10-15T17:00:00+02:00"));
    let listing =
        String::from_utf8(axioval(&["report", "r2.json", "--decision", "accepted"]).stdout)
            .unwrap();
    assert!(
        listing.contains(
            "fixed on site; assigned to C. Engineer; due 2026-10-15T17:00:00+02:00; priority Critical; labels site visit"
        ),
        "{listing}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn comments_by_several_reviewers_form_a_thread_through_bcf() {
    let case = Case::new("decisions-thread");
    let axioval = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(case.path("."))
            .args(args)
            .output()
            .unwrap()
    };
    let read = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(case.path(name)).unwrap()).unwrap()
    };
    let r1 = case.path("r1.json");
    let output = case.check(&revision(1, &[]), true, &["--report", r1.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let id = read("r1.json")["report"]["findings"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (author, status, comment, date) in [
        (
            "A. Reviewer",
            "open",
            "is this a lining?",
            "2026-09-27T08:00:00Z",
        ),
        (
            "B. Architect",
            "open",
            "yes, a lining",
            "2026-09-28T08:00:00Z",
        ),
        (
            "A. Reviewer",
            "rejected",
            "then no reference is needed",
            "2026-09-29T08:00:00Z",
        ),
    ] {
        let output = axioval(&[
            "decide",
            "r1.json",
            "--decisions",
            "decisions.json",
            "--finding",
            &id,
            "--status",
            status,
            "--author",
            author,
            "--comment",
            comment,
            "--date",
            date,
        ]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    }
    let thread = read("decisions.json")["decisions"][0]["comments"].clone();
    assert_eq!(thread.as_array().unwrap().len(), 3, "{thread:#}");
    assert_eq!(thread[1]["author"], "B. Architect");

    let bcf = case.path("r2.bcfzip");
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            case.path("r2.json").to_str().unwrap(),
            "--decisions",
            case.path("decisions.json").to_str().unwrap(),
            "--bcf",
            bcf.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let archive = openbim_bcf::read_path(&bcf).unwrap();
    let markup = archive
        .topics()
        .find(|markup| markup.topic.guid.as_deref() == Some(id.as_str()))
        .unwrap();
    let comments: Vec<_> = markup
        .comments
        .iter()
        .map(|c| c.comment.as_deref().unwrap())
        .collect();
    assert_eq!(
        comments,
        [
            "Rejected",
            "is this a lining?",
            "yes, a lining",
            "then no reference is needed"
        ]
    );
    let listing =
        String::from_utf8(axioval(&["report", "r2.json", "--decision", "rejected"]).stdout)
            .unwrap();
    assert!(
        listing.contains(
            "rejected by A. Reviewer on 2026-09-29T08:00:00Z: then no reference is needed (+2 earlier comment(s))"
        ),
        "{listing}"
    );

    // Read back from the archive, the thread is the same.
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            case.path("r3.json").to_str().unwrap(),
            "--decisions-from",
            bcf.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let carried = read("r3.json")["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["id"] == id.as_str())
        .unwrap()["decision"]
        .clone();
    assert_eq!(carried["status"], "rejected");
    assert_eq!(carried["comments"], thread);
}

/// The BCF API test server of `axioval-bcf-api`: in process, never a real
/// server.
#[path = "../../../sinks/bcf-api/tests/support/mock.rs"]
mod mock;

#[test]
#[allow(clippy::too_many_lines)]
fn findings_are_pushed_to_a_bcf_server_and_its_review_is_pulled_back() {
    let case = Case::new("bcf-server");
    let server = mock::Mock::start();
    let axioval = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(case.path("."))
            .args(args)
            .env("AXIOVAL_BCF_CLIENT_SECRET", mock::CLIENT_SECRET)
            .env_remove("AXIOVAL_BCF_TOKEN")
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap()
    };
    let text = |output: &Output| String::from_utf8_lossy(&output.stdout).into_owned();
    let r1 = case.path("r1.json");
    let output = case.check(&revision(1, &[]), true, &["--report", r1.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&r1).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    let topics = findings.len() + result["report"]["not_evaluated"].as_array().unwrap().len();
    let kept = findings[0]["id"].as_str().unwrap().to_owned();
    let server_args = [
        "--server",
        server.url.as_str(),
        "--project",
        mock::PROJECT,
        "--client-id",
        mock::CLIENT_ID,
    ];
    let bcf = |command: &str, extra: &[&str]| {
        let mut args = vec!["bcf", command, "r1.json"];
        args.extend_from_slice(&server_args);
        args.extend_from_slice(extra);
        axioval(&args)
    };

    // Pushing the run creates its topics.
    let output = bcf("push", &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        text(&output).contains(&format!("{topics} topic(s) created, 0 updated")),
        "{}",
        text(&output)
    );
    assert_eq!(server.topic_count(), topics);
    assert_eq!(server.topic(&kept)["topic_status"], "Open");

    // A reviewer closes one on the server; pulling records it as accepted.
    server.set_status(&kept, "Closed");
    let output = bcf("pull", &["--decisions", "decisions.json"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        text(&output).contains("pulled 1 decision(s)"),
        "{}",
        text(&output)
    );
    let saved = std::fs::read_to_string(case.path("decisions.json")).unwrap();
    // No credential ever reaches a file.
    assert!(!saved.contains(mock::CLIENT_SECRET) && !saved.contains(mock::TOKEN));
    let decisions: Value = serde_json::from_str(&saved).unwrap();
    let decision = &decisions["decisions"][0];
    assert_eq!(decision["finding"], kept.as_str());
    assert_eq!(decision["status"], "accepted");
    assert_eq!(decision["author"], "C. Reviewer");
    assert!(decision["basis"].is_object(), "{decision:#}");

    // A second push updates the topics instead of duplicating them, and
    // keeps the status the reviewer set.
    let output = bcf("push", &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        text(&output).contains(&format!("0 topic(s) created, {topics} updated")),
        "{}",
        text(&output)
    );
    assert_eq!(server.topic_count(), topics);
    assert_eq!(server.topic(&kept)["topic_status"], "Closed");

    // The next check carries the pulled decision over.
    let output = case.check(
        &revision(1, &[]),
        true,
        &[
            "--report",
            case.path("r2.json").to_str().unwrap(),
            "--decisions",
            case.path("decisions.json").to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let second: Value =
        serde_json::from_str(&std::fs::read_to_string(case.path("r2.json")).unwrap()).unwrap();
    let carried = second["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["id"] == kept.as_str())
        .unwrap()["decision"]
        .clone();
    assert_eq!(carried["status"], "accepted");
    assert_eq!(carried["evidence"], "unchanged");

    // A client id without a secret or device flow cannot sign in.
    let output = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .current_dir(case.path("."))
        .args(["bcf", "push", "r1.json"])
        .args(server_args)
        .env_remove("AXIOVAL_BCF_CLIENT_SECRET")
        .env_remove("AXIOVAL_BCF_TOKEN")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("AXIOVAL_BCF_CLIENT_SECRET"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn bcf_snapshots_show_a_clash_and_are_marked_illustrative() {
    let case = Case::new("bcf-snapshots");
    let with = case.path("with.bcfzip");
    let output = case.clash_check(&[
        "--geometry",
        "--bcf",
        with.to_str().unwrap(),
        "--bcf-date",
        "2026-09-30T10:00:00Z",
        "--bcf-snapshots",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let archive = openbim_bcf::read_path(&with).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );
    let markup = archive.topics().next().unwrap();
    assert!(
        markup
            .topic
            .description
            .as_deref()
            .unwrap()
            .ends_with("Snapshot: illustrative rendering of tessellated bodies, not evidence."),
        "{:?}",
        markup.topic.description
    );
    assert_eq!(markup.viewpoints.len(), 2);
    for viewpoint in &markup.viewpoints {
        let name = viewpoint.snapshot.as_deref().unwrap();
        assert!(
            archive.entries().iter().any(|entry| entry.ends_with(name)),
            "{name} in {:?}",
            archive.entries()
        );
    }

    // Without the flag the archive has no snapshot, byte for byte as before.
    let without = case.path("without.bcfzip");
    let output = case.clash_check(&[
        "--geometry",
        "--bcf",
        without.to_str().unwrap(),
        "--bcf-date",
        "2026-09-30T10:00:00Z",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let plain = openbim_bcf::read_path(&without).unwrap();
    assert!(
        plain
            .topics()
            .all(|markup| markup.viewpoints.iter().all(|v| v.snapshot.is_none()))
    );
    let again = case.path("again.bcfzip");
    case.clash_check(&[
        "--geometry",
        "--bcf",
        again.to_str().unwrap(),
        "--bcf-date",
        "2026-09-30T10:00:00Z",
    ]);
    assert_eq!(
        std::fs::read(&without).unwrap(),
        std::fs::read(&again).unwrap()
    );

    // Snapshots need meshes.
    let output = case.clash_check(&[
        "--bcf",
        case.path("none.bcfzip").to_str().unwrap(),
        "--bcf-snapshots",
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("--geometry"),
        "{}",
        stderr(&output)
    );
}

/// A round column #19 (radius 0.2 m, axis at x 5, y 3, from z 0.5 to 3.5,
/// placed turned a quarter about z) and a wall #29 whose face stands at
/// y 4: 0.8 m from the column's surface.
fn round_column_beside_a_wall() -> String {
    model_with(&format!(
        "#10=IFCCARTESIANPOINT((5.,3.,0.5));\n\
         #11=IFCDIRECTION((0.,1.,0.));\n\
         #12=IFCAXIS2PLACEMENT3D(#10,#4,#11);\n\
         #13=IFCLOCALPLACEMENT($,#12);\n\
         #14=IFCCIRCLEPROFILEDEF(.AREA.,$,$,0.2);\n\
         #15=IFCEXTRUDEDAREASOLID(#14,#2,#4,3.);\n\
         #16=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#15));\n\
         #17=IFCPRODUCTDEFINITIONSHAPE($,$,(#16));\n\
         #19=IFCCOLUMN('0000000000000000000019',$,$,$,$,#13,#17,$,$);\n\
         {}",
        placed_box(
            20,
            [5.0, 4.1, 0.0],
            [4.0, 0.2, 4.0],
            "IFCWALL('GID',$,$,$,$,PL,REP,$,$)"
        ),
    ))
}

/// The column's distance to the wall against a 0.79999 m minimum, with
/// further `check` arguments.
///
/// The column's mesh faces the wall with a chord about 0.96 mm inside the
/// circle, and the bridge declares 1 mm, so the mesh alone measures between
/// about 0.79996 m and 0.80196 m: a minimum just under the true 0.8 m is
/// what only the certified distance can decide.
fn round_column_distance(case: &Case, args: &[&str]) -> (Output, Value) {
    case.write("model.ifc", &round_column_beside_a_wall());
    case.geometry_rule_with(
        &["model.ifc"],
        &[("column", "IfcColumn"), ("wall", "IfcWall")],
        (
            "axioval:capability.distance",
            &registry_signature("axioval:capability.distance"),
        ),
        entity("column"),
        json!({
            "counterparts": {"type": "selector", "value": entity("wall")},
            "mode": {"type": "string", "value": "none_closer_than"},
            "minimum_metres": {"type": "number", "value": 0.79999},
        }),
        &json!({}),
        args,
    )
}

#[test]
fn with_geometry_a_round_columns_distance_to_a_wall_is_certified() {
    // The column's mesh is a tessellation within 1 mm, so on its own it
    // leaves the minimum open.
    let case = Case::new("geometry-certified-column");
    let (output, result) = round_column_distance(&case, &["--no-exact-boundaries"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(result["geometry"]["tessellated"], 1, "{result:#}");
    assert_eq!(
        result["geometry"].get("exact_boundaries"),
        None,
        "{result:#}"
    );
    assert_eq!(
        result["report"]["not_evaluated"][0]["object_id"]["local_id"],
        json!("#19"),
        "{result:#}"
    );

    // By default both bodies also get their exact boundaries, built from
    // the same extrusions, and the certified distance clears the minimum.
    let (output, result) = round_column_distance(&case, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(result["geometry"]["exact_boundaries"], 2, "{result:#}");
    assert!(finding_ids(&result).is_empty(), "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
    assert!(
        stderr(&output).contains("2 with an exact boundary"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn exact_boundaries_are_a_geometry_option() {
    let case = Case::new("exact-boundaries-need-geometry");
    let output = case.check(&model_with(""), true, &["--no-exact-boundaries"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

/// A round member #19 (radius 0.2 m, 3 m long) tilted out of the
/// vertical: its axis runs from (5, 3, 2) along (0, 0.6, 0.8), so its
/// side stays 0.2 m from x 5 all along. A wall #29 stands with its face
/// at x 6, 0.8 m from the member's surface.
fn tilted_member_beside_a_wall() -> String {
    model_with(&format!(
        "#10=IFCCARTESIANPOINT((5.,3.,2.));\n\
         #11=IFCDIRECTION((1.,0.,0.));\n\
         #12=IFCAXIS2PLACEMENT3D(#10,#18,#11);\n\
         #13=IFCLOCALPLACEMENT($,#12);\n\
         #14=IFCCIRCLEPROFILEDEF(.AREA.,$,$,0.2);\n\
         #15=IFCEXTRUDEDAREASOLID(#14,#2,#4,3.);\n\
         #16=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#15));\n\
         #17=IFCPRODUCTDEFINITIONSHAPE($,$,(#16));\n\
         #18=IFCDIRECTION((0.,0.6,0.8));\n\
         #19=IFCMEMBER('0000000000000000000019',$,$,$,$,#13,#17,$,$);\n\
         {}",
        placed_box(
            20,
            [6.1, 4.0, 0.0],
            [0.2, 6.0, 5.0],
            "IFCWALL('GID',$,$,$,$,PL,REP,$,$)"
        ),
    ))
}

#[test]
fn with_geometry_a_tilted_round_members_distance_is_certified() {
    let case = Case::new("geometry-certified-tilted-member");
    case.write("model.ifc", &tilted_member_beside_a_wall());
    let distance = |args: &[&str]| {
        case.geometry_rule_with(
            &["model.ifc"],
            &[("member", "IfcMember"), ("wall", "IfcWall")],
            (
                "axioval:capability.distance",
                &registry_signature("axioval:capability.distance"),
            ),
            entity("member"),
            json!({
                "counterparts": {"type": "selector", "value": entity("wall")},
                "mode": {"type": "string", "value": "none_closer_than"},
                "minimum_metres": {"type": "number", "value": 0.79999},
            }),
            &json!({}),
            args,
        )
    };

    // Its mesh alone, within 1 mm of the cylinder, leaves the minimum open.
    let (output, result) = distance(&["--no-exact-boundaries"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(result["geometry"]["tessellated"], 1, "{result:#}");
    assert_eq!(
        result["report"]["not_evaluated"][0]["object_id"]["local_id"],
        json!("#19"),
        "{result:#}"
    );

    // The tilted placement is applied to the exact cylinder as it is to
    // the mesh, and the certified distance clears the minimum.
    let (output, result) = distance(&[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(result["geometry"]["exact_boundaries"], 2, "{result:#}");
    assert!(finding_ids(&result).is_empty(), "{result:#}");
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{result:#}"
    );
}

/// A ring 5 km round needs more than the kernel's 4096 steps a turn to stay
/// within the 1 mm chord tolerance (axiolid/kernel#231): the kernel refuses
/// the budget rather than mesh it coarser, so the wall is unmeasured with
/// the reason and the clash between the other walls is still found.
#[test]
fn a_body_beyond_the_kernels_step_budget_is_unmeasured() {
    let case = Case::new("step-budget");
    let ring = "#40=IFCCARTESIANPOINT((5000.,0.));\n\
                #41=IFCAXIS2PLACEMENT2D(#40,$);\n\
                #42=IFCRECTANGLEPROFILEDEF(.AREA.,$,#41,0.2,0.2);\n\
                #43=IFCDIRECTION((0.,1.,0.));\n\
                #44=IFCAXIS1PLACEMENT(#1,#43);\n\
                #45=IFCREVOLVEDAREASOLID(#42,#2,#44,6.283185307179586);\n\
                #46=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#45));\n\
                #47=IFCPRODUCTDEFINITIONSHAPE($,$,(#46));\n\
                #48=IFCWALL('0000000000000000000048',$,$,$,$,#3,#47,$,$);\n";
    let (output, result) = case.wall_clash(&crossing_walls_with(ring), &json!({}));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    let unmeasured = result["geometry"]["unmeasured"].as_array().unwrap();
    let ring = unmeasured
        .iter()
        .find(|entry| entry["object"]["local_id"] == "#48")
        .unwrap_or_else(|| panic!("{result:#}"));
    let reason = ring["reason"].as_str().unwrap();
    assert!(
        reason.contains("within the 0.001 m chord tolerance")
            && reason.contains("revolution angular steps")
            && reason.contains("a coarser mesh would break that tolerance"),
        "{reason}"
    );
    // Its clashes are neither found nor passed.
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|outcome| outcome.to_string().contains("#48")),
        "{result:#}"
    );
}
