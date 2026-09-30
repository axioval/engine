//! `axioval check --ids` and `axioval ids translate` end to end: the real
//! binary over IDS documents and IFC models written here.
#![allow(missing_docs)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const HEADER: &str = r#"<ids xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://standards.buildingsmart.org/IDS http://standards.buildingsmart.org/IDS/1.0/ids.xsd"><info><title>Fire safety</title></info><specifications>"#;

/// Every wall must carry `Pset_WallCommon.FireRating`.
const FIRE_RATING: &str = r#"<specification name="Walls are rated" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IFCWALL</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>FireRating</simpleValue></baseName></property></requirements></specification>"#;

/// An entity named in mixed case, which IDS never matches: its
/// applicability cannot be translated.
const MIXED_CASE: &str = r#"<specification name="Mixed case" ifcVersion="IFC4"><applicability minOccurs="0" maxOccurs="unbounded"><entity><name><simpleValue>IfcWall</simpleValue></name></entity></applicability><requirements><property><propertySet><simpleValue>Pset_WallCommon</simpleValue></propertySet><baseName><simpleValue>IsExternal</simpleValue></baseName></property></requirements></specification>"#;

fn ids(specifications: &[&str]) -> String {
    format!("{HEADER}{}</specifications></ids>", specifications.concat())
}

