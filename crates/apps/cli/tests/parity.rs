//! The differential parity harness on public IFC models, through the real
//! binary: `axioval check --geometry --rule-status` runs each case's
//! ruleset over each model, and every pair of rules (a built-in capability
//! and its re-expression) must judge every scope alike, or differ exactly
//! as the case records.
//!
//! The models are openly licensed but not vendored: `fixtures/parity/
//! models.json` pins each by URL and SHA-256, and `scripts/parity_models.py
//! fetch` downloads them into a cache. This test is ignored by default and
//! runs where `AXIOVAL_PARITY_MODELS` names that cache (the gate's `test`
//! section, and its own CI job); once set, a missing or altered model fails
//! rather than skips.
//!
//! A case is a directory of `fixtures/parity/cases` holding
//! `definitions.json`, `ruleset.json` and `parity.json`:
//!
//! - `models`: `"*"` for every pinned model, or a list of their names;
//! - `geometry`: `false` to check without meshing (meshed by default);
//! - `pairs`: `{"capability": "<rule id>", "expression": "<rule id>"}`, with
//!   `"comparison": "contract"` for a template held to the whole outside
//!   contract (`outcomes` by default), `"uncounted": true` for a rewrite
//!   reporting one finding where the capability reports one per check, and
//!   `"values": {"<name>": <rounding>}` for measured values to compare;
//! - `recorded`: rules of a capability rebuilt as a template, each
//!   `{"rule": "<rule id>", "reason": "…"}` with a pair's `comparison`,
//!   `uncounted` and `values`. The rule's outcomes as the retired
//!   implementation reported them are stored per model in the case's
//!   `recorded/<model>.json` (the report restricted to those rules), and
//!   the rule as it runs now is compared with them, so the retired
//!   implementation's side outlives its code. `AXIOVAL_PARITY_RECORD=1`
//!   writes those files from the current run instead of comparing: run it
//!   once, before the implementation is retired, and review its output
//!   like any other change;
//! - `divergences`: every difference a pair or a recorded rule is known to
//!   show on a model, `{"model", "capability", "difference", "reason",
//!   "decision"}` (`"recorded"` naming the rule in place of
//!   `"capability"`), the difference as the harness prints it. A
//!   difference not recorded, or a recorded one no longer shown, fails the
//!   test.
//!
//! Each pair's evidence on each model prints as one JSON line.
#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use axioval::ir::Report;
use axioval::rules::parity::{Observations, Parity};
use serde_json::Value;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../fixtures/parity");

fn read(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}; fetch it with scripts/parity_models.py fetch",
            path.display()
        )
    });
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Every pinned model in `directory`, by name, its digest checked.
fn models(directory: &Path) -> Vec<(String, PathBuf)> {
    let manifest = read(&Path::new(FIXTURES).join("models.json"));
    manifest["models"]
        .as_array()
        .expect("`models` lists the pinned models")
        .iter()
        .map(|model| {
            let name = model["name"].as_str().expect("a model name").to_owned();
            let path = directory.join(&name);
            assert_eq!(
                sha256(&path),
                model["sha256"].as_str().expect("a pinned digest"),
                "{}: not the pinned model; fetch it again",
                path.display()
            );
            (name, path)
        })
        .collect()
}

/// The comparison a pair asks for.
fn comparison(pair: &Value) -> Parity {
    let mut parity = match pair["comparison"].as_str() {
        None | Some("outcomes") => Parity::outcomes(),
        Some("contract") => Parity::contract(),
        Some(other) => panic!("unknown comparison `{other}`"),
    };
    if pair["uncounted"].as_bool() == Some(true) {
        parity = parity.uncounted();
    }
    if let Some(values) = pair["values"].as_object() {
        for (name, step) in values {
            parity = parity.value(name, step.as_f64().expect("a rounding step"));
        }
    }
    parity
}

/// The report of the case's ruleset over `model`, meshed unless
/// `geometry` is false.
fn check(case: &Path, model: &Path, geometry: bool) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_axioval"));
    command.arg("check");
    if geometry {
        command.arg("--geometry");
    }
    let output = command
        .arg("--rule-status")
        .arg("--model")
        .arg(model)
        .arg("--definitions")
        .arg(case.join("definitions.json"))
        .arg("--ruleset")
        .arg(case.join("ruleset.json"))
        .env("SOURCE_DATE_EPOCH", "1790416800")
        .output()
        .unwrap();
    // 0: nothing found, 3: findings, 4: something not evaluated.
    assert!(
        matches!(output.status.code(), Some(0 | 3 | 4)),
        "{} over {}: {}\n{}",
        case.display(),
        model.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("a JSON result");
    serde_json::from_value(result["report"].clone()).expect("a report")
}

