//! `.mcs` package archives wherever `--ruleset` or `--definitions` are
//! read: a check with archives reports what the same check with the
//! normalized JSON they carry reports.
//!
//! The archives are `fixtures/mcs-v0.1.0`, packed by the Axioval MCS packer
//! at commit `42c97b547c98f1dc28dcda436a776db3fae5d15d` (see
//! `fixtures/README.md`).
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../fixtures/mcs-v0.1.0");

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(format!("{FIXTURES}/{name}"))
}

/// Walls #1 and #2 (a subtype) and a slab; only #1 has `Reference` in
/// `Pset_WallCommon`.
const MODEL: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
#1=IFCWALL('0000000000000000000001',$,$,$,$,$,$,$,$);\n\
#2=IFCWALLSTANDARDCASE('0000000000000000000002',$,$,$,$,$,$,$,$);\n\
#3=IFCSLAB('0000000000000000000003',$,$,$,$,$,$,$,$);\n\
#4=IFCPROPERTYSINGLEVALUE('Reference',$,IFCIDENTIFIER('W-1'),$);\n\
#5=IFCPROPERTYSET('0000000000000000000005',$,'Pset_WallCommon',$,(#4));\n\
#6=IFCRELDEFINESBYPROPERTIES('0000000000000000000006',$,$,$,(#1),#5);\n\
ENDSEC;\nEND-ISO-10303-21;\n";

struct Case {
    dir: PathBuf,
}

