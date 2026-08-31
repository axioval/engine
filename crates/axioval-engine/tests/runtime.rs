//! Strict compiler contract tests.
#![allow(missing_docs)]

use axioval_engine::{
    CapabilityEvaluation, CapabilityRegistry, CompiledRule, EngineError, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, Runtime, compile,
};
use axioval_ir::{DefinitionPackage, Project, RuleSetPackage};

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
fn packages() -> (DefinitionPackage, RuleSetPackage) {
    (
        serde_json::from_str(include_str!(
            "../../../fixtures/schema-v0.1.0/definitions.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!("../../../fixtures/schema-v0.1.0/ruleset.json")).unwrap(),
    )
}
#[test]
fn canonical_packages_compile() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert_eq!(
        compile(&registry, &[definitions], &rules)
            .unwrap()
            .rules()
            .len(),
        1
    );
}
#[test]
fn compiler_fails_closed_for_missing_required_parameter() {
    let (definitions, mut rules) = packages();
    rules.root.rules[0].parameters.clear();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert!(compile(&registry, &[definitions], &rules).is_err());
}

#[test]
fn compiler_rejects_unsupported_definition_schema_version() {
    let (mut definitions, rules) = packages();
    definitions.schema_version = "999.0.0".into();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert!(compile(&registry, &[definitions], &rules).is_err());
}

#[test]
fn compiler_rejects_unsupported_ruleset_schema_version() {
    let (definitions, mut rules) = packages();
    rules.schema_version = "999.0.0".into();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();
    assert!(compile(&registry, &[definitions], &rules).is_err());
}

#[test]
fn runtime_rejects_capability_registry_drift() {
    let (definitions, rules) = packages();
    let compiler_registry = CapabilityRegistry::new().register(Stub).unwrap();
    let plan = compile(&compiler_registry, &[definitions], &rules).unwrap();

    let error = Runtime::new(CapabilityRegistry::new())
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap_err();
    assert!(matches!(error, EngineError::UnknownCapability(_)));
}

#[test]
fn compiler_rejects_duplicate_definition_package_ids() {
    let (definitions, rules) = packages();
    let duplicate = definitions.clone();
    let registry = CapabilityRegistry::new().register(Stub).unwrap();

    let error = compile(&registry, &[definitions, duplicate], &rules).unwrap_err();
    assert!(matches!(error, EngineError::DuplicateDefinitionPackage(_)));
}

struct Unavailable;
impl RuleCapability for Unavailable {
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
        let mut evaluation = CapabilityEvaluation::default();
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, "z diagnostic");
        evaluation.push_not_evaluated(NotEvaluatedReason::MissingService, "a diagnostic");
        evaluation
    }
}

#[test]
fn runtime_reports_capability_unavailability_without_false_pass() {
    let (definitions, rules) = packages();
    let registry = CapabilityRegistry::new().register(Unavailable).unwrap();
    let plan = compile(&registry, &[definitions], &rules).unwrap();
    let report = Runtime::new(registry)
        .run(&Project::new(vec![]).unwrap(), plan)
        .unwrap();
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 2);
    assert_eq!(report.not_evaluated()[0].message, "a diagnostic");
    assert_eq!(report.not_evaluated()[1].message, "z diagnostic");
    assert_eq!(
        report.not_evaluated()[0].reason,
        NotEvaluatedReason::MissingService
    );
    assert_eq!(
        report.not_evaluated()[0].rule_id.to_string(),
        "wall-reference-required"
    );
}
