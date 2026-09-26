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
        let parameter = |id: &str, kind: &str, required: bool| {
            json!({"id": id, "name": {"default": id, "translations": {}}, "kind": kind,
                   "required": required, "allowedValues": [], "citations": []})
        };
        definitions["definitions"]["axioval:example.clash"] = json!({
            "id": "axioval:example.clash",
            "name": {"default": "Clash", "translations": {}},
            "description": {"default": "Bodies must not interpenetrate.", "translations": {}},
            "capability": "axioval:capability.clash",
            "parameters": {
                "counterparts": parameter("counterparts", "selector", true),
                "penetration_tolerance_metres":
                    parameter("penetration_tolerance_metres", "number", true),
                "clearance_metres": parameter("clearance_metres", "number", false),
            },
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
    fn envelope_check(&self, extra: &[&str]) -> Output {
        let definitions = self.definitions(true);
        let mut definitions: Value =
            serde_json::from_str(&std::fs::read_to_string(definitions).unwrap()).unwrap();
        definitions["definitions"]["axioval:example.envelope"] = json!({
            "id": "axioval:example.envelope",
            "name": {"default": "External walls", "translations": {}},
            "description": {"default": "Declared external walls form the envelope.",
                            "translations": {}},
            "capability": "axioval:capability.external-wall-validation",
            "parameters": {"envelope_derivation": {
                "id": "envelope_derivation",
                "name": {"default": "envelope_derivation", "translations": {}},
                "kind": "string", "required": true, "allowedValues": [], "citations": []}},
            "citations": [],
            "tags": [],
        });
        let text = std::fs::read_to_string(format!("{FIXTURES}/ruleset.json")).unwrap();
        let mut ruleset: Value = serde_json::from_str(&text).unwrap();
        let rule = &mut ruleset["root"]["rules"][0];
        rule["id"] = json!("external-walls");
        rule["definitionId"] = json!("axioval:example.envelope");
        rule["parameters"] = json!({
            "envelope_derivation": {"type": "string", "value": "gross-area-groups"},
        });
        let model = self.write("model.ifc", &envelope_model());
        let definitions = self.write("definitions.json", &definitions.to_string());
        let ruleset = self.write("ruleset.json", &ruleset.to_string());
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
fn with_an_envelope_zone_declared_external_walls_are_checked_against_it() {
    let case = Case::new("geometry-envelope");
    let saved = case.path("result.json");
    let output = case.envelope_check(&[
        "--geometry",
        "--envelope-zone",
        "Envelope",
        "--report",
        saved.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
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
    assert_eq!(
        findings,
        [
            (
                "#26".to_owned(),
                "on the gross-area-groups envelope but not declared external".to_owned()
            ),
            (
                "#36".to_owned(),
                "declared external but not on the gross-area-groups envelope".to_owned()
            ),
        ],
        "{result:#}"
    );
    let not_evaluated = result["report"]["not_evaluated"].as_array().unwrap();
    assert_eq!(not_evaluated.len(), 1, "{result:#}");
    assert_eq!(not_evaluated[0]["object_id"]["local_id"], "#56");
}

#[test]
fn without_an_envelope_zone_envelope_rules_are_not_evaluated() {
    let case = Case::new("geometry-envelope-off");
    let output = case.envelope_check(&["--geometry", "--summary"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("missing-service"),
        "{}",
        stdout(&output)
    );
    let unknown = case.envelope_check(&["--geometry", "--envelope-zone", "Nowhere"]);
    assert_eq!(unknown.status.code(), Some(1), "{}", stderr(&unknown));
    assert!(
        stderr(&unknown).contains("`Nowhere`"),
        "{}",
        stderr(&unknown)
    );
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