impl Case {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("mcs-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.ifc"), MODEL).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The normalized JSON an archive carries, written beside the model:
    /// its rule set as `<stem>.ruleset.json` and each definition document
    /// as `<stem>.<module>.json`, by the source module it was evaluated
    /// from.
    fn unpack(&self, archive: &str) -> BTreeMap<String, PathBuf> {
        let file = std::fs::File::open(fixture(archive)).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut read = |name: &str| {
            let mut text = String::new();
            zip.by_name(name)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            text
        };
        let metadata: Value = serde_json::from_str(&read("META-INF/mcs.json")).unwrap();
        let stem = archive.trim_end_matches(".mcs");
        let mut written = BTreeMap::new();
        for (module, member) in metadata["normalized"].as_object().unwrap() {
            let module = module.trim_end_matches(".pkl");
            let path = self.path(&format!("{stem}.{module}.json"));
            std::fs::write(&path, read(member.as_str().unwrap())).unwrap();
            written.insert(module.to_owned(), path);
        }
        written
    }

    fn run(&self, command: &str, args: &[&Path]) -> Output {
        let mut process = Command::new(env!("CARGO_BIN_EXE_axioval"));
        process.arg(command);
        if command == "check" {
            process.arg("--model").arg(self.path("model.ifc"));
        }
        process
            .args(args)
            .env("SOURCE_DATE_EPOCH", "1790416800")
            .output()
            .unwrap()
    }

    /// `check` with `--ruleset` and `--definitions` files, in that order.
    fn check(&self, rulesets: &[&Path], definitions: &[&Path]) -> Output {
        let mut args: Vec<&Path> = Vec::new();
        for ruleset in rulesets {
            args.extend([Path::new("--ruleset"), ruleset]);
        }
        for definitions in definitions {
            args.extend([Path::new("--definitions"), definitions]);
        }
        self.run("check", &args)
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn a_ruleset_archive_with_a_template_library_checks_as_its_json_does() {
    let case = Case::new("library");
    let rules = fixture("library-rules.mcs");
    let library = fixture("template-library.mcs");
    let archives = case.check(&[&rules], &[&library]);
    assert_eq!(archives.status.code(), Some(3), "{}", stderr(&archives));
    let findings = &json(&archives)["report"]["findings"];
    assert_eq!(findings.as_array().unwrap().len(), 1, "{findings:#}");
    assert_eq!(findings[0]["object_id"]["local_id"], "#2");

    let unpacked = case.unpack("library-rules.mcs");
    let ruleset = &unpacked["ruleset"];
    let library_json = &case.unpack("template-library.mcs")["definitions"];
    let equivalent = case.check(&[ruleset], &[&unpacked["concepts"], library_json]);
    assert_eq!(equivalent.status.code(), Some(3), "{}", stderr(&equivalent));
    assert_eq!(json(&archives), json(&equivalent));

    // The rule-set archive carries the library: the archive alone checks
    // the same, and so does its JSON with the library archive.
    let alone = case.check(&[&rules], &[]);
    assert_eq!(alone.status.code(), Some(3), "{}", stderr(&alone));
    assert_eq!(json(&alone), json(&equivalent));
    let mixed = case.check(&[ruleset], &[&unpacked["concepts"], &library]);
    assert_eq!(mixed.status.code(), Some(3), "{}", stderr(&mixed));
    assert_eq!(json(&mixed), json(&equivalent));
}

#[test]
fn the_minimal_ruleset_archive_checks_as_its_json_does() {
    let case = Case::new("minimal");
    let archive = fixture("minimal.mcs");
    let library = fixture("template-library.mcs");
    let archives = case.check(&[&archive], &[&library]);
    let unpacked = case.unpack("minimal.mcs");
    let library_json = &case.unpack("template-library.mcs")["definitions"];
    let equivalent = case.check(
        &[&unpacked["ruleset"]],
        &[&unpacked["definitions"], library_json],
    );
    assert_eq!(
        archives.status.code(),
        equivalent.status.code(),
        "{}\n{}",
        stderr(&archives),
        stderr(&equivalent)
    );
    assert!(
        matches!(archives.status.code(), Some(3 | 4)),
        "{}",
        stderr(&archives)
    );
    assert_eq!(json(&archives), json(&equivalent));
}

#[test]
fn an_archive_given_as_the_other_kind_is_refused() {
    let case = Case::new("kinds");
    let library = fixture("template-library.mcs");
    let rules = fixture("library-rules.mcs");
    let output = case.check(&[&library], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output)
            .contains("a definitions archive was given where a ruleset archive is expected"),
        "{}",
        stderr(&output)
    );
    assert!(output.stdout.is_empty());
    let output = case.check(&[&rules], &[&rules]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output)
            .contains("a ruleset archive was given where a definitions archive is expected"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_definition_package_given_by_nothing_is_refused() {
    let case = Case::new("unknown");
    let unpacked = case.unpack("library-rules.mcs");
    // The library is neither carried nor supplied.
    let output = case.check(&[&unpacked["ruleset"]], &[&unpacked["concepts"]]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("axioval:example.template-library"),
        "{}",
        stderr(&output)
    );
    // A carried copy that differs from the one supplied is refused too.
    let mut library: Value =
        serde_json::from_slice(&std::fs::read(&unpacked["library"]).unwrap()).unwrap();
    library["package"]["version"] = "0.1.1".into();
    let changed = case.path("changed-library.json");
    std::fs::write(&changed, library.to_string()).unwrap();
    let output = case.check(&[&fixture("library-rules.mcs")], &[&changed]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("two different definition packages are named"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_tampered_archive_is_refused_before_anything_runs() {
    let case = Case::new("tampered");
    let mut bytes = std::fs::read(fixture("library-rules.mcs")).unwrap();
    bytes.push(0);
    // Without the extension, the ZIP magic still reads it as an archive.
    let tampered = case.path("rules.bin");
    std::fs::write(&tampered, bytes).unwrap();
    let output = case.check(&[&tampered], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("not a canonical MCS container"),
        "{}",
        stderr(&output)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn validate_and_catalogue_read_archives() {
    let case = Case::new("validate");
    let rules = fixture("library-rules.mcs");
    let library = fixture("template-library.mcs");
    let output = case.run("validate", &[Path::new("--ruleset"), &rules]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = case.run("catalogue", &[Path::new("--definitions"), &library]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("axioval:example.template.property-exists"),
        "{text}"
    );
}
