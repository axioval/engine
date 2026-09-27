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
    assert!(stderr(&output).contains("1 finding(s), 0 not evaluated"));
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
    assert_eq!(topic.labels, ["spaces-exist"]);
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
    let model = case.write("model.ifc", &rooms_without_containment());
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
    /// `SprinklerProtection`, `TotalThickness` and `Access.ClearWidth` are
    /// always bound.
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
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

impl Case {
    /// Architectural walls (subjects) against structural bodies
    /// (counterparts), in two files of one check.
    fn discipline_clash(&self, models: &[&str], extra: &[&str]) -> Output {
        // Two crossing architectural walls: they clash with each other, but
        // the rule only compares architecture with structure.
        self.write(
            "arch.ifc",
            &walls_file(&[
                (10, 2.0, 0.0, 4.0, 0.2, "0000000000000000000A16"),
                (20, 2.0, 0.0, 0.2, 4.0, "0000000000000000000A26"),
            ]),
        );
        // A structural wall through the first architectural wall at x = 0.5.
        self.write(
            "struct.ifc",
            &walls_file(&[(10, 0.5, 0.0, 0.2, 4.0, "0000000000000000000S16")]),
        );
        let (definitions, ruleset) = self.clash_packages();
        let mut ruleset: Value =
            serde_json::from_str(&std::fs::read_to_string(&ruleset).unwrap()).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["applicability"]["groups"]["walls"]["selector"] = json!({"kind": "allOf", "operands": [
            {"kind": "entityType", "objectType": "axioval:example.ifc.wall", "includeSubtypes": true},
            {"kind": "discipline", "value": "architecture"},
        ]});
        rule["parameters"]["counterparts"]["value"] =
            json!({"kind": "discipline", "value": "structure"});
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
         ENDSEC;\nEND-ISO-10303-21;\n",
        105
    )
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
