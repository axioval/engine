//! Built-in property capability tests against canonical schema fixtures.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CapabilityRegistry, CompiledRule, CompletePropertyAbsenceEvidence, PropertyRequest,
    PropertyResolution, PropertyResolutionError, PropertyResolutionService,
    PropertyResolutionServiceHandle, ResolvedProperty, RuleCapability, RuleContext, Runtime,
    ServiceRegistry, compile,
};
use axioval_ir::contract::{
    ComparisonOperator, ParameterValue, Selector, Severity as RuleSeverity,
};
use axioval_ir::{
    DefinitionPackage, Evidence, Object, ObjectId, Project, Property, PropertyValue, RuleId,
    RuleSetPackage, SourceId,
};
use axioval_rules::{BooleanPropertyEquals, PropertyPredicate, register_builtins};

fn packages() -> (DefinitionPackage, RuleSetPackage) {
    (
        serde_json::from_str(include_str!(
            "../../../fixtures/schema-v0.1.0/definitions.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!("../../../fixtures/schema-v0.1.0/ruleset.json")).unwrap(),
    )
}
fn source() -> SourceId {
    SourceId::new("cad", "native-model").unwrap()
}
fn object() -> Object {
    Object::new(
        ObjectId::new(source(), "wall-1").unwrap(),
        "axioval:example.ifc.wall",
    )
}
fn exact_property(set: &str, name: &str, value: PropertyValue) -> Property {
    Property::new(set, name, value)
        .unwrap()
        .with_evidence(Evidence::exact(source(), format!("native {set}.{name}")))
}

struct ExactProperties(Vec<Property>);
impl PropertyResolutionService for ExactProperties {
    fn resolve(
        &self,
        request: &PropertyRequest,
    ) -> Result<PropertyResolution, PropertyResolutionError> {
        if request.object_id() != &object().id {
            return Err(PropertyResolutionError::Unavailable(
                "object is outside the exact fixture".into(),
            ));
        }
        if let Some(property) = self.0.iter().find(|property| {
            property.name == request.property()
                && request
                    .property_set()
                    .is_none_or(|set| property.property_set == set)
        }) {
            return Ok(PropertyResolution::Present(ResolvedProperty::try_new(
                request.clone(),
                property.clone(),
            )?));
        }
        Ok(PropertyResolution::Absent(
            CompletePropertyAbsenceEvidence::try_new(
                request.clone(),
                Evidence::exact(source(), "complete native property table"),
            )
            .unwrap(),
        ))
    }
}

fn run(
    project: &Project,
    definitions: DefinitionPackage,
    rules: &RuleSetPackage,
    properties: Option<Vec<Property>>,
) -> axioval_ir::Report {
    let registry = register_builtins(CapabilityRegistry::new()).unwrap();
    let plan = compile(&registry, &[definitions], rules).unwrap();
    let runtime = if let Some(properties) = properties {
        let mut services = ServiceRegistry::new();
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(
                ExactProperties(properties),
            )))
            .unwrap();
        Runtime::new(registry).with_services(services)
    } else {
        Runtime::new(registry)
    };
    runtime.run(project, plan).unwrap()
}

#[test]
fn missing_property_service_is_not_a_false_violation_or_pass() {
    let (definitions, rules) = packages();
    let report = run(
        &Project::new(vec![object()]).unwrap(),
        definitions,
        &rules,
        None,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].reason,
        axioval_ir::NotEvaluatedReason::MissingService
    );
}

#[test]
fn exact_property_absence_reports_a_violation_with_proof() {
    let (definitions, rules) = packages();
    let report = run(
        &Project::new(vec![object()]).unwrap(),
        definitions,
        &rules,
        Some(vec![]),
    );
    assert_eq!(report.findings().len(), 1);
    assert_eq!(
        report.findings()[0].evidence[0].locator,
        "complete native property table"
    );
    assert!(report.not_evaluated().is_empty());
}

#[test]
fn property_exists_accepts_exact_non_ifc_source_evidence() {
    let (definitions, rules) = packages();
    let property = exact_property(
        "axioval:example.ifc.pset-wall-common",
        "axioval:example.ifc.reference",
        PropertyValue::String("EI60".into()),
    );
    let project = Project::new(vec![object().with_property(property.clone())]).unwrap();
    let report = run(&project, definitions, &rules, Some(vec![property]));
    assert!(report.findings().is_empty());
    assert!(report.not_evaluated().is_empty());
}

#[test]
fn property_applicability_uses_exact_resolution_without_silent_skip() {
    let (definitions, mut rules) = packages();
    rules.root.rules[0].applicability = Selector::Property {
        property_set: Some("Pset.Trigger".into()),
        property: "Enabled".into(),
        operator: ComparisonOperator::Exists,
        value: None,
    };
    let trigger = exact_property("Pset.Trigger", "Enabled", PropertyValue::Boolean(true));
    let project = Project::new(vec![object().with_property(trigger.clone())]).unwrap();
    let report = run(&project, definitions, &rules, Some(vec![trigger]));
    assert_eq!(report.findings().len(), 1);
    assert!(report.not_evaluated().is_empty());
}

