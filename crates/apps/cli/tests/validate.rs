//! `axioval validate` with drafts: diagnostics, a dry run on a model and
//! the JSON-lines process, end to end through the real binary.
#![allow(missing_docs)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../fixtures/schema-v0.1.0"
);

/// Walls #1 and #2; only #1 states `Reference`.
const IFC: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
#2=IFCWALL('0000000000000000000002',$,$,$,$,$,$,$,$);\n\
#4=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W-1'),$);\n\
#5=IFCPROPERTYSET('0000000000000000000005',$,'Pset_WallCommon',$,(#4));\n\
#6=IFCRELDEFINESBYPROPERTIES('0000000000000000000006',$,$,$,(#1),#5);\n\
ENDSEC;\nEND-ISO-10303-21;\n";

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("axioval-validate-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The fixture ruleset's first rule, as a draft.
fn fixture_rule() -> Value {
    let ruleset: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(FIXTURES).join("ruleset.json")).unwrap(),
    )
    .unwrap();
    let mut folders = vec![&ruleset["root"]];
    while let Some(folder) = folders.pop() {
        if let Some(rule) = folder["rules"].as_array().and_then(|rules| rules.first()) {
            return rule.clone();
        }
        folders.extend(folder["folders"].as_array().into_iter().flatten());
    }
    panic!("the fixture ruleset holds a rule");
}

fn validate(extra: &[&str], stdin: Option<&str>) -> Output {
    let definitions = Path::new(FIXTURES).join("definitions.json");
    let ruleset = Path::new(FIXTURES).join("ruleset.json");
    let mut child = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .arg("validate")
        .arg("--definitions")
        .arg(definitions)
        .arg("--ruleset")
        .arg(ruleset)
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

#[test]
fn a_refused_draft_prints_positioned_diagnostics() {
    let mut draft = fixture_rule();
    let definition = draft["definitionId"].as_str().unwrap().to_owned();
    draft["definitionId"] = json!(format!("{definition}x"));
    let output = validate(&["--rule", "-"], Some(&draft.to_string()));
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["valid"], false);
    let diagnostic = &answer["diagnostics"][0];
    assert_eq!(diagnostic["code"], "unknownDefinition");
    assert_eq!(diagnostic["pointer"], "/definitionId");
    assert_eq!(
        diagnostic["suggestion"],
        format!("did you mean `{definition}`?")
    );
}

#[test]
fn a_valid_draft_dry_runs_on_a_model() {
    let dir = directory("dry-run");
    let model = dir.join("walls.ifc");
    std::fs::write(&model, IFC).unwrap();
    let draft = dir.join("draft.json");
    std::fs::write(&draft, fixture_rule().to_string()).unwrap();
    let output = validate(
        &[
            "--rule",
            draft.to_str().unwrap(),
            "--model",
            model.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["valid"], true);
    let run = &answer["dryRun"];
    // The fixture's concepts name no IFC4 external name, so the run leaves
    // the walls not evaluated: either way it reports the drafted rule.
    let outcomes: Vec<&Value> = run["findings"]
        .as_array()
        .unwrap()
        .iter()
        .chain(run["notEvaluated"].as_array().unwrap())
        .collect();
    assert!(!outcomes.is_empty(), "{run}");
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome["rule_id"] == fixture_rule()["id"])
    );
    // A rule that is no expression traces nothing; its findings stand.
    assert_eq!(run["traces"], json!([]));
}

#[test]
fn drafts_are_served_as_json_lines() {
    let rule = fixture_rule();
    let input = format!(
        "{}\n\n{}\nnot json\n",
        json!({"rule": rule}),
        json!({"expression": {"kind": "null"}, "into": "missing"})
    );
    let output = validate(&["--serve"], Some(&input));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["valid"], true);
    assert_eq!(lines[1]["diagnostics"][0]["code"], "unknownRule");
    assert!(lines[2]["error"].as_str().unwrap().contains("not JSON"));
}
