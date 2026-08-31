//! Free-floor-rectangle capability contract tests.
#![allow(missing_docs)]

use std::{collections::BTreeMap, sync::Arc};

use axioval_engine::{
    ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest, ClearanceShape, CompiledRule,
    CompletePlacementEvidence, CompleteSupportEvidence, FreeAreaEvidence, FreeAreaRequest,
    FreeSpaceError, FreeSpaceService, FreeSpaceServiceHandle, MetricDirection, MetricFrame,
    MetricPoint, NotEvaluatedReason, PlacementDomain, PlacementOutcome, PlacementRequest,
    RuleCapability, RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::FreeFloorRectangle;

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn object(local: &str, kind: &str) -> Object {
    Object::new(ObjectId::new(source(), local).unwrap(), kind)
}
fn rule() -> CompiledRule {
    CompiledRule {
        id: RuleId::new("free-rectangle").unwrap(),
        capability: "axioval:capability.free-floor-rectangle".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: true,
        },
        parameters: BTreeMap::from([
            ("width_metres".into(), ParameterValue::Number { value: 1.8 }),
            (
                "length_metres".into(),
                ParameterValue::Number { value: 1.5 },
            ),
            (
                "height_metres".into(),
                ParameterValue::Number { value: 2.0 },
            ),
        ]),
    }
}

#[test]
fn missing_free_space_service_is_not_a_pass_or_violation() {
    let room = object("room", "space");
    let project = Project::new(vec![room.clone()]).unwrap();
    let services = ServiceRegistry::new();
    let outcome = FreeFloorRectangle.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    );
    assert!(outcome.findings().is_empty());
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);
    let unavailable = &outcome.not_evaluated_outcomes()[0];
    assert_eq!(unavailable.object_id(), Some(&room.id));
    assert_eq!(unavailable.reason(), &NotEvaluatedReason::MissingService);
}

#[derive(Clone, Copy)]
enum Answer {
    Found,
    NoPlacement,
    Unavailable,
    Incomplete,
    Invalid,
}
struct FakeService(Answer);
fn exact(locator: &str) -> Evidence {
    Evidence::exact(source(), locator)
}
fn placement_frame(request: &PlacementRequest) -> MetricFrame {
    MetricFrame::try_new(
        MetricPoint::try_new(request.scope().clone(), [0.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap(),
        MetricDirection::try_new([0.0, 0.0, 1.0]).unwrap(),
    )
    .unwrap()
}

impl FreeSpaceService for FakeService {
    fn assess_clearance(&self, _: &ClearanceRequest) -> Result<ClearanceOutcome, FreeSpaceError> {
        Err(FreeSpaceError::Unavailable("unused".into()))
    }
    fn measure_free_area(&self, _: &FreeAreaRequest) -> Result<FreeAreaEvidence, FreeSpaceError> {
        Err(FreeSpaceError::Unavailable("unused".into()))
    }
    fn find_placement(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementOutcome, FreeSpaceError> {
        assert_eq!(request.scope(), &ObjectId::new(source(), "room").unwrap());
        assert_eq!(
            request.obstacles(),
            &[ObjectId::new(source(), "chair").unwrap()]
        );

        let ClearanceShape::Box(shape) = request.shape() else {
            panic!("expected box")
        };
        assert!(shape.width_metres().total_cmp(&1.8).is_eq());
        assert!(shape.depth_metres().total_cmp(&1.5).is_eq());
        assert!(shape.height_metres().total_cmp(&2.0).is_eq());
        let PlacementDomain::Supported(support) = request.domain() else {
            panic!("free-floor placement must be supported")
        };
        assert_eq!(support.support(), request.scope());
        assert!(support.maximum_gap_metres().total_cmp(&0.0).is_eq());
        match self.0 {
            Answer::Found => {
                let frame = placement_frame(request);
                let support = CompleteSupportEvidence::try_new(
                    request.scope().clone(),
                    frame.clone(),
                    0.0,
                    exact("whole-base-support"),
                )
                .unwrap();
                Ok(PlacementOutcome::Found(
                    ClearancePlacementEvidence::try_new_supported(
                        request.clone(),
                        frame,
                        support,
                        exact("placement"),
                    )
                    .unwrap(),
                ))
            }
            Answer::NoPlacement => Ok(PlacementOutcome::NoPlacement(
                CompletePlacementEvidence::try_new(request.clone(), exact("complete-search"))
                    .unwrap(),
            )),
            Answer::Unavailable => Err(FreeSpaceError::Unavailable("kernel offline".into())),
            Answer::Incomplete => Err(FreeSpaceError::IncompleteClearanceEvidence),
            Answer::Invalid => Err(FreeSpaceError::InexactPlacementEvidence),
        }
    }
}

fn evaluate(answer: Answer) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![
        Object::new(ObjectId::new(source(), "room").unwrap(), "space"),
        Object::new(ObjectId::new(source(), "chair").unwrap(), "furniture"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(FreeSpaceServiceHandle::new(Arc::new(FakeService(answer))))
        .unwrap();
    FreeFloorRectangle.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    )
}

#[test]
fn exact_complete_no_placement_emits_native_rectangle_issue() {
    let outcome = evaluate(Answer::NoPlacement);
    assert!(outcome.not_evaluated_outcomes().is_empty());
    assert_eq!(outcome.findings().len(), 1);
    assert_eq!(
        outcome.findings()[0].message,
        "NO_FREE_FLOOR_SPACE_FOR_RECTANGLE"
    );
    assert_eq!(outcome.findings()[0].evidence[0].locator, "complete-search");
}

#[test]
fn exact_found_placement_passes_without_findings() {
    let outcome = evaluate(Answer::Found);
    assert!(outcome.findings().is_empty());
    assert!(outcome.not_evaluated_outcomes().is_empty());
}

#[test]
fn backend_unavailable_is_not_a_pass_or_violation() {
    let outcome = evaluate(Answer::Unavailable);
    assert!(outcome.findings().is_empty());
    assert_eq!(outcome.not_evaluated_outcomes().len(), 1);
    assert_eq!(
        outcome.not_evaluated_outcomes()[0].reason(),
        &NotEvaluatedReason::BackendUnavailable
    );
    assert_eq!(
        outcome.not_evaluated_outcomes()[0]
            .object_id()
            .unwrap()
            .local_id,
        "room"
    );
}

#[test]
fn unusable_proofs_remain_not_evaluated() {
    for (answer, expected) in [
        (Answer::Incomplete, NotEvaluatedReason::IncompleteEvidence),
        (Answer::Invalid, NotEvaluatedReason::InvalidEvidence),
    ] {
        let outcome = evaluate(answer);
        assert!(outcome.findings().is_empty());
        assert_eq!(outcome.not_evaluated_outcomes()[0].reason(), &expected);
    }
}
