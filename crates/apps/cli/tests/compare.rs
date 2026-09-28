//! `axioval compare` end to end: the real binary over two revisions of one
//! IFC model.
//!
//! The revisions are exported with different entity numbers, as every
//! re-export renumbers, so matching must follow the `GlobalId`. Exit status and
//! output shape are automation contracts, so every case asserts the status
//! first.
#![allow(missing_docs)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// One element of a revision.
struct Element {
    entity: &'static str,
    global_id: &'static str,
    /// Placement origin, in metres.
    at: [f64; 3],
    /// Box size, in metres.
    size: [f64; 3],
    fire_rating: Option<&'static str>,
}

const fn element(
    entity: &'static str,
    global_id: &'static str,
    at: [f64; 3],
    fire_rating: Option<&'static str>,
) -> Element {
    Element {
        entity,
        global_id,
        at,
        size: [4.0, 0.2, 3.0],
        fire_rating,
    }
}

/// A revision: every element placed by its own `IfcLocalPlacement`, with an
/// extruded box body and, when given, a `Pset_WallCommon.FireRating`.
/// `first` offsets every entity number, so two revisions of one model number
/// their entities differently. `world` is the model context's origin.
fn revision(first: u32, world: [f64; 3], elements: &[Element]) -> String {
    let mut data = String::new();
    for (index, element) in elements.iter().enumerate() {
        let base = first + 20 * u32::try_from(index).unwrap();
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
        ] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map(|offset| base + offset);
        let [x, y, z] = element.at;
        let [length, width, depth] = element.size;
        let _ = write!(
            data,
            "#{origin}=IFCCARTESIANPOINT(({x:.3},{y:.3},{z:.3}));\n\
             #{frame}=IFCAXIS2PLACEMENT3D(#{origin},$,$);\n\
             #{placement}=IFCLOCALPLACEMENT($,#{frame});\n\
             #{p}=IFCCARTESIANPOINT(({:.3},{:.3}));\n\
             #{pos}=IFCAXIS2PLACEMENT2D(#{p},$);\n\
             #{profile}=IFCRECTANGLEPROFILEDEF(.AREA.,$,#{pos},{length:.3},{width:.3});\n\
             #{solid}=IFCEXTRUDEDAREASOLID(#{profile},#2,#4,{depth:.3});\n\
             #{shape}=IFCSHAPEREPRESENTATION(#5,'Body','SweptSolid',(#{solid}));\n\
             #{definition}=IFCPRODUCTDEFINITIONSHAPE($,$,(#{shape}));\n\
             #{object}={}('{}',$,$,$,$,#{placement},#{definition},$,$);\n",
            length / 2.0,
            width / 2.0,
            element.entity,
            element.global_id,
        );
        if let Some(rating) = element.fire_rating {
            let [value, set, relation] = [10, 11, 12].map(|offset| base + offset);
            let _ = write!(
                data,
                "#{value}=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('{rating}'),$);\n\
                 #{set}=IFCPROPERTYSET('{set:022}',$,'Pset_WallCommon',$,(#{value}));\n\
                 #{relation}=IFCRELDEFINESBYPROPERTIES('{relation:022}',$,$,$,(#{object}),#{set});\n"
            );
        }
    }
    let [wx, wy, wz] = world;
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
         #1=IFCCARTESIANPOINT((0.,0.,0.));\n\
         #2=IFCAXIS2PLACEMENT3D(#1,$,$);\n\
         #3=IFCCARTESIANPOINT(({wx:.3},{wy:.3},{wz:.3}));\n\
         #4=IFCDIRECTION((0.,0.,1.));\n\
         #5=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#6,$);\n\
         #6=IFCAXIS2PLACEMENT3D(#3,$,$);\n\
         #7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
         #8=IFCUNITASSIGNMENT((#7));\n\
         #9=IFCPROJECT('0000000000000000000009',$,'P',$,$,$,$,(#5),#8);\n\
         {data}ENDSEC;\nEND-ISO-10303-21;\n"
    )
}

