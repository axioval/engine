//! Results of external calculations (#323): a results catalogue of the
//! sound insulation index R'w per wall, keyed by the wall's stable key,
//! joined to the walls and compared with a requirement table, each result
//! traced to its method, report, date and the model revision it was
//! computed on; a result of another revision is stale and never read.
#![allow(missing_docs)]

mod common;

use axioval_engine::SuppliedCatalogue;
use axioval_engine::{CapabilityRegistry, EngineError, ExecutionPlan, ModelRevision, compile};
use axioval_ir::contract::ParameterDefinition;
use axioval_ir::{DefinitionPackage, NotEvaluatedReason, PropertyValue, Report};
use axioval_rules::register_builtins;
use common::Model;
use common::runtime::{definition, definitions, entity, rule, ruleset, run, session};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

/// The digest the checked model's file is taken to have.
const CHECKED: &str = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e";

/// R'w per wall from an invented calculation tool's report: W1 to W3
/// computed on the checked model, W4 on an earlier revision.
fn results_file() -> String {
    format!(
        "wall,R'w (dB),method,report,date,model revision\n\
         W1,54,ex-sound-method 2018,ex-acoustics report 12,2026-09-30,sha256:{CHECKED}\n\
         W2,51,ex-sound-method 2018,ex-acoustics report 12,2026-09-30,sha256:{CHECKED}\n\
         W3,48,ex-sound-method 2018,ex-acoustics report 12,2026-09-30,sha256:{CHECKED}\n\
         W4,58,ex-sound-method 2018,ex-acoustics report 11,2026-08-01,rev-2026-07\n"
    )
}

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn text(value: &str) -> Value {
    json!({ "default": value, "translations": {} })
}

/// Walls with a stable key and a use, and the expression definition
/// taking the requirement table `required` (use → least R'w).
fn vocabulary(registry: &CapabilityRegistry) -> DefinitionPackage {
    let mut package = definitions(
        registry,
        &[EXPRESSION],
        &["wall"],
        &["Key", "Use"],
        &["Pset"],
    );
    let required: ParameterDefinition = serde_json::from_value(json!({
    "id": "required", "name": text("required R'w"), "kind": "table", "required": true,
    "columns": [
        {"id": "use", "name": text("use"), "kind": "textPattern", "required": false},
        {"id": "rw", "name": text("R'w"), "kind": "number", "required": false},
    ]}))
    .unwrap();
    package
        .definitions
        .get_mut(&definition(EXPRESSION))
        .unwrap()
        .parameters
        .insert(required.id.clone(), required);
    package
}

/// The results catalogue `sound`, keyed by the wall's key.
fn sound() -> Value {
    json!({ "sound": {
        "id": "sound",
        "name": text("Sound insulation results"),
        "key": "wall",
        "columns": [
            {"id": "wall", "kind": "string"},
            {"id": "rw", "header": "R'w (dB)", "kind": "number"},
            {"id": "method", "kind": "string"},
            {"id": "report", "kind": "string"},
            {"id": "date", "kind": "date"},
            {"id": "revision", "header": "model revision", "kind": "string"},
        ],
        "results": {"method": "method", "report": "report", "date": "date",
                    "revision": "revision"},
    } })
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": format!("t.{name}")})
}

/// The wall's R'w, at least the requirement table's for its use.
fn requirement() -> Value {
    json!({"kind": "compare", "operator": "greaterThanOrEquals",
           "left": {"kind": "catalogue", "catalogue": "sound", "key": property("Key"),
                    "column": "rw", "label": "R'w"},
           "right": {"kind": "lookup", "table": "required", "column": "rw",
                     "keys": {"use": property("Use")}}})
}

fn requirement_rule() -> Value {
    let row = |pattern: &str, rw: f64| json!({"use": {"type": "string", "value": pattern}, "rw": {"type": "number", "value": rw}});
    rule(
        "sound",
        EXPRESSION,
        "error",
        entity("wall"),
        json!({"requirement": {"type": "expression", "value": requirement()},
               "required": {"type": "table",
                            "value": [row("apartment-separating", 53.0), row("*", 47.0)]}}),
        json!({}),
    )
}

fn compiled() -> Result<ExecutionPlan, EngineError> {
    let registry = registry();
    let mut set = ruleset(vec![requirement_rule()]);
    set.catalogues = serde_json::from_value(sound()).unwrap();
    compile(&registry, &[vocabulary(&registry)], &set)
}

/// Walls by local id, key and use.
fn walls(walls: &[(&str, &str, &str)]) -> Model {
    let mut model = Model::default();
    for &(local, key, used) in walls {
        model = model
            .object(local, "wall")
            .value(local, "Pset", "Key", PropertyValue::String(key.into()))
            .value(local, "Pset", "Use", PropertyValue::String(used.into()));
    }
    model
}

fn supplied(plan: &mut ExecutionPlan, revision: Option<ModelRevision>) {
    let definition = plan.catalogues()[0].clone();
    let file = results_file();
    let results =
        SuppliedCatalogue::read(&definition, file.as_bytes(), "rw-results.csv", None, None)
            .unwrap();
    plan.supply_catalogue("sound", results).unwrap();
    if let Some(revision) = revision {
        plan.supply_model_revision(common::source(), revision)
            .unwrap();
    }
}

