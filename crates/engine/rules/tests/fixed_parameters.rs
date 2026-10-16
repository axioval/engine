//! A rule definition that fixes a parameter (axioval/mcs#11): instances take
//! its default and may not state their own, and only a new definition, a
//! fork, changes it. Built on the MCS contract's own fixture.
#![allow(missing_docs)]

use axioval_engine::draft::validate_rule;
use axioval_engine::{CapabilityRegistry, EngineError, compile};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{DefinitionPackage, RuleSetPackage};
use axioval_rules::register_builtins;
use serde_json::json;

const DEFINITION: &str = "axioval:example.minimum-width";
const MESSAGE: &str = "rule `wall-reference-required`: parameters \
    ['requirement'] are fixed by definition 'axioval:example.minimum-width'; instances may \
    not state them, and changing them means forking the definition";

fn registry() -> CapabilityRegistry {
    register_builtins(CapabilityRegistry::new()).unwrap()
}

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../../../fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// The minimal example's packages beside the fixed-parameter fixture, with
/// the example's rule bound to the fixed definition and stating `minimum`
/// only, as the MCS binder's tests do.
fn documents() -> (RuleSetPackage, Vec<DefinitionPackage>) {
    let mut fixed: DefinitionPackage =
        serde_json::from_str(&fixture("fixed-parameter/definitions.json")).unwrap();
    // The engine binds a definition to the capability's whole signature,
    // its optional parameters included, which the MCS fixture leaves out.
    let parameters = &mut fixed.definitions.get_mut(DEFINITION).unwrap().parameters;
    for (id, kind) in [("deviation", "expression"), ("message", "string")] {
        let parameter = json!({"id": id, "name": {"default": id, "translations": {}},
                               "kind": kind, "required": false});
        parameters.insert(id.into(), serde_json::from_value(parameter).unwrap());
    }
    let minimal: DefinitionPackage =
        serde_json::from_str(&fixture("schema-v0.1.0/definitions.json")).unwrap();
    let mut ruleset: serde_json::Value =
        serde_json::from_str(&fixture("schema-v0.1.0/ruleset.json")).unwrap();
    ruleset["definitionPackages"]
        .as_array_mut()
        .unwrap()
        .push(json!(fixed.package.id));
    let rule = &mut ruleset["root"]["rules"][0];
    rule.as_object_mut().unwrap().remove("explanatoryImages");
    rule["definitionId"] = json!(DEFINITION);
    rule["parameters"] = json!({"minimum": {"type": "number", "value": 0.9}});
    (
        serde_json::from_value(ruleset).unwrap(),
        vec![minimal, fixed],
    )
}

fn requirement(
    definitions: &mut [DefinitionPackage],
) -> &mut axioval_ir::contract::ParameterDefinition {
    definitions[1]
        .definitions
        .get_mut(DEFINITION)
        .unwrap()
        .parameters
        .get_mut("requirement")
        .unwrap()
}

#[test]
fn instances_inherit_a_fixed_default_even_when_it_is_required() {
    let (ruleset, mut definitions) = documents();
    let fixed = requirement(&mut definitions).clone();
    assert!(fixed.fixed && fixed.required);
    let plan = compile(&registry(), &definitions, &ruleset).unwrap();
    let [rule] = plan.rules() else {
        panic!("one rule: {:?}", plan.rules());
    };
    assert_eq!(
        Some(&rule.parameters["requirement"]),
        fixed.default_value.as_ref()
    );
}

#[test]
fn an_instance_stating_a_fixed_parameter_is_refused() {
    let (mut ruleset, mut definitions) = documents();
    // Even the default itself: stating it at all replaces the template's.
    let stated = requirement(&mut definitions).default_value.clone().unwrap();
    ruleset.root.rules[0]
        .parameters
        .insert("requirement".into(), stated);
    let error = compile(&registry(), &definitions, &ruleset).unwrap_err();
    assert_eq!(
        error,
        EngineError::FixedParameter {
            rule: ruleset.root.rules[0].id.clone(),
            definition: DEFINITION.into(),
            parameters: vec!["requirement".into()],
        }
    );
    assert_eq!(error.to_string(), MESSAGE);
}

#[test]
fn the_draft_diagnostic_points_at_the_fixed_parameter() {
    let (ruleset, definitions) = documents();
    let mut draft = serde_json::to_value(&ruleset.root.rules[0]).unwrap();
    draft["parameters"]["requirement"] = json!({"type": "expression", "value": {"kind": "literal", "value": {"type": "boolean", "value": true}}});
    let validation = validate_rule(&registry(), &definitions, &[ruleset], &draft.to_string());
    assert!(validation.plan.is_none());
    let [diagnostic] = validation.diagnostics.as_slice() else {
        panic!("one diagnostic: {:?}", validation.diagnostics);
    };
    assert_eq!(diagnostic.code, "fixedParameter");
    assert_eq!(diagnostic.message, MESSAGE);
    assert_eq!(diagnostic.parameter.as_deref(), Some("requirement"));
    assert_eq!(
        diagnostic.pointer.as_deref(),
        Some("/parameters/requirement")
    );
}

#[test]
fn a_fixed_parameter_without_a_default_is_refused() {
    let (ruleset, mut definitions) = documents();
    requirement(&mut definitions).default_value = None;
    let error = compile(&registry(), &definitions, &ruleset).unwrap_err();
    assert_eq!(
        error,
        EngineError::FixedWithoutDefault {
            definition: DEFINITION.into(),
            parameter: "requirement".into(),
        }
    );
    assert!(
        error
            .to_string()
            .contains("a fixed parameter requires a defaultValue")
    );
    // Refused with its package, whether or not a rule uses the definition.
    let (_, mut definitions) = documents();
    requirement(&mut definitions).default_value = None;
    let mut ruleset: RuleSetPackage =
        serde_json::from_str(&fixture("schema-v0.1.0/ruleset.json")).unwrap();
    ruleset
        .definition_packages
        .push(definitions[1].package.id.clone());
    assert!(matches!(
        compile(&registry(), &definitions, &ruleset),
        Err(EngineError::FixedWithoutDefault { .. })
    ));
}

#[test]
fn a_fork_may_change_or_unfix_the_parameter() {
    let changed: ParameterValue = serde_json::from_value(json!({"type": "expression", "value":
        {"kind": "compare", "operator": "greaterThan",
         "left": {"kind": "property", "property": "axioval:example.width"},
         "right": {"kind": "parameter", "name": "minimum"}}}))
    .unwrap();
    for fixed in [true, false] {
        let (mut ruleset, mut definitions) = documents();
        let mut fork = definitions[1].definitions[DEFINITION].clone();
        fork.id = format!("{DEFINITION}-fork");
        let parameter = fork.parameters.get_mut("requirement").unwrap();
        parameter.fixed = fixed;
        parameter.default_value = Some(changed.clone());
        definitions[1]
            .definitions
            .insert(fork.id.clone(), fork.clone());
        let rule = &mut ruleset.root.rules[0];
        rule.definition_id = fork.id;
        if !fixed {
            rule.parameters
                .insert("requirement".into(), changed.clone());
        }
        let plan = compile(&registry(), &definitions, &ruleset).unwrap();
        assert_eq!(
            plan.rules()[0].parameters["requirement"],
            changed,
            "fixed: {fixed}"
        );
    }
}