/// Walls #1 and #2 on storey `Level 1`, #3 and #4 on `Level 2`; the walls
/// in `rated` carry a fire rating.
fn model(rated: &[u32]) -> String {
    let mut data = String::from(
        "#20=IFCBUILDINGSTOREY('0000000000000000000020',$,'Level 1',$,$,$,$,$,.ELEMENT.,0.);\n\
         #21=IFCBUILDINGSTOREY('0000000000000000000021',$,'Level 2',$,$,$,$,$,.ELEMENT.,3.);\n\
         #30=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000030',$,$,$,(#1,#2),#20);\n\
         #31=IFCRELCONTAINEDINSPATIALSTRUCTURE('0000000000000000000031',$,$,$,(#3,#4),#21);\n\
         #40=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI 90'),$);\n\
         #41=IFCPROPERTYSET('0000000000000000000041',$,'Pset_WallCommon',$,(#40));\n",
    );
    for wall in 1..=4 {
        let _ = writeln!(
            data,
            "#{wall}=IFCWALL('00000000000000000000{wall:02}',$,'W{wall}',$,$,$,$,$,$);"
        );
    }
    if !rated.is_empty() {
        let related: Vec<String> = rated.iter().map(|wall| format!("#{wall}")).collect();
        let _ = writeln!(
            data,
            "#42=IFCRELDEFINESBYPROPERTIES('0000000000000000000042',$,$,$,({}),#41);",
            related.join(",")
        );
    }
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

struct Case {
    dir: PathBuf,
}

impl Case {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("ids")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(&self.dir)
            .args(args)
            .output()
            .unwrap()
    }

    /// `check --ids document.ids --model model.ifc` and `extra`.
    fn check(&self, document: &str, model: &str, extra: &[&str]) -> Output {
        self.write("rules.ids", document);
        self.write("model.ifc", model);
        let mut args = vec!["check", "--ids", "rules.ids", "--model", "model.ifc"];
        args.extend_from_slice(extra);
        self.run(&args)
    }
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The subjects of the findings, by local id.
fn found(result: &Value) -> Vec<String> {
    let mut found: Vec<String> = result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            finding["object_id"]["local_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_met_ids_document_passes_and_an_unmet_one_names_its_specification() {
    let case = Case::new("pass-fail");
    let output = case.check(&ids(&[FIRE_RATING]), &model(&[1, 2, 3, 4]), &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let result = json(&output);
    assert!(found(&result).is_empty());
    assert_eq!(result["ids"]["document"], "rules.ids");
    assert_eq!(
        result["ids"]["specifications"][0]["name"],
        "Walls are rated"
    );

    let output = case.check(&ids(&[FIRE_RATING]), &model(&[1, 4]), &[]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    assert_eq!(found(&result), ["#2", "#3"]);
    // Each finding's rule is one the failing specification ran as.
    let rules = &result["ids"]["specifications"][0]["rules"];
    for finding in result["report"]["findings"].as_array().unwrap() {
        assert!(
            rules.as_array().unwrap().contains(&finding["rule_id"]),
            "{finding:#} not in {rules}"
        );
    }
}

#[test]
fn an_untranslatable_specification_is_listed_and_the_rest_still_run() {
    let case = Case::new("gap");
    let output = case.check(&ids(&[MIXED_CASE, FIRE_RATING]), &model(&[1, 4]), &[]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let text = stderr(&output);
    assert!(
        text.contains("rules.ids: specification 1 \"Mixed case\" is not checked:"),
        "{text}"
    );
    assert!(
        text.contains("applicability facet 1: entity name \"IfcWall\""),
        "{text}"
    );
    assert!(
        text.contains("1 of 2 specification(s) not checked"),
        "{text}"
    );
    let result = json(&output);
    assert_eq!(found(&result), ["#2", "#3"]);
    let specifications = result["ids"]["specifications"].as_array().unwrap();
    assert_eq!(specifications[0]["rules"], serde_json::json!([]));
    assert_eq!(specifications[0]["gaps"].as_array().unwrap().len(), 1);
    assert!(specifications[1].get("gaps").is_none());

    // Nothing found, but a specification did not run: never a pass.
    let output = case.check(&ids(&[MIXED_CASE, FIRE_RATING]), &model(&[1, 2, 3, 4]), &[]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
}

#[test]
fn a_translated_document_runs_as_packages_as_it_does_in_memory() {
    let case = Case::new("translate");
    case.write("rules.ids", &ids(&[MIXED_CASE, FIRE_RATING]));
    case.write("model.ifc", &model(&[1, 4]));
    let output = case.run(&[
        "ids",
        "translate",
        "rules.ids",
        "--definitions",
        "definitions.json",
        "--ruleset",
        "ruleset.json",
    ]);
    // Written, without the specification that has a gap.
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("translated 1 of 2 specification(s)"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr(&output).contains("\"Mixed case\" is not checked"));
    let ruleset: Value =
        serde_json::from_str(&std::fs::read_to_string(case.dir.join("ruleset.json")).unwrap())
            .unwrap();
    assert_eq!(ruleset["package"]["id"], "ids:rules");
    assert_eq!(ruleset["root"]["folders"].as_array().unwrap().len(), 1);

    let packaged = case.run(&[
        "check",
        "--definitions",
        "definitions.json",
        "--ruleset",
        "ruleset.json",
        "--model",
        "model.ifc",
    ]);
    assert_eq!(packaged.status.code(), Some(3), "{}", stderr(&packaged));
    let in_memory = case.run(&["check", "--ids", "rules.ids", "--model", "model.ifc"]);
    let (mut packaged, in_memory) = (json(&packaged), json(&in_memory));
    assert_eq!(found(&packaged), found(&in_memory));
    packaged["ids"] = in_memory["ids"].clone();
    assert_eq!(packaged, in_memory);

    // A complete document exits 0.
    case.write("complete.ids", &ids(&[FIRE_RATING]));
    let output = case.run(&[
        "ids",
        "translate",
        "complete.ids",
        "--definitions",
        "d.json",
        "--ruleset",
        "r.json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

/// The objects contained in the storey named `Level 1`, in IFC names.
const LEVEL_1: &str = r#"{"kind": "related", "path": ["IfcRelContainedInSpatialStructure:backward"], "selector": {"kind": "allOf", "operands": [{"kind": "entityType", "objectType": "IfcBuildingStorey"}, {"kind": "property", "propertySet": "axioval:attributes", "property": "Name", "operator": "equals", "value": {"type": "string", "value": "Level 1"}}]}}"#;

#[test]
fn a_prefilter_restricts_every_specification_to_one_storey() {
    let case = Case::new("filter");
    let unfiltered = case.check(&ids(&[FIRE_RATING]), &model(&[1, 4]), &[]);
    assert_eq!(found(&json(&unfiltered)), ["#2", "#3"]);

    case.write("level-1.json", LEVEL_1);
    let output = case.check(
        &ids(&[FIRE_RATING]),
        &model(&[1, 4]),
        &["--ids-filter", "level-1.json"],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result = json(&output);
    // Only the unrated wall on Level 1; #3 on Level 2 is not checked.
    assert_eq!(found(&result), ["#2"]);
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{result:#}"
    );
    assert_eq!(result["ids"]["filter"]["kind"], "related");

    // Every wall on Level 1 rated: a pass, whatever Level 2 lacks.
    let output = case.check(
        &ids(&[FIRE_RATING]),
        &model(&[1, 2]),
        &["--ids-filter", "level-1.json"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    // `ids translate` writes the filtered packages.
    let output = case.run(&[
        "ids",
        "translate",
        "rules.ids",
        "--ids-filter",
        "level-1.json",
        "--definitions",
        "d.json",
        "--ruleset",
        "r.json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let packaged = case.run(&[
        "check",
        "--definitions",
        "d.json",
        "--ruleset",
        "r.json",
        "--model",
        "model.ifc",
    ]);
    assert_eq!(packaged.status.code(), Some(0), "{}", stderr(&packaged));

    // A filter needs `--ids`, and one naming a rule is refused.
    let output = case.run(&[
        "check",
        "--ids-filter",
        "level-1.json",
        "--model",
        "model.ifc",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    case.write(
        "rule.json",
        r#"{"kind": "ruleOutcome", "rule": "spec1.facet1", "outcome": "passed"}"#,
    );
    let output = case.check(
        &ids(&[FIRE_RATING]),
        &model(&[]),
        &["--ids-filter", "rule.json"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("never by a rule"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn ids_and_packages_are_exclusive_and_bad_documents_exit_1() {
    let case = Case::new("usage");
    case.write("rules.ids", &ids(&[FIRE_RATING]));
    case.write("model.ifc", &model(&[]));
    let output = case.run(&[
        "check",
        "--ids",
        "rules.ids",
        "--ruleset",
        "r.json",
        "--model",
        "model.ifc",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let output = case.run(&["check", "--model", "model.ifc"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));

    case.write("broken.ids", "<ids>");
    let output = case.run(&["check", "--ids", "broken.ids", "--model", "model.ifc"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("broken.ids"),
        "{}",
        stderr(&output)
    );
    assert!(output.stdout.is_empty());
}

/// `ids export` over the packages in `case`, into `exported.ids`.
fn export(case: &Case) -> Output {
    case.run(&[
        "ids",
        "export",
        "--definitions",
        "definitions.json",
        "--ruleset",
        "ruleset.json",
        "--out",
        "exported.ids",
    ])
}

#[test]
fn an_exported_package_lists_the_rules_ids_cannot_state() {
    let case = Case::new("export");
    case.write("rules.ids", &ids(&[FIRE_RATING]));
    let output = case.run(&[
        "ids",
        "translate",
        "rules.ids",
        "--definitions",
        "definitions.json",
        "--ruleset",
        "ruleset.json",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    // A translated document is written back as it was.
    let output = export(&case);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let read = |name: &str| {
        openbim_ids::from_str(&std::fs::read_to_string(case.dir.join(name)).unwrap()).unwrap()
    };
    assert_eq!(
        read("exported.ids").specifications,
        read("rules.ids").specifications
    );

    // A clash rule beside it has no IDS facet.
    let load = |name: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(case.dir.join(name)).unwrap()).unwrap()
    };
    let mut definitions = load("definitions.json");
    definitions["definitions"]["t:clash"] = serde_json::json!({
        "id": "t:clash",
        "name": {"default": "Clash", "translations": {}},
        "description": null,
        "capability": "axioval:capability.clash",
    });
    let mut ruleset = load("ruleset.json");
    ruleset["root"]["rules"] = serde_json::json!([{
        "id": "walls-clash",
        "definitionId": "t:clash",
        "name": {"default": "Walls clash", "translations": {}},
    }]);
    case.write("definitions.json", &definitions.to_string());
    case.write("ruleset.json", &ruleset.to_string());
    std::fs::remove_file(case.dir.join("exported.ids")).unwrap();
    let output = export(&case);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(
            "rule walls-clash is not exported: capability axioval:capability.clash has no IDS facet"
        ),
        "{}",
        stderr(&output)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("exported 1 rule(s) as 1 specification(s); 1 rule(s) not exported"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        read("exported.ids").specifications,
        read("rules.ids").specifications
    );

    // With nothing exportable, nothing is written.
    ruleset["root"]["folders"] = serde_json::json!([]);
    case.write("ruleset.json", &ruleset.to_string());
    std::fs::remove_file(case.dir.join("exported.ids")).unwrap();
    let output = export(&case);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("rule walls-clash is not exported"));
    assert!(stderr(&output).contains("no rule can be exported as IDS"));
    assert!(!case.dir.join("exported.ids").exists());
}
