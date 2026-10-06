//! Rule and expression drafts checked within a ruleset context: every type
//! and binding refusal positioned in the draft, and a dry run's traces.
#![allow(missing_docs, clippy::needless_pass_by_value)]

mod common;

use axioval_engine::CapabilityRegistry;
use axioval_engine::draft::{Diagnostic, ExpressionTraces, validate_expression, validate_rule};
use axioval_ir::contract::PropertyValueKind;
use axioval_ir::{DefinitionPackage, PropertyValue, QuantityDimension};
use axioval_rules::register_builtins;
use common::Model;
use common::runtime::{definition, definitions, entity, rule, ruleset, run, session, snapshot};
use serde_json::{Value, json};

const EXPRESSION: &str = "axioval:capability.expression";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn vocabulary() -> DefinitionPackage {
    let mut package = definitions(
        &registry(),
        &[EXPRESSION],
        &["slab"],
        &["Cover", "Class"],
        &["Pset"],
    );
    let cover = package.properties.get_mut("t.Cover").unwrap();
    cover.value_kind = PropertyValueKind::Quantity;
    cover.unit_dimension = Some("length".into());
    package
}

fn property(name: &str) -> Value {
    json!({"kind": "property", "propertySet": "t.Pset", "property": name})
}

fn mm(value: f64) -> Value {
    json!({"kind": "literal", "value": {"type": "quantity", "value": value, "unit": "mm"}})
}

fn at_least(left: Value, right: Value) -> Value {
    json!({"kind": "compare", "operator": "greaterThanOrEquals", "left": left, "right": right})
}

fn cover_rule(id: &str, requirement: Value) -> Value {
    rule(
        id,
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "expression", "value": requirement}}),
        json!({}),
    )
}

/// One diagnostic of drafting `draft` into a context holding `cover`.
fn refused(draft: &Value) -> Diagnostic {
    let context = ruleset(vec![cover_rule(
        "cover",
        at_least(property("t.Cover"), mm(25.0)),
    )]);
    let validation = validate_rule(&registry(), &[vocabulary()], &[context], &draft.to_string());
    assert!(validation.plan.is_none(), "compiled: {draft}");
    let [diagnostic] = validation.diagnostics.as_slice() else {
        panic!("one diagnostic: {:?}", validation.diagnostics);
    };
    diagnostic.clone()
}

