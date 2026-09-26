//! `axioval check` end to end: the real binary over real packages and IFC.
//!
//! Exit status and output shape are automation contracts, so every case
//! asserts the status first.
#![allow(missing_docs)]

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