const MOVED: &str = "0000000000000000000A01";
const RATED: &str = "0000000000000000000A02";
const REMOVED: &str = "0000000000000000000A03";
const KEPT: &str = "0000000000000000000A04";
const ADDED: &str = "0000000000000000000A05";

/// The first revision.
fn base() -> String {
    revision(
        100,
        [0.0; 3],
        &[
            element("IFCWALL", MOVED, [0.0, 0.0, 0.0], Some("EI30")),
            element("IFCWALL", RATED, [0.0, 5.0, 0.0], Some("EI30")),
            element("IFCWALL", REMOVED, [0.0, 10.0, 0.0], None),
            element("IFCSLAB", KEPT, [0.0, 15.0, 0.0], None),
        ],
    )
}

/// The second revision, renumbered: the first wall moved half a metre, the
/// second rerated, the third removed, and a column added.
fn revised() -> String {
    revision(
        500,
        [0.0; 3],
        &[
            element("IFCSLAB", KEPT, [0.0, 15.0, 0.0], None),
            element("IFCWALL", RATED, [0.0, 5.0, 0.0], Some("EI60")),
            element("IFCWALL", MOVED, [0.5, 0.0, 0.0], Some("EI30")),
            element("IFCCOLUMN", ADDED, [8.0, 0.0, 0.0], None),
        ],
    )
}

struct Case {
    dir: PathBuf,
}

