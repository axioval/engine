//! The translation against the buildingSMART IDS test corpus.
//!
//! Each case pairs an `.ids` with an `.ifc` and states in its name whether the
//! model passes. The translation is run through the real IFC adapter and
//! engine, and every case must land in a class that is consistent with it:
//!
//! - a `pass-` case produces no finding: translated rules never invent a
//!   failure, complete or not;
//! - a `fail-` case either produces a finding, or its translation reported a
//!   gap that explains why not. A complete translation with no finding is a
//!   miss.
//!
//! Findings about a whole source count like findings about objects: a
//! required specification with no applicable object fails that way.
//!
//! `invalid-` cases judge the IDS against the IFC schema and are skipped.
//! The corpus is CC BY-ND 4.0 and is not vendored: point `IDS_TEST_CASES` at
//! `Documentation/ImplementersDocumentation/TestCases` of
//! <https://github.com/buildingSMART/IDS> and run
//! `cargo test -p axioval-ids -- --ignored corpus`. `IDS_CORPUS_VERBOSE` lists
//! every case.
#![allow(missing_docs, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axioval::default_registry;
use axioval::engine::{Runtime, compile};
use axioval::ifc::import_ifc_session;
use axioval_ids::{Options, translate};

fn cases() -> Vec<PathBuf> {
    let root = PathBuf::from(
        std::env::var_os("IDS_TEST_CASES")
            .expect("set IDS_TEST_CASES to the buildingSMART IDS TestCases directory"),
    );
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable corpus directory") {
            let path = entry.expect("readable entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ids"))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// A pass case whose complete translation found nothing.
    ExactPass,
    /// A pass case whose partial translation found nothing.
    SoundPass,
    /// A fail case the rules caught.
    CaughtFail,
    /// A fail case with no finding and gaps that may explain it.
    UnjudgedFail,
    /// The adapter refused the model, e.g. one declaring several schemas.
    ModelRefused,
    /// Rules could not be evaluated; not a verdict either way.
    NotEvaluated,
    /// A false failure or an unexplained miss.
    Mismatch,
}

fn classify(case: &Path) -> Option<(Class, String)> {
    let stem = case.file_stem()?.to_str()?;
    let expected_pass = if stem.starts_with("pass-") {
        true
    } else if stem.starts_with("fail-") {
        false
    } else {
        return None;
    };
    let ids = openbim_ids::from_slice(&std::fs::read(case).ok()?).expect("corpus IDS reads");
    let options = Options::new("ids:corpus", "1.0.0");
    let translation = translate(&ids, &options).expect("valid options");
    let model = std::fs::read(case.with_extension("ifc")).ok()?;
    let session = match import_ifc_session("model.ifc", &model) {
        Ok(session) => session,
        Err(error) => return Some((Class::ModelRefused, error.to_string())),
    };
    let registry = default_registry().expect("built-in registry");
    let plan = compile(
        &registry,
        &[translation.definitions.clone()],
        &translation.ruleset,
    )
    .expect("translated packages compile");
    let report = Runtime::new(registry)
        .run_session(&session, plan)
        .expect("plan runs");
    let findings = report.findings().len();
    let mut detail = format!(
        "{} finding(s), {} not evaluated, gaps: [{}]",
        findings,
        report.not_evaluated().len(),
        translation
            .gaps()
            .map(|(_, gap)| gap.to_string())
            .collect::<Vec<_>>()
            .join("; "),
    );
    if std::env::var_os("IDS_CORPUS_VERBOSE").is_some() {
        for finding in report.findings() {
            detail.push_str("\n        finding: ");
            detail.push_str(&finding.message);
        }
        for outcome in report.not_evaluated() {
            detail.push_str("\n        not evaluated: ");
            detail.push_str(&outcome.message);
        }
    }
    let class = match (expected_pass, findings > 0) {
        (true, true) => Class::Mismatch,
        (false, true) => Class::CaughtFail,
        _ if !report.not_evaluated().is_empty() => Class::NotEvaluated,
        (true, false) if translation.is_complete() => Class::ExactPass,
        (true, false) => Class::SoundPass,
        (false, false) if translation.is_complete() => Class::Mismatch,
        (false, false) => Class::UnjudgedFail,
    };
    Some((class, detail))
}

