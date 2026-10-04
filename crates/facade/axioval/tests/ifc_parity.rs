//! Differential parity on private models: every case under
//! `AXIOVAL_PARITY_CASES` runs a capability rule and its expression rewrite
//! over each of its IFC models and must judge every object alike.
//!
//! The models are private, so this test is ignored by default and the gate
//! runs it only where a maintainer sets `AXIOVAL_PARITY_CASES`; once set, a
//! missing or unreadable case fails rather than skips. A case is a
//! directory holding `definitions.json` (a definition package),
//! `ruleset.json` (a ruleset), `parity.json`
//! (`{"pairs": [{"capability": "<rule id>", "expression": "<rule id>"}]}`)
//! and one or more `.ifc` models. Each pair's parity evidence is printed as
//! one JSON line, which is what a migration ledger records as proof.
#![cfg(feature = "ifc")]
#![allow(missing_docs)]

use std::path::{Path, PathBuf};

use axioval::engine::{CapabilityRegistry, Runtime, compile};
use axioval::ifc::import_ifc_session;
use axioval::ir::{DefinitionPackage, RuleSetPackage};
use axioval::rules::parity::compare;
use axioval::rules::register_builtins;
use serde_json::Value;

fn read(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("readable entry").path())
        .collect();
    entries.sort();
    entries
}

/// Every pair of one case over each of its models; the differences found.
fn run_case(case: &Path) -> Vec<String> {
    let definitions: DefinitionPackage =
        serde_json::from_value(read(&case.join("definitions.json"))).expect("definition package");
    let ruleset: RuleSetPackage =
        serde_json::from_value(read(&case.join("ruleset.json"))).expect("ruleset");
    let parity = read(&case.join("parity.json"));
    let pairs = parity["pairs"]
        .as_array()
        .expect("`pairs` lists rule pairs");
    assert!(!pairs.is_empty(), "{}: no pairs to compare", case.display());
    let models: Vec<PathBuf> = entries(case)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("ifc"))
        })
        .collect();
    assert!(!models.is_empty(), "{}: no .ifc models", case.display());
    let mut failures = Vec::new();
    for model in models {
        let registry = register_builtins(CapabilityRegistry::new()).expect("built-ins");
        let plan = compile(&registry, &[definitions.clone()], &ruleset).expect("ruleset compiles");
        let bytes = std::fs::read(&model).expect("readable model");
        let name = model.file_name().unwrap().to_string_lossy().into_owned();
        let session = import_ifc_session(&name, &bytes).expect("model imports");
        let report = Runtime::new(registry)
            .run_session(&session, plan)
            .expect("ruleset runs");
        for pair in pairs {
            let capability = pair["capability"].as_str().expect("`capability` rule id");
            let expression = pair["expression"].as_str().expect("`expression` rule id");
            let evidence = compare(&report, capability, expression);
            println!("{}", serde_json::to_string(&evidence).unwrap());
            if !evidence.holds() {
                failures.push(format!(
                    "{} {name}: {capability} vs {expression}\n{}",
                    case.display(),
                    evidence.diff()
                ));
            }
        }
    }
    failures
}

#[test]
#[ignore = "needs private parity cases in AXIOVAL_PARITY_CASES"]
fn private_models_hold_parity() {
    let root = PathBuf::from(
        std::env::var_os("AXIOVAL_PARITY_CASES")
            .expect("set AXIOVAL_PARITY_CASES to a directory of parity cases"),
    );
    let cases: Vec<PathBuf> = entries(&root)
        .into_iter()
        .filter(|path| path.is_dir())
        .collect();
    assert!(!cases.is_empty(), "{}: no parity cases", root.display());
    let failures: Vec<String> = cases.iter().flat_map(|case| run_case(case)).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
