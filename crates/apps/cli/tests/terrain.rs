//! `axioval check --terrain`: terrain surfaces read as context beside the
//! models, end to end through the real binary.
#![allow(missing_docs)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

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
