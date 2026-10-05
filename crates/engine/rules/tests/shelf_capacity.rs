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

mod common;

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
            include_subtypes: false,
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
            // Doors reach their spaces through `bounds`, forward.
            (
                "access_path".into(),
                ParameterValue::StringList {
                    value: vec!["bounds:forward".into()],
                },
            ),
            (
                "door_selector".into(),
                ParameterValue::Selector {
                    value: Box::new(Selector::EntityType {
                        object_type: "door".into(),
                        include_subtypes: false,
                    }),
                },
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
            // A room tall enough for the 2 m shelving.
            Answer::Measured(interval) => LinearQuantityEvidence::try_new(
                request.clone(),
                *interval,
                Evidence::exact(source(), "shelf:run"),
            )
            .map(|evidence| evidence.with_clear_height(LinearInterval::exact(3.0).unwrap())),
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

/// Answers every request with a length that falls by 10 m per door, a
/// clear height, and records the doors it was asked about.
struct Doors {
    height: LinearInterval,
    asked: std::sync::Mutex<Vec<Vec<ObjectId>>>,
}

impl LinearQuantityService for Doors {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        self.asked.lock().unwrap().push(request.doors().to_vec());
        let count = f64::from(u32::try_from(request.doors().len()).unwrap());
        LinearQuantityEvidence::try_new(
            request.clone(),
            LinearInterval::exact(20.0 - 10.0 * count)?,
            Evidence::exact(source(), "shelf:run"),
        )
        .map(|evidence| evidence.with_clear_height(self.height))
    }
}

/// Space `store` with door `d1` on its boundary and door `d2` elsewhere.
fn store() -> common::Model {
    common::Model::default()
        .object("store", "space")
        .object("other", "space")
        .object("d1", "door")
        .object("d2", "door")
        .edge("bounds", "d1", "store")
        .edge("bounds", "d2", "other")
}

