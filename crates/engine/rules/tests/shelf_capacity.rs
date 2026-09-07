//! Shelf-capacity capability contract tests.
//!
//! ADR 0004: the service measures, the capability decides. These pin the
//! decision half -- in particular that an ambiguous measurement is never
//! reported as a violation.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    CompiledRule, LinearInterval, LinearQuantityError, LinearQuantityEvidence,
    LinearQuantityRequest, LinearQuantityService, LinearQuantityServiceHandle, NotEvaluatedReason,
    RuleCapability, RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::ShelfCapacity;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn object(local: &str) -> Object {
    Object::new(ObjectId::new(source(), local).unwrap(), "space")
}
fn rule_with(minimum: ParameterValue) -> CompiledRule {
    CompiledRule {
        id: RuleId::new("shelf").unwrap(),
        capability: "axioval:capability.shelf-capacity".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: true,
        },
        parameters: BTreeMap::from([
            ("minimum_running_metres".into(), minimum),
            // A physically realisable arrangement; the measured run is stubbed,
            // so these only need to pass ShelfGeometry validation.
            (
                "shelf_depth_metres".into(),
                ParameterValue::Number { value: 0.4 },
            ),
            (
                "horizontal_spacing_metres".into(),
                ParameterValue::Number { value: 0.3 },
            ),
            (
                "vertical_spacing_metres".into(),
                ParameterValue::Number { value: 0.35 },
            ),
            (
                "bottom_elevation_metres".into(),
                ParameterValue::Number { value: 0.1 },
            ),
            (
                "top_elevation_metres".into(),
                ParameterValue::Number { value: 2.0 },
            ),
            (
                "door_clearance_metres".into(),
                ParameterValue::Number { value: 0.9 },
            ),
        ]),
    }
}
fn rule() -> CompiledRule {
    rule_with(ParameterValue::Number { value: 10.0 })
}

enum Answer {
    Measured(LinearInterval),
    Failed(LinearQuantityError),
    /// Evidence that is not exact -- an adapter trying to launder an estimate.
    Inexact(LinearInterval),
}

struct StubQuantities(Answer);

impl LinearQuantityService for StubQuantities {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        match &self.0 {
            Answer::Failed(error) => Err(*error),
            Answer::Measured(interval) => LinearQuantityEvidence::try_new(
                request.clone(),
                *interval,
                Evidence::exact(source(), "shelf:run"),
            ),
            Answer::Inexact(interval) => LinearQuantityEvidence::try_new(
                request.clone(),
                *interval,
                Evidence {
                    source: source(),
                    locator: "shelf:estimate".into(),
                    exact: false,
                },
            ),
        }
    }
}

fn evaluate_with(answer: Answer, rule: &CompiledRule) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![object("store")]).unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(LinearQuantityServiceHandle::new(Arc::new(StubQuantities(
            answer,
        ))))
        .unwrap();
    ShelfCapacity.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        rule,
    )
}

#[test]
fn measurement_clearing_the_minimum_is_not_a_finding() {
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(12.0).unwrap()),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn measurement_below_the_minimum_is_a_finding_carrying_its_evidence() {
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(4.0).unwrap()),
        &rule(),
    );
    assert_eq!(outcome.findings().len(), 1);
    let finding = &outcome.findings()[0];
    assert!(finding.message.contains("4.000"));
    assert!(finding.message.contains("10.000"));
    assert_eq!(finding.evidence.len(), 1, "a finding must carry its proof");
    assert!(finding.evidence[0].exact);
}

/// The reason the measurement is an interval: an approximate range that spans
/// the minimum has not been shown to fail. Reporting a violation there would
/// assert more than was measured.
#[test]
fn measurement_straddling_the_minimum_is_not_evaluated_rather_than_a_violation() {
    let straddling = LinearInterval::try_new(9.0, 11.0).unwrap();
    let outcome = evaluate_with(Answer::Measured(straddling), &rule());
    assert!(
        outcome.findings().is_empty(),
        "an ambiguous measurement must not become a violation"
    );
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

/// A range entirely below the minimum is a genuine failure even though it is
/// approximate -- fail-closed must not mean "never decide".
#[test]
fn interval_entirely_below_the_minimum_is_still_a_finding() {
    let below = LinearInterval::try_new(2.0, 3.0).unwrap();
    let outcome = evaluate_with(Answer::Measured(below), &rule());
    assert_eq!(outcome.findings().len(), 1);
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn inexact_evidence_is_refused_by_the_service_contract() {
    let outcome = evaluate_with(
        Answer::Inexact(LinearInterval::exact(4.0).unwrap()),
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidEvidence
    );
}

#[test]
fn unavailable_measurement_is_not_a_pass() {
    let outcome = evaluate_with(Answer::Failed(LinearQuantityError::Unavailable), &rule());
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::IncompleteEvidence
    );
}

#[test]
fn missing_service_is_neither_a_pass_nor_a_violation() {
    let project = Project::new(vec![object("store")]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = ShelfCapacity.evaluate(
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
fn non_numeric_or_negative_minimum_is_an_invalid_declaration() {
    for bad in [
        ParameterValue::String {
            value: "ten".into(),
        },
        ParameterValue::Number { value: -1.0 },
        ParameterValue::Number {
            value: f64::INFINITY,
        },
    ] {
        let outcome = evaluate_with(
            Answer::Measured(LinearInterval::exact(1.0).unwrap()),
            &rule_with(bad),
        );
        assert!(outcome.findings().is_empty());
        assert_eq!(
            outcome.not_evaluated_outcomes()[0].reason(),
            &NotEvaluatedReason::InvalidDeclaration
        );
    }
}

/// Geometry is a measurement input, not a threshold. An impossible arrangement
/// is a declaration defect, and must not reach an adapter.
#[test]
fn impossible_shelf_geometry_is_an_invalid_declaration() {
    let mut rule = rule();
    rule.parameters.insert(
        "top_elevation_metres".into(),
        ParameterValue::Number { value: 0.0 },
    );
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(50.0).unwrap()),
        &rule,
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}
