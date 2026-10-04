//! The outside contract of every built-in capability, pinned: its id, its
//! parameters (name, package kind, requirement, whether per object or an
//! expression's text, a table's columns), whether it grades deviations and
//! whether it takes authored parameters. A capability rebuilt on shared
//! parts must keep this contract byte for byte, so a change to it shows in
//! review as a change to `tests/golden/descriptors.json`. Regenerate it with
//! `AXIOVAL_BLESS=1 cargo test -p axioval-rules --test descriptors` only for
//! a deliberate contract change.

use axioval_engine::{CapabilityRegistry, ParameterType};
use serde_json::{Value, json};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/descriptors.json");

fn kind(parameter_type: ParameterType) -> Value {
    match parameter_type {
        ParameterType::Table(columns) => json!({
            "kind": "table",
            "columns": columns
                .iter()
                .map(|column| json!({
                    "id": column.id,
                    "kind": column.kind.as_str(),
                    "required": column.required,
                }))
                .collect::<Vec<_>>(),
        }),
        ParameterType::Expression => json!({"kind": "expression", "type": "boolean"}),
        ParameterType::NumberExpression => json!({"kind": "expression", "type": "number"}),
        other => json!({"kind": other.package_kind()}),
    }
}

fn descriptors() -> String {
    let registry =
        axioval_rules::register_builtins(CapabilityRegistry::new()).expect("built-ins register");
    let capabilities: Vec<Value> = registry
        .ids()
        .map(|id| {
            let capability = registry.get(id).expect("a registered id");
            json!({
                "id": id,
                "gradesDeviation": capability.grades_deviation(),
                "takesAuthoredParameters": capability.takes_authored_parameters(),
                "parameters": capability
                    .parameters()
                    .into_iter()
                    .map(|parameter| {
                        let mut entry = kind(parameter.parameter_type);
                        entry["name"] = json!(parameter.name);
                        entry["required"] = json!(parameter.required);
                        entry["perObject"] = json!(parameter.per_object);
                        entry["expressionText"] = json!(parameter.expression_text);
                        entry
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&capabilities).expect("descriptors serialize");
    text.push('\n');
    text
}

#[test]
fn every_capability_keeps_its_outside_contract() {
    let text = descriptors();
    if std::env::var_os("AXIOVAL_BLESS").is_some() {
        std::fs::write(GOLDEN, &text).expect("the golden copy is writable");
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN).expect("the golden copy exists");
    assert!(
        text == golden,
        "a capability's outside contract changed; a rebuild must keep it. Regenerate \
         `tests/golden/descriptors.json` with `AXIOVAL_BLESS=1 cargo test -p axioval-rules \
         --test descriptors` only for a deliberate contract change"
    );
}