fn run() {
    let mut classes: BTreeMap<Class, Vec<String>> = BTreeMap::new();
    // Classes per facet directory of the corpus.
    let mut facets: BTreeMap<String, BTreeMap<Class, usize>> = BTreeMap::new();
    for case in cases() {
        if let Some((class, detail)) = classify(&case) {
            let name = case.file_stem().unwrap().to_string_lossy().into_owned();
            let facet = case
                .parent()
                .and_then(Path::file_name)
                .map_or_else(String::new, |facet| facet.to_string_lossy().into_owned());
            *facets.entry(facet).or_default().entry(class).or_default() += 1;
            classes
                .entry(class)
                .or_default()
                .push(format!("{name}: {detail}"));
        }
    }
    for (facet, counts) in &facets {
        let shown: Vec<String> = counts
            .iter()
            .map(|(class, count)| format!("{class:?} {count}"))
            .collect();
        println!("{facet}: {}", shown.join(", "));
    }
    for (class, cases) in &classes {
        println!("{class:?}: {}", cases.len());
        if std::env::var_os("IDS_CORPUS_VERBOSE").is_some()
            || matches!(class, Class::NotEvaluated | Class::ModelRefused)
        {
            for case in cases {
                println!("    {case}");
            }
        }
    }
    let mismatches = classes.get(&Class::Mismatch).cloned().unwrap_or_default();
    assert!(
        mismatches.is_empty(),
        "mismatches:\n{}",
        mismatches.join("\n")
    );
}

#[test]
#[ignore = "needs a local buildingSMART IDS checkout in IDS_TEST_CASES"]
fn corpus_translations_never_contradict_the_expected_verdict() {
    run();
}

/// What a run reports, comparable across two runs of the same rules.
fn verdicts(
    translation: &axioval_ids::Translation,
    session: &axioval::engine::EvidenceSession,
) -> Vec<String> {
    let registry = default_registry().expect("built-in registry");
    let plan = compile(
        &registry,
        &[translation.definitions.clone()],
        &translation.ruleset,
    )
    .expect("translated packages compile");
    let report = Runtime::new(registry)
        .run_session(session, plan)
        .expect("plan runs");
    let mut verdicts: Vec<String> = report
        .findings()
        .iter()
        .map(|finding| {
            format!(
                "finding {} {:?} {:?} {}",
                finding.rule_id, finding.scope, finding.severity, finding.message
            )
        })
        .chain(report.not_evaluated().iter().map(|outcome| {
            format!(
                "not evaluated {} {:?} {:?} {}",
                outcome.rule_id, outcome.scope, outcome.reason, outcome.message
            )
        }))
        .collect();
    verdicts.sort();
    verdicts
}

/// What each rule, named by `name`, reported about which object: the
/// verdicts without their wording, which a rule read on its own words
/// otherwise.
fn subjects(
    translation: &axioval_ids::Translation,
    session: &axioval::engine::EvidenceSession,
    name: &dyn Fn(&str) -> String,
) -> Vec<String> {
    let registry = default_registry().expect("built-in registry");
    let plan = compile(
        &registry,
        &[translation.definitions.clone()],
        &translation.ruleset,
    )
    .expect("translated packages compile");
    let report = Runtime::new(registry)
        .run_session(session, plan)
        .expect("plan runs");
    let mut subjects: Vec<String> = report
        .findings()
        .iter()
        .map(|finding| {
            format!(
                "{} finding {:?} {:?}",
                name(&finding.rule_id.to_string()),
                finding.scope,
                finding.severity
            )
        })
        .chain(report.not_evaluated().iter().map(|outcome| {
            format!(
                "{} not evaluated {:?} {:?}",
                name(&outcome.rule_id.to_string()),
                outcome.scope,
                outcome.reason
            )
        }))
        .collect();
    subjects.sort();
    subjects
}