#[test]
fn property_applicability_without_service_is_explicitly_not_evaluated() {
    let (definitions, mut rules) = packages();
    rules.root.rules[0].applicability = Selector::Property {
        property_set: Some("Pset.Trigger".into()),
        property: "Enabled".into(),
        operator: ComparisonOperator::Exists,
        value: None,
    };
    let report = run(
        &Project::new(vec![object()]).unwrap(),
        definitions,
        &rules,
        None,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
}

#[test]
fn malformed_property_selector_is_rejected_before_resolution() {
    let (definitions, mut rules) = packages();
    rules.root.rules[0].applicability = Selector::Property {
        property_set: Some("Pset.Trigger".into()),
        property: "Enabled".into(),
        operator: ComparisonOperator::Exists,
        value: Some(ParameterValue::Boolean { value: true }),
    };
    let report = run(
        &Project::new(vec![object()]).unwrap(),
        definitions,
        &rules,
        None,
    );
    assert!(report.findings().is_empty());
    assert_eq!(report.not_evaluated().len(), 1);
    assert_eq!(
        report.not_evaluated()[0].reason,
        axioval_ir::NotEvaluatedReason::InvalidDeclaration
    );
}

#[test]
fn info_severity_is_preserved_in_exact_absence_finding() {
    let (definitions, mut rules) = packages();
    rules.root.rules[0].severity = RuleSeverity::Info;
    let report = run(
        &Project::new(vec![object()]).unwrap(),
        definitions,
        &rules,
        Some(vec![]),
    );
    assert_eq!(report.findings()[0].severity, axioval_ir::Severity::Info);
}

fn predicate(actual: i64, operator: &str, expected: i64) -> axioval_engine::CapabilityEvaluation {
    let property = exact_property("Pset.Counter", "Count", PropertyValue::Integer(actual));
    let project = Project::new(vec![object().with_property(property.clone())]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(PropertyResolutionServiceHandle::new(Arc::new(
            ExactProperties(vec![property]),
        )))
        .unwrap();
    let rule = CompiledRule {
        id: RuleId::new("predicate").unwrap(),
        capability: "axioval:capability.property-predicate".into(),
        severity: RuleSeverity::Error,
        selector: Selector::All,
        parameters: BTreeMap::from([
            (
                "property_set".into(),
                ParameterValue::String {
                    value: "Pset.Counter".into(),
                },
            ),
            (
                "property".into(),
                ParameterValue::String {
                    value: "Count".into(),
                },
            ),
            (
                "operator".into(),
                ParameterValue::String {
                    value: operator.into(),
                },
            ),
            ("value".into(), ParameterValue::Integer { value: expected }),
        ]),
    };
    PropertyPredicate.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule,
    )
}

#[test]
fn integer_predicate_uses_the_declared_operator() {
    let passing = predicate(5, "greater_or_equal", 5);
    assert!(passing.findings().is_empty());
    assert!(passing.not_evaluated_outcomes().is_empty());

    let failing = predicate(4, "greater_or_equal", 5);
    assert_eq!(failing.findings().len(), 1);
    assert!(failing.not_evaluated_outcomes().is_empty());
}

#[test]
fn unsupported_integer_predicate_operator_is_not_evaluated() {
    let evaluation = predicate(5, "approximately", 5);
    assert!(evaluation.findings().is_empty());
    assert_eq!(evaluation.not_evaluated_outcomes().len(), 1);
    assert_eq!(
        evaluation.not_evaluated_outcomes()[0].reason(),
        &axioval_ir::NotEvaluatedReason::InvalidDeclaration
    );
}
fn boolean_equals(
    actual: Option<bool>,
    with_service: bool,
) -> axioval_engine::CapabilityEvaluation {
    let property = actual.map(|value| {
        exact_property(
            "Pset_DoorCommon",
            "HandicapAccessible",
            PropertyValue::Boolean(value),
        )
    });
    let project = Project::new(vec![
        property
            .as_ref()
            .map_or_else(object, |value| object().with_property(value.clone())),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    if with_service {
        services
            .register(PropertyResolutionServiceHandle::new(Arc::new(
                ExactProperties(property.into_iter().collect()),
            )))
            .unwrap();
    }
    let rule = CompiledRule {
        id: RuleId::new("boolean-equals").unwrap(),
        capability: "axioval:capability.property-value-equals".into(),
        severity: RuleSeverity::Error,
        selector: Selector::All,
        parameters: BTreeMap::from([
            (
                "property".into(),
                ParameterValue::PropertyReference {
                    property: "HandicapAccessible".into(),
                    property_set: Some("Pset_DoorCommon".into()),
                },
            ),
            ("expected".into(), ParameterValue::Boolean { value: true }),
        ]),
    };
    BooleanPropertyEquals.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule,
    )
}

#[test]
fn boolean_property_equals_uses_exact_values_and_absence() {
    let passing = boolean_equals(Some(true), true);
    assert!(passing.findings().is_empty());
    assert!(passing.not_evaluated_outcomes().is_empty());
    let false_value = boolean_equals(Some(false), true);
    assert_eq!(false_value.findings().len(), 1);
    let absent = boolean_equals(None, true);
    assert_eq!(absent.findings().len(), 1);
    assert_eq!(
        absent.findings()[0].evidence[0].locator,
        "complete native property table"
    );
}

#[test]
fn boolean_property_equals_without_service_is_not_evaluated() {
    let outcome = boolean_equals(Some(true), false);
    assert!(outcome.findings().is_empty());
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &axioval_ir::NotEvaluatedReason::MissingService
    );
}