/// The part of `report` about `rules`: their findings, not-evaluated
/// outcomes, tables and summaries, as a case stores a retired
/// implementation's outcomes.
fn restricted(report: &Report, rules: &BTreeSet<&str>) -> Value {
    let mut value = serde_json::to_value(report).expect("a report serializes");
    let about = |entry: &Value| {
        entry["rule_id"]
            .as_str()
            .is_some_and(|rule| rules.contains(rule))
    };
    for field in ["findings", "not_evaluated", "tables", "rules"] {
        if let Some(entries) = value.get_mut(field).and_then(Value::as_array_mut) {
            entries.retain(about);
        }
    }
    if let Some(fields) = value.as_object_mut() {
        fields.remove("stale_decisions");
    }
    value
}

/// What a case's comparisons showed: every difference by model, side and
/// difference, and how many scopes each side's comparisons covered.
#[derive(Default)]
struct Tally {
    shown: BTreeSet<(String, String, String)>,
    covered: BTreeMap<String, usize>,
}

impl Tally {
    /// Prints `evidence` as one JSON line and counts it.
    fn add(&mut self, model: &str, side: &str, evidence: &axioval::rules::parity::ParityEvidence) {
        let mut line = serde_json::to_value(evidence).unwrap();
        line["model"] = model.into();
        println!("{line}");
        *self.covered.entry(side.to_owned()).or_default() += evidence.objects;
        for difference in &evidence.differences {
            self.shown
                .insert((model.to_owned(), side.to_owned(), difference.to_string()));
        }
    }
}

/// Writes the outcomes the recorded rules reported in `report` to `path`.
fn write_recording(path: &Path, report: &Report, stored: &[&Value]) {
    let rules: BTreeSet<&str> = stored
        .iter()
        .map(|entry| entry["rule"].as_str().expect("a recorded rule id"))
        .collect();
    std::fs::create_dir_all(path.parent().expect("a case directory"))
        .expect("the recordings are writable");
    let mut text =
        serde_json::to_string_pretty(&restricted(report, &rules)).expect("a recording serializes");
    text.push('\n');
    std::fs::write(path, text).expect("the recording is writable");
}

/// The rules a case compares with their recorded outcomes.
fn recorded_rules(parity: &Value) -> Vec<&Value> {
    parity["recorded"]
        .as_array()
        .map(|rules| rules.iter().collect())
        .unwrap_or_default()
}

/// Where a case stores the recorded outcomes on `model`.
fn recording(case: &Path, model: &str) -> PathBuf {
    case.join("recorded").join(format!("{model}.json"))
}

/// The divergences a case records, by model, side and difference.
fn divergences(case: &Path, parity: &Value) -> BTreeSet<(String, String, String)> {
    parity["divergences"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|divergence| {
            for field in ["reason", "decision"] {
                assert!(
                    divergence[field]
                        .as_str()
                        .is_some_and(|text| !text.is_empty()),
                    "{}: a divergence without a {field}",
                    case.display()
                );
            }
            let field = |name: &str| divergence[name].as_str().map(str::to_owned);
            let side = field("capability")
                .or_else(|| field("recorded").map(|rule| format!("{rule} (recorded)")))
                .expect("a divergence names its `capability` or `recorded` rule");
            (
                field("model").expect("a divergence names its model"),
                side,
                field("difference").expect("a divergence states its difference"),
            )
        })
        .collect()
}

/// Runs one case over every model it names; the failures.
fn run_case(case: &Path, models: &[(String, PathBuf)]) -> Vec<String> {
    let parity = read(&case.join("parity.json"));
    let named: Vec<&(String, PathBuf)> = match &parity["models"] {
        Value::String(all) if all == "*" => models.iter().collect(),
        Value::Array(names) => names
            .iter()
            .map(|name| {
                let name = name.as_str().expect("a model name");
                models
                    .iter()
                    .find(|(pinned, _)| pinned == name)
                    .unwrap_or_else(|| panic!("{}: {name} is not pinned", case.display()))
            })
            .collect(),
        other => panic!("{}: `models` is {other}", case.display()),
    };
    let pairs = parity["pairs"]
        .as_array()
        .expect("`pairs` lists rule pairs");
    let stored = recorded_rules(&parity);
    assert!(
        !pairs.is_empty() || !stored.is_empty(),
        "{}: nothing to compare",
        case.display()
    );
    let recording_now = std::env::var_os("AXIOVAL_PARITY_RECORD").is_some();
    let recorded = divergences(case, &parity);
    let geometry = parity["geometry"].as_bool().unwrap_or(true);
    let mut tally = Tally::default();
    for (name, model) in named {
        let report = check(case, model, geometry);
        for pair in pairs {
            let capability = pair["capability"].as_str().expect("`capability` rule id");
            let expression = pair["expression"].as_str().expect("`expression` rule id");
            let evidence = comparison(pair).compare(
                (capability, &Observations::of_report(&report, capability)),
                (expression, &Observations::of_report(&report, expression)),
            );
            tally.add(name, capability, &evidence);
        }
        if stored.is_empty() {
            continue;
        }
        let path = recording(case, name);
        if recording_now {
            write_recording(&path, &report, &stored);
            continue;
        }
        let before: Report = serde_json::from_value(read(&path))
            .unwrap_or_else(|error| panic!("{}: not a report: {error}", path.display()));
        for entry in &stored {
            let rule = entry["rule"].as_str().expect("a recorded rule id");
            let retired = format!("{rule} (recorded)");
            let evidence = comparison(entry).compare(
                (&retired, &Observations::of_report(&before, rule)),
                (rule, &Observations::of_report(&report, rule)),
            );
            tally.add(name, &retired, &evidence);
        }
    }
    if recording_now {
        return Vec::new();
    }
    let Tally { shown, covered } = tally;
    for (capability, objects) in &covered {
        assert!(
            *objects > 0,
            "{}: {capability} and its re-expression judged nothing on any model",
            case.display()
        );
    }
    let mut failures: Vec<String> = shown
        .difference(&recorded)
        .map(|(model, capability, difference)| {
            format!("{model} {capability}: not recorded: {difference}")
        })
        .collect();
    failures.extend(
        recorded
            .difference(&shown)
            .map(|(model, capability, difference)| {
                format!("{model} {capability}: recorded, no longer shown: {difference}")
            }),
    );
    failures
        .into_iter()
        .map(|failure| format!("{}: {failure}", case.file_name().unwrap().to_string_lossy()))
        .collect()
}