/// The corpus documents that translate without a gap are exported again,
/// specification by specification as they were read, and the export
/// translates to rules that report exactly what the original's did on the
/// case's model.
fn round_trip() {
    let written = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ids-export");
    let _ = std::fs::remove_dir_all(&written);
    std::fs::create_dir_all(&written).expect("export directory");
    let options = Options::new("ids:corpus", "1.0.0");
    let (mut complete, mut identical, mut refused) = (0, 0, 0);
    let (mut rules_total, mut rules_read) = (0, 0);
    let mut exported = Vec::new();
    let mut read_files = Vec::new();
    let mut failures = Vec::new();
    for case in cases() {
        let name = case.file_stem().unwrap().to_string_lossy().into_owned();
        if name.starts_with("invalid-") {
            continue;
        }
        let ids = openbim_ids::from_slice(&std::fs::read(&case).expect("readable case"))
            .expect("corpus IDS reads");
        let translation = translate(&ids, &options).expect("valid options");
        if !translation.is_complete() {
            continue;
        }
        complete += 1;
        let export = axioval_ids::export(
            std::slice::from_ref(&translation.definitions),
            &translation.ruleset,
        );
        if !export.is_complete() {
            let listed: Vec<String> = export
                .not_exported
                .iter()
                .map(ToString::to_string)
                .collect();
            failures.push(format!("{name}: not exported: {}", listed.join("; ")));
            continue;
        }
        let xml = export.to_xml().expect("a specification");
        // The profile every export target shares writes the same document
        // and loses nothing.
        let outcome = axioval_export::ExportProfile::export(
            &axioval_ids::IdsProfile,
            std::slice::from_ref(&translation.definitions),
            &translation.ruleset,
        );
        if !outcome.is_complete() || outcome.artifact.as_deref() != Some(xml.as_bytes()) {
            failures.push(format!("{name}: the ids profile differs from export"));
            continue;
        }
        let path = written.join(format!("{name}.ids"));
        std::fs::write(&path, &xml).expect("writable export");
        exported.push(path);
        let again = match openbim_ids::from_str(&xml) {
            Ok(again) => again,
            Err(error) => {
                failures.push(format!("{name}: the export does not read: {error}"));
                continue;
            }
        };
        if again.info != ids.info || again.specifications != ids.specifications {
            failures.push(format!("{name}: the export differs from the document"));
            continue;
        }
        let retranslated = translate(&again, &options).expect("valid options");
        if retranslated.ruleset != translation.ruleset
            || retranslated.definitions != translation.definitions
        {
            failures.push(format!("{name}: the export translates to other packages"));
            continue;
        }
        let Ok(model) = std::fs::read(case.with_extension("ifc")) else {
            continue;
        };
        let Ok(session) = import_ifc_session("model.ifc", &model) else {
            refused += 1;
            continue;
        };
        if verdicts(&translation, &session) == verdicts(&retranslated, &session) {
            identical += 1;
        } else {
            failures.push(format!("{name}: the export reports otherwise"));
        }
        let detached = one_by_one(&name, &translation, &session, &written);
        rules_total += detached.total;
        rules_read += detached.read;
        read_files.extend(detached.file);
        failures.extend(detached.failure);
    }
    println!(
        "round trip: {complete} complete translation(s), {} exported, {identical} with identical verdicts, {refused} model(s) refused",
        exported.len()
    );
    println!(
        "without their origin: {rules_read} of {rules_total} rule(s) read as specifications of their own"
    );
    exported.extend(read_files);
    validate(&exported, &mut failures);
    assert!(
        failures.is_empty(),
        "round-trip failures:\n{}",
        failures.join("\n")
    );
}

