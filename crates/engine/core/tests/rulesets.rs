//! Several rulesets compiled into one plan.
#![allow(missing_docs)]

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, CompiledRule, ConceptKind, EngineError,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, compile_rulesets,
};
use axioval_ir::{DefinitionPackage, RuleSetPackage};

struct Stub;
impl RuleCapability for Stub {
    fn id(&self) -> &'static str {
        "axioval:capability.property-exists"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![ParameterDescriptor::required(
            "property",
            ParameterType::PropertyReference,
        )]
    }
    fn evaluate(&self, _: &RuleContext<'_>, _: &CompiledRule) -> CapabilityEvaluation {
        CapabilityEvaluation::evaluated(vec![])
    }
}

fn definitions() -> DefinitionPackage {
    serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/definitions.json"
    ))
    .unwrap()
}

/// The example ruleset as package `package`, its one rule named `r1`.
fn ruleset(package: &str) -> RuleSetPackage {
    let mut ruleset: RuleSetPackage = serde_json::from_str(include_str!(
        "../../../../fixtures/schema-v0.1.0/ruleset.json"
    ))
    .unwrap();
    ruleset.package.id = package.into();
    ruleset.root.rules[0].id = "r1".into();
    ruleset
}

fn registry() -> CapabilityRegistry {
    CapabilityRegistry::new().register(Stub).unwrap()
}

#[test]
fn rule_ids_are_qualified_by_package_and_ordered_by_it() {
    let plan = compile_rulesets(
        &registry(),
        &[definitions()],
        &[ruleset("org.example.b"), ruleset("org.example.a")],
    )
    .unwrap();
    let ids: Vec<String> = plan
        .rules()
        .iter()
        .map(|rule| rule.id.to_string())
        .collect();
    assert_eq!(ids, ["org.example.a/r1", "org.example.b/r1"]);
    assert!(
        plan.concepts()
            .contains(ConceptKind::Property, "axioval:example.ifc.reference")
    );
}

#[test]
fn one_ruleset_keeps_its_ids() {
    let plan = compile_rulesets(&registry(), &[definitions()], &[ruleset("a")]).unwrap();
    assert_eq!(plan.rules()[0].id.to_string(), "r1");
}

#[test]
fn rulesets_must_be_given_and_distinct() {
    assert_eq!(
        compile_rulesets(&registry(), &[definitions()], &[]).err(),
        Some(EngineError::NoRuleSet)
    );
    assert_eq!(
        compile_rulesets(&registry(), &[definitions()], &[ruleset("a"), ruleset("a")]).err(),
        Some(EngineError::DuplicateRuleSet("a".into()))
    );
}

#[test]
fn each_ruleset_is_compiled_on_its_own_terms() {
    let mut broken = ruleset("b");
    broken.root.rules[0].parameters.clear();
    assert!(matches!(
        compile_rulesets(&registry(), &[definitions()], &[ruleset("a"), broken]),
        Err(EngineError::MissingParameter { .. })
    ));
}
