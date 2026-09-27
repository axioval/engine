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

/// Two 4 m × 4 m × 3 m rooms. Room A (#40) is bounded by door #50 directly
/// and by opening #51, which door #52 fills. Room B (#41) states no space
/// boundary at all.
fn rooms_with_doors() -> String {
    let room = |first: u32, x: f64, global: &str| {
        let [p, pos, profile, solid, shape, product, space] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        format!(
            "#{p}=IFCCARTESIANPOINT(({x},0.));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},4.,4.);\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,3.);\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{product}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{space}=IFCSPACE('{global}',$,$,$,$,#3,#{product},$,.ELEMENT.,$,$);\n"
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
         #50=IFCDOOR('0000000000000000000050',$,$,$,$,#3,$,$,2.,0.9,$,$,$);\n\
         #51=IFCOPENINGELEMENT('0000000000000000000051',$,$,$,$,#3,$,$,.OPENING.);\n\
         #52=IFCDOOR('0000000000000000000052',$,$,$,$,#3,$,$,2.,0.9,$,$,$);\n\
         #53=IFCRELFILLSELEMENT('0000000000000000000053',$,$,$,#51,#52);\n\
         #54=IFCRELSPACEBOUNDARY('0000000000000000000054',$,$,$,#16,#50,$,.PHYSICAL.,.INTERNAL.);\n\
         #55=IFCRELSPACEBOUNDARY('0000000000000000000055',$,$,$,#16,#51,$,.PHYSICAL.,.INTERNAL.);\n\
         ENDSEC;\nEND-ISO-10303-21;\n",
        room(10, 2.0, "0000000000000000000016"),
        room(20, 8.0, "0000000000000000000026"),
    )
}

#[test]
fn with_geometry_shelf_capacity_counts_doorways_from_space_boundaries() {
    let case = Case::new("geometry-doorways");
    let definitions = case.definitions(true);
    let mut definitions: Value =
        serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
    definitions["objectTypes"]["axioval:example.ifc.space"] = json!({
        "id": "axioval:example.ifc.space",
        "name": {"default": "Space", "translations": {}},
        "externalNames": [{"typeSystem": IFC4_TYPE_SYSTEM, "name": "IfcSpace"}],
        "citations": [],
    });
    let names = [
        "minimum_running_metres",
        "shelf_depth_metres",
        "horizontal_spacing_metres",
        "vertical_spacing_metres",
        "bottom_elevation_metres",
        "top_elevation_metres",
        "door_clearance_metres",
    ];
    let parameters: serde_json::Map<String, Value> = names
        .iter()
        .map(|id| {
            (
                (*id).to_owned(),
                json!({"id": id, "name": {"default": id, "translations": {}},
                       "kind": "number", "required": true, "allowedValues": [],
                       "citations": []}),
            )
        })
        .collect();
    definitions["definitions"]["axioval:example.shelf"] = json!({
        "id": "axioval:example.shelf",
        "name": {"default": "Shelf capacity", "translations": {}},
        "description": {"default": "Rooms hold enough shelving.", "translations": {}},
        "capability": "axioval:capability.shelf-capacity",
        "parameters": parameters,
        "citations": [],
        "tags": [],
    });
    let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
    let mut ruleset: Value = serde_json::from_str(&text).unwrap();
    let rule = &mut ruleset["root"]["rules"][0];
    rule["id"] = json!("rooms-hold-shelving");
    rule["definitionId"] = json!("axioval:example.shelf");
    // 16 m of perimeter in five 0.4 m tiers holds 80 m. Each doorway takes
    // 0.9 m of wall, so exactly two doorways leave 14 whole pitches (70 m);
    // none or one would leave at least 75 m, which is not below the minimum.
    let values = [75.0, 0.3, 1.0, 0.4, 0.0, 2.0, 0.9];
    rule["parameters"] = names
        .iter()
        .zip(values)
        .map(|(id, value)| ((*id).to_owned(), json!({"type": "number", "value": value})))
        .collect::<serde_json::Map<_, _>>()
        .into();
    rule["applicability"]["groups"]["walls"]["selector"]["objectType"] =
        json!("axioval:example.ifc.space");
    let model = case.write("model.ifc", &rooms_with_doors());
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
    // A finding decides the exit status even though room B stays unmeasured.
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let findings = result["report"]["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{result:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#16", "{result:#}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(
        message.contains("70.000 below required 75.000"),
        "{message}"
    );
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 1, "{result:#}");
    assert_eq!(
        not_evaluated[0]["object_id"]["local_id"], "#26",
        "{result:#}"
    );
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
    /// `SprinklerProtection` and `TotalThickness` are always bound.
    fn geometry_rule(
        &self,
        model: &str,
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
        let model = self.write("model.ifc", model);
        let definitions = self.write("definitions.json", &definitions.to_string());
        let ruleset = self.write("ruleset.json", &ruleset.to_string());
        let saved = self.path("result.json");
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
}

/// A file of 3 m-high rectangular walls: `(first id, centre x, centre y,
/// x extent, y extent, GlobalId)`. Each wall's product is `first + 6`.
fn walls_file(walls: &[(u32, f64, f64, f64, f64, &str)]) -> String {
    let mut data = String::new();
    for &(first, x, y, length, width, global) in walls {
        let [p, pos, profile, solid, shape, product, wall] =
            [0, 1, 2, 3, 4, 5, 6].map(|offset| first + offset);
        let _ = write!(
            data,
            "#{p}=IFCCARTESIANPOINT(({x},{y}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length},{width});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,3.);\n\
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