/// What reading a translation's rules one by one gave.
struct OneByOne {
    /// Rules in the translation.
    total: usize,
    /// Rules read as specifications of their own.
    read: usize,
    /// The document they were exported to.
    file: Option<PathBuf>,
    /// How they report otherwise than as translated.
    failure: Option<String>,
}

/// Exports the rules of `translation` without their origin, so each is
/// read as a specification of its own, and checks that every rule read
/// reports on `session` as it did.
fn one_by_one(
    name: &str,
    translation: &axioval_ids::Translation,
    session: &axioval::engine::EvidenceSession,
    written: &Path,
) -> OneByOne {
    let mut detached = translation.ruleset.clone();
    detached.root.annotations.clear();
    for folder in &mut detached.root.folders {
        folder.annotations.clear();
    }
    let export = axioval_ids::export(std::slice::from_ref(&translation.definitions), &detached);
    if std::env::var_os("IDS_CORPUS_VERBOSE").is_some() {
        for entry in &export.not_exported {
            println!("    {name}: not read on its own: {entry}");
        }
    }
    let mut outcome = OneByOne {
        total: export.not_exported.len() + export.specifications.len(),
        read: export.specifications.len(),
        file: None,
        failure: None,
    };
    let Some(xml) = export.to_xml() else {
        return outcome;
    };
    let path = written.join(format!("{name}.rules.ids"));
    std::fs::write(&path, &xml).expect("writable export");
    outcome.file = Some(path);
    let back = translate(
        &openbim_ids::from_str(&xml).expect("the export reads"),
        &Options::new("ids:corpus", "1.0.0"),
    )
    .expect("valid options");
    // Specification n is the n-th rule read.
    let origin: BTreeMap<String, String> = back
        .specifications
        .iter()
        .flat_map(|specification| {
            let rule = export.specifications[specification.number - 1].rules[0].clone();
            specification
                .rules
                .iter()
                .map(move |id| (id.clone(), rule.clone()))
        })
        .collect();
    let read: Vec<&str> = export
        .specifications
        .iter()
        .map(|specification| specification.rules[0].as_str())
        .collect();
    let original: Vec<String> = subjects(translation, session, &|id| id.to_owned())
        .into_iter()
        .filter(|verdict| {
            read.iter()
                .any(|rule| verdict.starts_with(&format!("{rule} ")))
        })
        .collect();
    if original != subjects(&back, session, &|id| origin[id].clone()) {
        outcome.failure = Some(format!("{name}: rules read one by one report otherwise"));
    }
    outcome
}

/// Validates every exported document against `ids.xsd` of the corpus
/// checkout, with Python's `lxml`: the schema is CC BY-ND 4.0 like the
/// corpus, and not vendored.
fn validate(files: &[PathBuf], failures: &mut Vec<String>) {
    let cases = PathBuf::from(std::env::var_os("IDS_TEST_CASES").expect("IDS_TEST_CASES"));
    let schema = cases.join("../../../Schema/ids.xsd");
    assert!(
        schema.exists(),
        "no ids.xsd at {}; IDS_TEST_CASES must point into a checkout of buildingSMART/IDS",
        schema.display()
    );
    let script = "import sys\nfrom lxml import etree\nschema = etree.XMLSchema(etree.parse(sys.argv[1]))\nfor path in sys.argv[2:]:\n    if not schema.validate(etree.parse(path)):\n        print(path, schema.error_log.last_error)\n";
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(&schema)
        .args(files)
        .output()
        .expect("python3 runs");
    assert!(
        output.status.success(),
        "schema validation needs python3 with lxml: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let invalid = String::from_utf8_lossy(&output.stdout);
    failures.extend(
        invalid
            .lines()
            .map(|line| format!("invalid against ids.xsd: {line}")),
    );
    println!("{} export(s) validated against ids.xsd", files.len());
}

#[test]
#[ignore = "needs a local buildingSMART IDS checkout in IDS_TEST_CASES"]
fn corpus_round_trips_through_export() {
    round_trip();
}