#[test]
fn a_valid_draft_compiles_with_its_context() {
    let context = ruleset(vec![cover_rule(
        "cover",
        at_least(property("t.Cover"), mm(25.0)),
    )]);
    let draft = cover_rule("thicker", at_least(property("t.Cover"), mm(40.0)));
    let validation = validate_rule(
        &registry(),
        &[vocabulary()],
        &[context.clone()],
        &draft.to_string(),
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    assert_eq!(validation.plan.unwrap().rules().len(), 2);
    // A draft with a context rule's id replaces it.
    let validation = validate_rule(
        &registry(),
        &[vocabulary()],
        &[context],
        &cover_rule("cover", at_least(property("t.Cover"), mm(30.0))).to_string(),
    );
    assert_eq!(validation.plan.unwrap().rules().len(), 1);
}

#[test]
fn a_type_error_is_positioned_at_its_node() {
    let text = json!({"kind": "literal", "value": {"type": "string", "value": "thick"}});
    let diagnostic = refused(&cover_rule(
        "draft",
        json!({"kind": "and", "operands": [
            at_least(property("t.Cover"), mm(25.0)),
            at_least(property("t.Cover"), text)]}),
    ));
    assert_eq!(diagnostic.code, "invalidExpression");
    assert_eq!(diagnostic.rule.as_deref(), Some("draft"));
    assert_eq!(diagnostic.parameter.as_deref(), Some("requirement"));
    assert!(
        diagnostic
            .path
            .as_deref()
            .unwrap()
            .starts_with("requirement.and[1]")
    );
    assert!(
        diagnostic
            .pointer
            .as_deref()
            .unwrap()
            .starts_with("/parameters/requirement/value/operands/1"),
        "{diagnostic:?}"
    );
}

/// A measured value naming a rule parameter the draft does not state is
/// refused at the read, positioned there.
#[test]
fn a_measured_reference_to_an_unstated_parameter_is_positioned_at_its_read() {
    let shelving = json!({"kind": "property", "propertySet": "axioval:measured", "property":
        "shelf_length;depth=0.4;horizontal=0.3;vertical=0.35;bottom=0.1;top=2;clearance=0.9;\
         access=bounds:forward;doors=@door_selector"});
    let metres =
        json!({"kind": "literal", "value": {"type": "quantity", "value": 10.0, "unit": "m"}});
    let diagnostic = refused(&cover_rule("draft", at_least(shelving, metres)));
    assert_eq!(diagnostic.code, "invalidExpression");
    assert!(
        diagnostic
            .message
            .contains("`@door_selector`, which the rule does not state"),
        "{diagnostic:?}"
    );
    assert!(
        diagnostic
            .pointer
            .as_deref()
            .unwrap()
            .starts_with("/parameters/requirement/value/left"),
        "{diagnostic:?}"
    );
}

/// A value of a source never names the anchor, since no object is
/// measured: the draft is refused at the read, naming the subject.
#[test]
fn a_value_of_a_source_naming_the_anchor_is_refused_at_its_read() {
    let declared = json!({"kind": "property", "propertySet": "axioval:measured", "property":
        "external_declarations;derivations=all-spaces;bounding=IfcSpace;objects=@anchor"});
    let one = json!({"kind": "literal", "value": {"type": "integer", "value": 1}});
    let diagnostic = refused(&cover_rule("draft", at_least(declared, one)));
    assert!(
        diagnostic.message.contains("measured for each source"),
        "{diagnostic:?}"
    );
    assert!(
        diagnostic
            .pointer
            .as_deref()
            .unwrap()
            .starts_with("/parameters/requirement/value"),
        "{diagnostic:?}"
    );
}

#[test]
fn a_unit_mismatch_names_the_comparison() {
    let area =
        json!({"kind": "literal", "value": {"type": "quantity", "value": 1.0, "unit": "m2"}});
    let diagnostic = refused(&cover_rule("draft", at_least(property("t.Cover"), area)));
    assert_eq!(diagnostic.code, "invalidExpression");
    assert_eq!(diagnostic.path.as_deref(), Some("requirement"));
    assert_eq!(
        diagnostic.pointer.as_deref(),
        Some("/parameters/requirement/value")
    );
    assert!(
        diagnostic.message.contains("m2") || diagnostic.message.contains("m²"),
        "{diagnostic:?}"
    );
}

#[test]
fn an_unknown_concept_is_found_in_the_draft_with_the_nearest_name() {
    let diagnostic = refused(&cover_rule(
        "draft",
        at_least(property("t.Cuver"), mm(25.0)),
    ));
    // An unknown concept within an expression is refused as the expression.
    assert_eq!(diagnostic.code, "invalidExpression", "{diagnostic:?}");
    assert_eq!(diagnostic.path.as_deref(), Some("requirement.compare.left"));
    assert_eq!(
        diagnostic.pointer.as_deref(),
        Some("/parameters/requirement/value/left")
    );
    assert_eq!(
        diagnostic.suggestion.as_deref(),
        Some("did you mean `t.Cover`?")
    );
}

#[test]
fn an_unknown_measured_value_suggests_a_registered_one() {
    let measured =
        json!({"kind": "property", "propertySet": "axioval:measured", "property": "slpoe"});
    let angle =
        json!({"kind": "literal", "value": {"type": "quantity", "value": 2.0, "unit": "deg"}});
    let diagnostic = refused(&cover_rule("draft", at_least(angle, measured)));
    assert!(
        diagnostic.code == "invalidMeasured" || diagnostic.code == "invalidExpression",
        "{diagnostic:?}"
    );
    assert!(diagnostic.pointer.is_some(), "{diagnostic:?}");
    if diagnostic.code == "invalidMeasured" {
        assert_eq!(
            diagnostic.suggestion.as_deref(),
            Some("did you mean `slope`?")
        );
    }
}

#[test]
fn binding_errors_point_at_the_rule_fields() {
    let mut unknown_definition = cover_rule("draft", at_least(property("t.Cover"), mm(25.0)));
    unknown_definition["definitionId"] = json!(format!("{}x", definition(EXPRESSION)));
    let diagnostic = refused(&unknown_definition);
    assert_eq!(diagnostic.code, "unknownDefinition");
    assert_eq!(diagnostic.pointer.as_deref(), Some("/definitionId"));
    assert!(diagnostic.suggestion.is_some());

    let misspelt = rule(
        "draft",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirment": {"type": "expression", "value": at_least(property("t.Cover"), mm(25.0))}}),
        json!({}),
    );
    let diagnostic = refused(&misspelt);
    assert!(
        matches!(
            diagnostic.code.as_str(),
            "unknownParameter" | "missingParameter"
        ),
        "{diagnostic:?}"
    );
    assert!(
        diagnostic
            .pointer
            .as_deref()
            .unwrap()
            .starts_with("/parameters")
    );
    assert!(diagnostic.suggestion.is_some(), "{diagnostic:?}");

    let wrong_type = rule(
        "draft",
        EXPRESSION,
        "error",
        entity("slab"),
        json!({"requirement": {"type": "string", "value": "cover at least 25 mm"}}),
        json!({}),
    );
    let diagnostic = refused(&wrong_type);
    assert_eq!(diagnostic.code, "invalidParameterType", "{diagnostic:?}");
    assert_eq!(
        diagnostic.pointer.as_deref(),
        Some("/parameters/requirement")
    );

    let mut bad_id = cover_rule("draft", at_least(property("t.Cover"), mm(25.0)));
    bad_id["id"] = json!("");
    let diagnostic = refused(&bad_id);
    assert_eq!(diagnostic.code, "invalidRuleId", "{diagnostic:?}");
    assert_eq!(diagnostic.pointer.as_deref(), Some("/id"));
}

#[test]
fn a_draft_that_does_not_parse_names_its_line() {
    let context = ruleset(vec![]);
    let validation = validate_rule(
        &registry(),
        &[vocabulary()],
        &[context.clone()],
        "{\n  \"id\": \"x\",\n  oops\n}",
    );
    assert_eq!(validation.diagnostics[0].code, "syntax");
    assert_eq!(validation.diagnostics[0].line, Some(3));
    let validation = validate_rule(&registry(), &[vocabulary()], &[context], "{\"id\": \"x\"}");
    assert_eq!(validation.diagnostics[0].code, "shape");
}

#[test]
fn an_expression_draft_is_positioned_within_itself() {
    let context = ruleset(vec![cover_rule(
        "cover",
        at_least(property("t.Cover"), mm(25.0)),
    )]);
    let text = json!({"kind": "literal", "value": {"type": "string", "value": "thick"}});
    let draft = json!({"kind": "or", "operands": [at_least(property("t.Cover"), mm(25.0)), at_least(property("t.Cover"), text)]});
    let validation = validate_expression(
        &registry(),
        &[vocabulary()],
        &[context.clone()],
        "cover",
        "requirement",
        &draft.to_string(),
    );
    let diagnostic = &validation.diagnostics[0];
    assert_eq!(diagnostic.code, "invalidExpression");
    assert!(
        diagnostic
            .pointer
            .as_deref()
            .unwrap()
            .starts_with("/operands/1"),
        "{diagnostic:?}"
    );
    let validation = validate_expression(
        &registry(),
        &[vocabulary()],
        &[context],
        "covr",
        "requirement",
        &at_least(property("t.Cover"), mm(25.0)).to_string(),
    );
    assert_eq!(validation.diagnostics[0].code, "unknownRule");
    assert_eq!(
        validation.diagnostics[0].suggestion.as_deref(),
        Some("did you mean `cover`?")
    );
}

#[test]
fn a_dry_run_traces_every_object() {
    let mut model = Model::default();
    for (local, cover) in [("s1", 0.045), ("s2", 0.020)] {
        model = model.object(local, "slab").value(
            local,
            "Pset",
            "Cover",
            PropertyValue::Quantity {
                value: cover,
                dimension: QuantityDimension::Length,
            },
        );
    }
    model = model.object("s3", "slab");
    let context = ruleset(vec![]);
    let draft = cover_rule(
        "cover",
        json!({"kind": "implies", "antecedent": {"kind": "isDefined", "operand": property("t.Cover")},
            "consequent": at_least(property("t.Cover"), mm(25.0))}),
    );
    let validation = validate_rule(&registry(), &[vocabulary()], &[context], &draft.to_string());
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let recorder = ExpressionTraces::new();
    let session = session(model)
        .with_host_service(recorder.clone(), &[snapshot()])
        .unwrap();
    let report = run(registry(), validation.plan.unwrap(), &session, |runtime| {
        runtime
    })
    .unwrap();
    assert_eq!(report.findings().len(), 1);
    let traces = recorder.take();
    let verdicts: Vec<(String, &str)> = traces
        .iter()
        .map(|trace| (trace.object.local_id.clone(), trace.verdict))
        .collect();
    assert_eq!(
        verdicts,
        [
            ("s1".to_owned(), "passed"),
            ("s2".to_owned(), "failed"),
            ("s3".to_owned(), "passed")
        ]
    );
    for trace in &traces {
        assert_eq!(trace.rule, "cover");
        assert!(!trace.trace.entries.is_empty());
        assert_eq!(
            trace.trace.entries[0].path.split('.').next(),
            Some("requirement")
        );
    }
}