fn check(plan: ExecutionPlan) -> Report {
    run(
        registry(),
        plan,
        &session(walls(&[
            ("w1", "W1", "apartment-separating"),
            ("w2", "W2", "apartment-separating"),
            ("w3", "W3", "stair-enclosure"),
            ("w4", "W4", "apartment-separating"),
            ("w5", "W5", "stair-enclosure"),
        ])),
        |runtime| runtime,
    )
    .unwrap()
}

fn findings(report: &Report) -> Vec<String> {
    let mut subjects: Vec<String> = report.findings().iter().map(common::subject).collect();
    subjects.sort();
    subjects
}

fn outcomes_open(report: &Report) -> Vec<(String, NotEvaluatedReason, String)> {
    let mut open: Vec<_> = report
        .not_evaluated
        .iter()
        .map(|outcome| {
            (
                outcome
                    .object_id()
                    .map(|object| object.local_id.clone())
                    .unwrap_or_default(),
                outcome.reason.clone(),
                outcome.message.clone(),
            )
        })
        .collect();
    open.sort_by(|a, b| a.0.cmp(&b.0));
    open
}

#[test]
fn results_joined_to_walls_are_compared_with_a_requirement_table() {
    let mut plan = compiled().unwrap();
    supplied(&mut plan, Some(ModelRevision::new(CHECKED, None).unwrap()));
    // W4 was computed on an earlier revision: stale before anything runs.
    let stale = plan.stale_results();
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!((stale[0].key.as_str(), stale[0].stale), ("W4", true));
    assert_eq!(stale[0].revision.as_deref(), Some("rev-2026-07"));
    let report = check(plan);
    // W1 54 ≥ 53 and W3 48 ≥ 47 meet the table; W2 51 < 53 does not.
    assert_eq!(findings(&report), ["w2"]);
    let finding = &report.findings()[0];
    assert!(
        finding.message.contains("catalogue sound[W2].rw = 51"),
        "{}",
        finding.message
    );
    // The value is the calculation's, traced to its method, report, date
    // and the revision it was computed on.
    let cited: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .filter(|locator| locator.starts_with("axioval:catalogue;"))
        .collect();
    assert_eq!(cited.len(), 1, "{:?}", finding.evidence);
    assert!(
        cited[0].starts_with("axioval:catalogue;id=sound;sha256=")
            && cited[0].ends_with(&format!(
                "#entry=W2;method=ex-sound-method 2018;report=ex-acoustics report 12;\
                 date=2026-09-30;revision=sha256:{CHECKED}"
            )),
        "{}",
        cited[0]
    );
    // W4's result is stale and never read; W5 has none.
    let open = outcomes_open(&report);
    assert_eq!(open.len(), 2, "{open:?}");
    assert_eq!(open[0].0, "w4");
    assert_eq!(open[0].1, NotEvaluatedReason::IncompleteEvidence);
    assert!(
        open[0]
            .2
            .contains("result `W4` of catalogue `sound` is stale")
            && open[0].2.contains("`rev-2026-07`")
            && open[0].2.contains(CHECKED),
        "{}",
        open[0].2
    );
    assert_eq!(open[1].0, "w5");
    assert!(
        open[1].2.contains("has no entry whose `wall` is `W5`"),
        "{}",
        open[1].2
    );
}

#[test]
fn a_revision_the_host_states_matches_and_an_unstated_one_matches_nothing() {
    // The host states the checked model is revision rev-2026-07 too.
    let mut plan = compiled().unwrap();
    supplied(
        &mut plan,
        Some(ModelRevision::new(CHECKED, Some("rev-2026-07".into())).unwrap()),
    );
    assert!(plan.stale_results().is_empty());
    let report = check(plan);
    assert_eq!(findings(&report), ["w2"]);
    let open = outcomes_open(&report);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].0, "w5");

    // No revision stated for the checked model: no result is read.
    let mut plan = compiled().unwrap();
    supplied(&mut plan, None);
    assert_eq!(plan.stale_results().len(), 4);
    let report = check(plan);
    assert!(report.findings().is_empty());
    let open = outcomes_open(&report);
    assert_eq!(open.len(), 5, "{open:?}");
    assert!(
        open[0].2.contains("the revision of the checked model") && open[0].2.contains("not stated"),
        "{}",
        open[0].2
    );
}

#[test]
fn a_results_declaration_is_checked_when_compiled() {
    let registry = registry();
    let mut set = ruleset(vec![requirement_rule()]);
    let mut declared = sound();
    declared["sound"]["results"]["date"] = json!("report");
    set.catalogues = serde_json::from_value(declared).unwrap();
    assert!(matches!(
        compile(&registry, &[vocabulary(&registry)], &set),
        Err(EngineError::InvalidCatalogue { detail, .. }) if detail.contains("not date or dateTime")
    ));
    let mut plan = compiled().unwrap();
    let revision = ModelRevision::new(CHECKED, None).unwrap();
    plan.supply_model_revision(common::source(), revision.clone())
        .unwrap();
    assert!(matches!(
        plan.supply_model_revision(common::source(), revision),
        Err(EngineError::InvalidModelRevision { .. })
    ));
}
