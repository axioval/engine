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
//!   miss. So is one whose only gap is existence while the model does contain
//!   an applicable object.
//!
//! `invalid-` cases judge the IDS against the IFC schema and are skipped.
//! The corpus is CC BY-ND 4.0 and is not vendored: point `IDS_TEST_CASES` at
//! `Documentation/ImplementersDocumentation/TestCases` of
//! <https://github.com/buildingSMART/IDS> and run
//! `cargo test -- --ignored corpus`.
#![allow(missing_docs, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axioval::default_registry;
use axioval::engine::{Runtime, compile};
use axioval::ifc::import_ifc_session;
use axioval_ids::{Options, Part, Reason, Translation, translate};

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
    /// A fail case with no finding, explained by existence: the model has
    /// no applicable object and reports cannot say so.
    ExistenceFail,
    /// A fail case with no finding and gaps that may explain it.
    UnjudgedFail,
    /// The adapter refused the model, e.g. an IFC4X3 file.
    ModelRefused,
    /// Rules could not be evaluated; not a verdict either way.
    NotEvaluated,
    /// A false failure or an unexplained miss.
    Mismatch,
}

fn only_existence(translation: &Translation) -> bool {
    translation
        .gaps()
        .all(|(_, gap)| gap.part == Part::Occurrence && gap.reason == Reason::Existence)
}

/// Objects whose class the applicability names, counted independently of the
/// engine: the translation selects on entity names only.
fn applicable_objects(
    translation: &Translation,
    session: &axioval::engine::EvidenceSession,
) -> usize {
    let names: Vec<&str> = translation
        .definitions
        .object_types
        .values()
        .map(|concept| concept.name.default.as_str())
        .collect();
    session
        .project()
        .objects()
        .filter(|object| {
            names
                .iter()
                .any(|name| object.kind().eq_ignore_ascii_case(name))
        })
        .count()
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
    let options = Options {
        package_id: "ids:corpus".into(),
        version: "1.0.0".into(),
    };
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
    let detail = format!(
        "{} finding(s), {} not evaluated, gaps: [{}]",
        findings,
        report.not_evaluated().len(),
        translation
            .gaps()
            .map(|(_, gap)| gap.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    );
    let class = match (expected_pass, findings > 0) {
        (true, true) => Class::Mismatch,
        (false, true) => Class::CaughtFail,
        _ if !report.not_evaluated().is_empty() => Class::NotEvaluated,
        (true, false) if translation.is_complete() => Class::ExactPass,
        (true, false) => Class::SoundPass,
        (false, false) if translation.is_complete() => Class::Mismatch,
        (false, false) if only_existence(&translation) => {
            if applicable_objects(&translation, &session) == 0 {
                Class::ExistenceFail
            } else {
                Class::Mismatch
            }
        }
        (false, false) => Class::UnjudgedFail,
    };
    Some((class, detail))
}

fn run() {
    let mut classes: BTreeMap<Class, Vec<String>> = BTreeMap::new();
    for case in cases() {
        if let Some((class, detail)) = classify(&case) {
            let name = case.file_stem().unwrap().to_string_lossy().into_owned();
            classes
                .entry(class)
                .or_default()
                .push(format!("{name}: {detail}"));
        }
    }
    for (class, cases) in &classes {
        println!("{class:?}: {}", cases.len());
        if matches!(
            class,
            Class::CaughtFail
                | Class::ExactPass
                | Class::NotEvaluated
                | Class::ExistenceFail
                | Class::ModelRefused
        ) {
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