fn with_doors(
    model: common::Model,
    height: LinearInterval,
) -> (axioval_engine::CapabilityEvaluation, Arc<Doors>) {
    let service = Arc::new(Doors {
        height,
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let registered = service.clone();
    let mut rule = rule();
    rule.selector = Selector::EntityType {
        object_type: "space".into(),
        include_subtypes: false,
    };
    let outcome = model.evaluate_with(&ShelfCapacity, &rule, move |services| {
        services
            .register(LinearQuantityServiceHandle::new(registered))
            .unwrap();
    });
    (outcome, service)
}

/// The doors `access_path` reaches a space from travel in its request; a
/// door of another space does not.
#[test]
fn the_doors_of_each_space_are_sent_with_its_request() {
    let (outcome, service) = with_doors(store(), LinearInterval::exact(3.0).unwrap());
    let asked = service.asked.lock().unwrap().clone();
    assert_eq!(asked, vec![vec![common::id("d2")], vec![common::id("d1")]]);
    assert!(outcome.findings().is_empty(), "{:?}", outcome.findings());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

/// A shortfall relates the doors whose clearances were taken out.
#[test]
fn a_shortfall_relates_the_doors_of_the_space() {
    let model = store().edge("bounds", "d2", "store");
    let (outcome, _) = with_doors(model, LinearInterval::exact(3.0).unwrap());
    assert_eq!(outcome.findings().len(), 1, "{:?}", outcome.findings());
    let finding = &outcome.findings()[0];
    assert!(finding.message.contains("0.000 below required 10.000"));
    assert_eq!(finding.related, vec![common::id("d1"), common::id("d2")]);
}

/// A space under the shelving's top elevation is too low for it, whatever
/// length fits.
#[test]
fn a_space_lower_than_the_shelving_is_too_low() {
    let (outcome, _) = with_doors(store(), LinearInterval::exact(1.5).unwrap());
    let too_low: Vec<_> = outcome
        .findings()
        .iter()
        .filter(|finding| {
            finding
                .message
                .starts_with("space too low for the shelving")
        })
        .collect();
    assert_eq!(too_low.len(), 2, "{:?}", outcome.findings());
    assert!(too_low[0].message.contains("clear height 1.5 m"));

    // A height that may or may not reach 2 m decides nothing.
    let (outcome, _) = with_doors(store(), LinearInterval::try_new(1.9, 2.1).unwrap());
    assert!(outcome.findings().is_empty());
    assert_eq!(outcome.not_evaluated_outcomes().len(), 2);
}

/// A door whose spaces cannot be read might open into any space, so no
/// space's shelving is measured without it.
#[test]
fn a_door_with_unreadable_spaces_leaves_every_space_not_evaluated() {
    let mut rule = rule();
    rule.selector = Selector::EntityType {
        object_type: "space".into(),
        include_subtypes: false,
    };
    rule.parameters.insert(
        "access_path".into(),
        ParameterValue::StringList {
            value: vec!["unknown:forward".into()],
        },
    );
    let outcome = store().evaluate_with(&ShelfCapacity, &rule, |services| {
        services
            .register(LinearQuantityServiceHandle::new(Arc::new(Doors {
                height: LinearInterval::exact(3.0).unwrap(),
                asked: std::sync::Mutex::new(Vec::new()),
            })))
            .unwrap();
    });
    assert!(outcome.findings().is_empty());
    assert_eq!(common::unevaluated(&outcome).len(), 2);
    assert!(
        outcome
            .not_evaluated_outcomes()
            .iter()
            .all(|outcome| outcome.reason() == &NotEvaluatedReason::IncompleteEvidence)
    );
}

/// Without `access_path` nothing says where the doors are.
#[test]
fn a_rule_without_an_access_path_is_an_invalid_declaration() {
    let mut rule = rule();
    rule.parameters.remove("access_path");
    let outcome = evaluate_with(
        Answer::Measured(LinearInterval::exact(12.0).unwrap()),
        &rule,
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::InvalidDeclaration
    );
}

/// An estimated run never becomes an exact measured value: the service
/// contract refuses it, so it is not measured at all. A measured run is
/// exact as its evidence.
#[test]
fn an_estimated_run_is_never_measured_exactly() {
    let name = "shelf_length;depth=0.4;horizontal=0.3;vertical=0.35;bottom=0.1;top=2;\
                clearance=0.9;doors=door;access=bounds:forward";
    let read = |answer: Answer| {
        let (project, mut services) = common::Model::default().object("store", "space").services();
        services
            .register(LinearQuantityServiceHandle::new(Arc::new(StubQuantities(
                answer,
            ))))
            .unwrap();
        common::measured_cited(&services, &project, &common::id("store"), name)
    };
    let run = LinearInterval::exact(4.0).unwrap();
    assert!(read(Answer::Inexact(run)).is_err());
    assert_eq!(read(Answer::Measured(run)), Ok(Some(((4.0, 4.0), true))));
}

/// `shelf_clear_height` reaching the shelving's top and `shelf_length` the
/// minimum reach `shelf-capacity`'s verdicts, from the same request with the
/// same doors.
mod as_expressions {
    use super::*;
    use axioval_rules::ExpressionRequirement;
    use common::expressions::{and, assert_parity_uncounted, at_least, graded, m, measured};
    use serde_json::Value;

    const SHELVING: &str = "depth=0.4;horizontal=0.3;vertical=0.35;bottom=0.1;top=2;\
                            clearance=0.9;doors=door";

    fn requirement(access: &str, minimum: f64) -> Value {
        let value = |name: &str| measured(&format!("{name};{SHELVING};access={access}"));
        and(vec![
            at_least(value("shelf_clear_height"), m(2.0)),
            at_least(value("shelf_length"), m(minimum)),
        ])
    }

    fn parity(
        model: fn() -> common::Model,
        service: &dyn Fn() -> Arc<dyn LinearQuantityService>,
        capability: &CompiledRule,
        access: &str,
        minimum: f64,
    ) {
        let found = model().evaluate_with(&ShelfCapacity, capability, |services| {
            services
                .register(LinearQuantityServiceHandle::new(service()))
                .unwrap();
        });
        let rewritten = model().evaluate_measured(
            &ExpressionRequirement,
            &graded(
                capability.selector.clone(),
                &requirement(access, minimum),
                RuleSeverity::Warning,
            ),
            |services| {
                services
                    .register(LinearQuantityServiceHandle::new(service()))
                    .unwrap();
            },
        );
        // The capability reports each short door, the rewrite the space once
        // (divergence D1).
        assert_parity_uncounted("axioval:capability.shelf-capacity", &found, &rewritten);
    }

    fn lone_store() -> common::Model {
        common::Model::default().object("store", "space")
    }

    #[test]
    fn the_running_metres_and_the_clear_height_reach_the_verdicts() {
        let answers: [fn() -> Answer; 6] = [
            || Answer::Measured(LinearInterval::exact(12.0).unwrap()),
            || Answer::Measured(LinearInterval::exact(4.0).unwrap()),
            || Answer::Measured(LinearInterval::try_new(9.0, 11.0).unwrap()),
            || Answer::Measured(LinearInterval::try_new(2.0, 3.0).unwrap()),
            || Answer::Inexact(LinearInterval::exact(4.0).unwrap()),
            || Answer::Failed(LinearQuantityError::Unavailable),
        ];
        for answer in answers {
            for minimum in [10.0, 3.0] {
                parity(
                    lone_store,
                    &|| Arc::new(StubQuantities(answer())),
                    &rule_with(ParameterValue::Number { value: minimum }),
                    "bounds:forward",
                    minimum,
                );
            }
        }
    }

    fn space_rule() -> CompiledRule {
        let mut rule = rule();
        rule.selector = Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: false,
        };
        rule
    }

    #[test]
    fn the_doors_of_each_space_and_its_height_reach_the_verdicts() {
        let heights: [LinearInterval; 3] = [
            LinearInterval::exact(3.0).unwrap(),
            LinearInterval::exact(1.5).unwrap(),
            LinearInterval::try_new(1.9, 2.1).unwrap(),
        ];
        for height in heights {
            let doors = move || -> Arc<dyn LinearQuantityService> {
                Arc::new(Doors {
                    height,
                    asked: std::sync::Mutex::new(Vec::new()),
                })
            };
            parity(store, &doors, &space_rule(), "bounds:forward", 10.0);
            parity(
                || store().edge("bounds", "d2", "store"),
                &doors,
                &space_rule(),
                "bounds:forward",
                10.0,
            );
        }
    }

    #[test]
    fn doors_with_unreadable_spaces_leave_every_space_open_alike() {
        let mut capability = space_rule();
        capability.parameters.insert(
            "access_path".into(),
            ParameterValue::StringList {
                value: vec!["unknown:forward".into()],
            },
        );
        parity(
            store,
            &|| {
                Arc::new(Doors {
                    height: LinearInterval::exact(3.0).unwrap(),
                    asked: std::sync::Mutex::new(Vec::new()),
                })
            },
            &capability,
            "unknown:forward",
            10.0,
        );
    }
}
