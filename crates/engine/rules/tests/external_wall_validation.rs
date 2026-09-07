//! External-wall-validation capability contract tests.
//!
//! ADR 0004: the service measures envelope membership, the capability decides
//! whether the declaration agrees with it.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CompiledRule, EnvelopeMembershipError, EnvelopeMembershipEvidence, EnvelopeMembershipRequest,
    EnvelopeMembershipService, EnvelopeMembershipServiceHandle, NotEvaluatedReason, RuleCapability,
    RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::ExternalWallValidation;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn oid(local: &str) -> ObjectId {
    ObjectId::new(source(), local).unwrap()
}

fn rule_with(derivation: ParameterValue) -> CompiledRule {
    CompiledRule {
        id: RuleId::new("external-wall").unwrap(),
        capability: "axioval:capability.external-wall-validation".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "wall".into(),
            include_subtypes: true,
        },
        parameters: BTreeMap::from([("envelope_derivation".to_string(), derivation)]),
    }
}
fn rule() -> CompiledRule {
    rule_with(ParameterValue::String {
        value: "all-spaces".into(),
    })
}

struct Stub(Result<(Vec<ObjectId>, Vec<ObjectId>), EnvelopeMembershipError>);

impl EnvelopeMembershipService for Stub {
    fn measure_envelope_membership(
        &self,
        request: &EnvelopeMembershipRequest,
    ) -> Result<EnvelopeMembershipEvidence, EnvelopeMembershipError> {
        let (declared, derived) = self.0.clone()?;
        EnvelopeMembershipEvidence::try_new(
            *request,
            declared,
            derived,
            3,
            Evidence::exact(source(), "envelope:all-spaces"),
        )
    }
}

fn evaluate(stub: Stub, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![
        Object::new(oid("w1"), "wall"),
        Object::new(oid("w2"), "wall"),
        Object::new(oid("w3"), "wall"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(EnvelopeMembershipServiceHandle::new(Arc::new(stub)))
        .unwrap();
    ExternalWallValidation.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        rule,
    )
}

#[test]
fn matching_declaration_and_derivation_is_not_a_finding() {
    let outcome = evaluate(Stub(Ok((vec![oid("w1")], vec![oid("w1")]))), &rule());
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// Order and duplicates are an adapter artefact, not a discrepancy.
#[test]
fn agreement_is_order_and_duplicate_insensitive() {
    let outcome = evaluate(
        Stub(Ok((
            vec![oid("w2"), oid("w1"), oid("w2")],
            vec![oid("w1"), oid("w2")],
        ))),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
}

/// Each disagreeing wall is reported against itself, so a reviewer opens the
/// element rather than a whole-model message.
#[test]
fn disagreement_is_reported_per_element_in_both_directions() {
    let outcome = evaluate(
        Stub(Ok((vec![oid("w1"), oid("w2")], vec![oid("w2"), oid("w3")]))),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 2);

    let declared_only = outcome
        .findings()
        .iter()
        .find(|f| f.object_id == oid("w1"))
        .expect("w1 declared but not derived");
    assert!(declared_only.message.contains("not on the"));

    let derived_only = outcome
        .findings()
        .iter()
        .find(|f| f.object_id == oid("w3"))
        .expect("w3 derived but not declared");
    assert!(derived_only.message.contains("not declared"));

    // w2 agrees and must not be reported.
    assert!(outcome.findings().iter().all(|f| f.object_id != oid("w2")));
}

/// A model declaring nothing external while geometry finds walls is a real
/// discrepancy, not a vacuous pass.
#[test]
fn nothing_declared_against_derived_walls_is_a_finding() {
    let outcome = evaluate(Stub(Ok((Vec::new(), vec![oid("w1"), oid("w2")]))), &rule());
    assert_eq!(outcome.findings().len(), 2);
}

#[test]
fn every_finding_carries_its_evidence() {
    let outcome = evaluate(Stub(Ok((vec![oid("w1")], vec![oid("w2")]))), &rule());
    assert!(!outcome.findings().is_empty());
    for finding in outcome.findings() {
        assert_eq!(finding.evidence.len(), 1);
        assert!(finding.evidence[0].exact);
    }
}

#[test]
fn unsupported_derivation_is_incomplete_evidence_not_a_pass() {
    let outcome = evaluate(
        Stub(Err(EnvelopeMembershipError::UnsupportedDerivation)),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn unavailable_measurement_is_not_a_pass() {
    let outcome = evaluate(Stub(Err(EnvelopeMembershipError::Unavailable)), &rule());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let project = Project::new(vec![Object::new(oid("w1"), "wall")]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = ExternalWallValidation.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::MissingService
    );
}

#[test]
fn unknown_derivation_is_an_invalid_declaration() {
    let outcome = evaluate(
        Stub(Ok((Vec::new(), Vec::new()))),
        &rule_with(ParameterValue::String {
            value: "whole-building".into(),
        }),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}