/// Every public case directory, sorted.
fn cases() -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(Path::new(FIXTURES).join("cases"))
        .expect("the public parity cases")
        .map(|entry| entry.expect("readable entry").path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no public parity cases");
    cases
}

/// Without the models: every case's packages compile against the current
/// registry, every pair, recorded rule and recorded divergence names rules
/// of its ruleset, and a case comparing recorded rules has a recording for
/// every model it names, so a capability changing its signature fails here
/// and not only in the job that fetches the models.
#[test]
fn the_public_cases_compile_and_name_their_rules() {
    let registry = axioval::default_registry().unwrap();
    for case in cases() {
        let definitions: axioval::ir::DefinitionPackage =
            serde_json::from_value(read(&case.join("definitions.json"))).expect("definitions");
        let ruleset: axioval::ir::RuleSetPackage =
            serde_json::from_value(read(&case.join("ruleset.json"))).expect("a ruleset");
        let plan = axioval::engine::compile(&registry, &[definitions], &ruleset)
            .unwrap_or_else(|error| panic!("{}: {error}", case.display()));
        let rules: BTreeSet<String> = plan
            .rules()
            .iter()
            .map(|rule| rule.id.to_string())
            .collect();
        let parity = read(&case.join("parity.json"));
        for pair in parity["pairs"].as_array().expect("pairs") {
            for side in ["capability", "expression"] {
                let rule = pair[side].as_str().expect("a rule id");
                assert!(rules.contains(rule), "{}: no rule {rule}", case.display());
            }
            comparison(pair);
        }
        for entry in recorded_rules(&parity) {
            let rule = entry["rule"].as_str().expect("a recorded rule id");
            assert!(rules.contains(rule), "{}: no rule {rule}", case.display());
            assert!(
                entry["reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty()),
                "{}: recorded rule {rule} without a reason",
                case.display()
            );
            comparison(entry);
        }
        if !recorded_rules(&parity).is_empty() {
            for model in case_models(&parity) {
                let path = recording(&case, &model);
                let _: Report = serde_json::from_value(read(&path))
                    .unwrap_or_else(|error| panic!("{}: not a report: {error}", path.display()));
            }
        }
        for divergence in parity["divergences"].as_array().expect("divergences") {
            let rule = divergence["capability"]
                .as_str()
                .or_else(|| divergence["recorded"].as_str())
                .expect("a rule id");
            assert!(rules.contains(rule), "{}: no rule {rule}", case.display());
        }
    }
}

/// The names of the pinned models a case names.
fn case_models(parity: &Value) -> Vec<String> {
    match &parity["models"] {
        Value::String(all) if all == "*" => {
            read(&Path::new(FIXTURES).join("models.json"))["models"]
                .as_array()
                .expect("`models` lists the pinned models")
                .iter()
                .map(|model| model["name"].as_str().expect("a model name").to_owned())
                .collect()
        }
        Value::Array(names) => names
            .iter()
            .map(|name| name.as_str().expect("a model name").to_owned())
            .collect(),
        other => panic!("`models` is {other}"),
    }
}

#[test]
#[ignore = "needs the public models in AXIOVAL_PARITY_MODELS (scripts/parity_models.py fetch)"]
fn public_models_hold_parity() {
    let directory = PathBuf::from(
        std::env::var_os("AXIOVAL_PARITY_MODELS")
            .expect("set AXIOVAL_PARITY_MODELS to the fetched public models"),
    );
    let models = models(&directory);
    let cases = cases();
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|case| run_case(case, &models))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