impl Case {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("compare-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("r1")).unwrap();
        std::fs::create_dir_all(dir.join("r2")).unwrap();
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

    fn compare(&self, base: &Path, revised: &Path, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_axioval"))
            .current_dir(&self.dir)
            .arg("compare")
            .arg("--base")
            .arg(base)
            .arg("--revised")
            .arg(revised)
            .args(extra)
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap()
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn object<'a>(comparison: &'a Value, identity: &str) -> &'a Value {
    comparison["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|object| object["identity"] == identity)
        .unwrap_or_else(|| panic!("{identity} is listed: {comparison:#}"))
}

/// `(rule id, local id)` of every finding, in report order.
fn findings(result: &Value) -> Vec<(String, String)> {
    result["report"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| {
            (
                finding["rule_id"].as_str().unwrap().to_owned(),
                finding["object_id"]["local_id"]
                    .as_str()
                    .unwrap_or("-")
                    .to_owned(),
            )
        })
        .collect()
}

/// Compares the two revisions on every facet, saving the result and a BCF
/// archive in `case`.
fn compare_revisions(case: &Case) -> (Value, PathBuf, PathBuf) {
    let before = case.write("r1/model.ifc", &base());
    let after = case.write("r2/model.ifc", &revised());
    let saved = case.path("comparison.json");
    let bcf = case.path("changes.bcfzip");
    let output = case.compare(
        &before,
        &after,
        &[
            "--property",
            "Pset_WallCommon.FireRating",
            "--geometry",
            "--report",
            saved.to_str().unwrap(),
            "--bcf",
            bcf.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    (result, saved, bcf)
}

#[test]
fn two_revisions_report_a_moved_a_changed_an_added_and_a_removed_element() {
    let case = Case::new("revisions");
    let (result, _, _) = compare_revisions(&case);
    let comparison = &result["comparison"];

    // Both files are called model.ifc; the sources stay two.
    assert_eq!(comparison["base"], "ifc-step:model.ifc@base");
    assert_eq!(comparison["revised"], "ifc-step:model.ifc@revised");
    assert_eq!(comparison["scheme"], "ifc-globalid");
    assert_eq!(
        comparison["facets"],
        serde_json::json!([
            "kind",
            "classifications",
            "property",
            "relationship",
            "placement",
            "geometry",
            "coordinate-system"
        ])
    );
    assert_eq!(
        comparison["counts"],
        serde_json::json!({"added": 1, "removed": 1, "changed": 2, "unchanged": 2,
                           "incomplete": 0, "unidentified": 0, "ambiguous": 0})
    );

    let moved = object(comparison, MOVED);
    assert_eq!(moved["state"], "changed");
    assert_eq!(moved["kind"], "IFCWALL");
    // Matched by GlobalId across renumbering: #109 before, #549 after.
    assert_eq!(moved["base"]["local_id"], "#109");
    assert_eq!(moved["revised"]["local_id"], "#549");
    let changes: Vec<(&str, &str, f64)> = moved["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| {
            (
                change["facet"].as_str().unwrap(),
                change["measure"].as_str().unwrap(),
                change["upper"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(changes.len(), 2, "{moved:#}");
    assert_eq!((changes[0].0, changes[0].1), ("placement", "origin"));
    assert!((changes[0].2 - 0.5).abs() < 1e-9);
    assert_eq!((changes[1].0, changes[1].1), ("geometry", "bounds"));
    assert!((changes[1].2 - 0.5).abs() < 1e-9);
    assert_eq!(moved["changes"][0]["unit"], "m");

    let rated = object(comparison, RATED);
    assert_eq!(rated["state"], "changed");
    assert_eq!(
        rated["changes"][0]["detail"],
        "property Pset_WallCommon.FireRating \"EI30\" -> \"EI60\""
    );
    assert_eq!(object(comparison, REMOVED)["state"], "removed");
    let added = object(comparison, ADDED);
    assert_eq!(
        (&added["state"], &added["kind"]),
        (&"added".into(), &"IFCCOLUMN".into())
    );
    assert!(
        comparison["objects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|object| object["identity"] != KEPT),
        "unchanged objects are only counted"
    );
    let systems = comparison["coordinate_systems"].as_array().unwrap();
    assert_eq!(systems.len(), 1);
    assert!(systems[0].get("changes").is_none(), "{systems:#?}");
}

#[test]
fn a_comparison_is_a_report_one_rule_per_facet_readable_and_exportable() {
    let case = Case::new("report");
    let (result, saved, bcf) = compare_revisions(&case);

    assert_eq!(
        findings(&result),
        vec![
            ("compare.added".into(), "#569".into()),
            ("compare.geometry".into(), "#549".into()),
            ("compare.placement".into(), "#549".into()),
            ("compare.property".into(), "#529".into()),
            ("compare.removed".into(), "#149".into()),
        ]
    );
    assert!(
        result["report"]["not_evaluated"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let related = &result["report"]["findings"][2]["related"][0];
    assert_eq!(related["source"]["document"], "model.ifc@base");
    assert_eq!(result["geometry"]["exact"], 8);
    assert_eq!(
        result["objects"]["ifc-step:model.ifc@revised/#549"]["global_id"],
        MOVED
    );

    let archive = openbim_bcf::read_path(&bcf).unwrap();
    assert!(
        archive.diagnostics().is_empty(),
        "{:?}",
        archive.diagnostics()
    );
    assert_eq!(archive.topics().count(), 5);

    // The saved result reads back like a check's.
    let listing = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .args([
            "report",
            saved.to_str().unwrap(),
            "--rule",
            "compare.placement",
        ])
        .output()
        .unwrap();
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    assert!(stdout(&listing).contains("placement origin differs by 0.5000 m"));
    let summary = Command::new(env!("CARGO_BIN_EXE_axioval"))
        .args(["report", saved.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        stdout(&summary).contains("objects: 1 added · 1 removed · 2 changed · 2 unchanged"),
        "{}",
        stdout(&summary)
    );
}

#[test]
fn identical_revisions_exit_0_and_a_summary_names_the_counts() {
    let case = Case::new("identical");
    let before = case.write("r1/a.ifc", &base());
    let after = case.write(
        "r2/b.ifc",
        &revision(700, [0.0; 3], &{
            // The same elements, renumbered and reordered.
            let mut same = vec![
                element("IFCSLAB", KEPT, [0.0, 15.0, 0.0], None),
                element("IFCWALL", REMOVED, [0.0, 10.0, 0.0], None),
                element("IFCWALL", RATED, [0.0, 5.0, 0.0], Some("EI30")),
            ];
            same.push(element("IFCWALL", MOVED, [0.0, 0.0, 0.0], Some("EI30")));
            same
        }),
    );
    let output = case.compare(
        &before,
        &after,
        &["--property", "Pset_WallCommon.FireRating", "--summary"],
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.starts_with("compared: ifc-step:a.ifc -> ifc-step:b.ifc · kind, classifications, property, relationship, placement, coordinate-system\n"), "{text}");
    assert!(
        text.contains("objects: 0 added · 0 removed · 0 changed · 5 unchanged"),
        "{text}"
    );
    assert!(text.contains("status: passed"), "{text}");
}

#[test]
fn a_move_within_the_tolerance_is_no_change_and_a_moved_world_origin_is() {
    let case = Case::new("tolerance");
    let before = case.write(
        "r1/model.ifc",
        &revision(
            100,
            [0.0; 3],
            &[element("IFCWALL", MOVED, [0.0, 0.0, 0.0], None)],
        ),
    );
    // Moved 3 mm, under the default 5 mm tolerance, in a model whose world
    // origin moved 10 m east.
    let after = case.write(
        "r2/model.ifc",
        &revision(
            100,
            [10.0, 0.0, 0.0],
            &[element("IFCWALL", MOVED, [0.003, 0.0, 0.0], None)],
        ),
    );
    let output = case.compare(&before, &after, &[]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        findings(&result),
        vec![("compare.coordinate-system".into(), "-".into())]
    );
    let finding = &result["report"]["findings"][0];
    assert_eq!(finding["source"]["document"], "model.ifc@revised");
    assert!(
        finding["message"]
            .as_str()
            .unwrap()
            .contains("coordinate-system world-origin differs by 10.0000 m"),
        "{finding:#}"
    );

    // A tighter tolerance sees the 3 mm move.
    let output = case.compare(&before, &after, &["--length-tolerance", "0.001"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        findings(&result).contains(&("compare.placement".into(), "#109".into())),
        "{result:#}"
    );
}

#[test]
fn unreadable_input_and_bad_tolerances_exit_1() {
    let case = Case::new("unusable");
    let before = case.write("r1/model.ifc", &base());
    let output = case.compare(&before, &case.path("r2/missing.ifc"), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("missing.ifc"),
        "{}",
        stderr(&output)
    );
    let output = case.compare(&before, &before, &["--length-tolerance=-1"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("tolerance"), "{}", stderr(&output));
    let output = case.compare(&before, &before, &["--bcf-date", "2026-01-01T00:00:00Z"]);
    assert_eq!(output.status.code(), Some(2), "BCF options need --bcf");
}

#[test]
fn every_property_set_is_listed_and_compared_on_request() {
    let case = Case::new("revisions-property-sets");
    let before = case.write("r1/model.ifc", &base());
    let after = case.write("r2/model.ifc", &revised());
    let saved = case.path("comparison.json");
    let run = |extra: &[&str]| {
        let mut args = vec!["--report", saved.to_str().unwrap()];
        args.extend_from_slice(extra);
        let output = case.compare(&before, &after, &args);
        let result: Value =
            serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
        (output, result)
    };
    for extra in [
        &["--all-property-sets"][..],
        &["--property-set", "Pset_WallCommon"],
    ] {
        let (output, result) = run(extra);
        assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
        let rated = object(&result["comparison"], RATED);
        assert_eq!(rated["state"], "changed", "{result:#}");
        assert!(
            rated["changes"][0]["detail"]
                .as_str()
                .unwrap()
                .contains("Pset_WallCommon.FireRating"),
            "{result:#}"
        );
    }
}
