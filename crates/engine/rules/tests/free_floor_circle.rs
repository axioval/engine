//! Free-floor-circle capability contract tests.
#![allow(missing_docs)]

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axioval_engine::{
    AreaInterval, ClearanceOutcome, ClearancePlacementEvidence, ClearanceRequest, CompiledRule,
    CompletePlacementEvidence, CompleteSupportEvidence, FreeAreaEvidence, FreeAreaRequest,
    FreeFloorEvidence, FreeSpaceError, FreeSpaceService, FreeSpaceServiceHandle,
    LargestDiscEstimate, MetricDirection, MetricFrame, MetricPoint, NotEvaluatedReason,
    PlacementDomain, PlacementOutcome, PlacementRequest, PlacementShape, RuleCapability,
    RuleContext, ServiceRegistry,
};
use axioval_ir::contract::{ParameterValue, Selector, Severity as RuleSeverity};
use axioval_ir::{Evidence, Object, ObjectId, Project, RuleId, SourceId};
use axioval_rules::FreeFloorCircle;

mod common;

/// `FreeFloorCircle` as it runs, held to the implementation it replaced on every
/// evaluation.
static HELD: common::Held =
    common::Held(&FreeFloorCircle, &axioval_rules::reference::FreeFloorCircle);

fn source() -> SourceId {
    SourceId::new("cad", "model").unwrap()
}
fn object(local: &str, kind: &str) -> Object {
    Object::new(ObjectId::new(source(), local).unwrap(), kind)
}
fn rule() -> CompiledRule {
    CompiledRule {
        id: RuleId::new("free-circle").unwrap(),
        capability: "axioval:capability.free-floor-circle".into(),
        severity: RuleSeverity::Warning,
        selector: Selector::EntityType {
            object_type: "space".into(),
            include_subtypes: false,
        },
        parameters: BTreeMap::from([
            (
                "diameter_metres".into(),
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
    let outcome = HELD.evaluate(
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

/// A placement witnessed at the frame's origin.
fn found(request: &PlacementRequest) -> ClearancePlacementEvidence {
    let frame = placement_frame(request);
    let support = CompleteSupportEvidence::try_new(
        request.scope().clone(),
        frame.clone(),
        0.0,
        exact("whole-base-support"),
    )
    .unwrap();
    ClearancePlacementEvidence::try_new_supported(
        request.clone(),
        frame,
        support,
        exact("placement"),
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
    fn measure_free_floor(
        &self,
        request: &PlacementRequest,
    ) -> Result<FreeFloorEvidence, FreeSpaceError> {
        FreeFloorEvidence::try_new(
            request.clone(),
            AreaInterval::exact(6.25)?,
            request.obstacles().to_vec(),
            Vec::new(),
            exact("free-floor"),
        )
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

        let PlacementShape::Cylinder(shape) = request.shape() else {
            panic!("expected cylinder")
        };
        // The declared circle, or a smaller one the largest free circle is
        // bracketed with (#391).
        assert!(shape.radius_metres() <= 0.75);
        assert!(shape.height_metres().total_cmp(&2.0).is_eq());
        let PlacementDomain::Supported(support) = request.domain() else {
            panic!("free-floor placement must be supported")
        };
        assert_eq!(support.support(), request.scope());
        assert!(support.maximum_gap_metres().total_cmp(&0.0).is_eq());
        match self.0 {
            // A proven absence of the declared circle leaves a circle of
            // 1.12 m.
            Answer::NoPlacement if shape.radius_metres() <= 0.56 => {
                Ok(PlacementOutcome::Found(found(request)))
            }
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
    evaluate_with(Arc::new(FakeService(answer)))
}

fn evaluate_with(service: Arc<dyn FreeSpaceService>) -> axioval_engine::CapabilityEvaluation {
    let project = Project::new(vec![
        Object::new(ObjectId::new(source(), "room").unwrap(), "space"),
        Object::new(ObjectId::new(source(), "chair").unwrap(), "furniture"),
    ])
    .unwrap();
    let mut services = ServiceRegistry::new();
    services
        .register(FreeSpaceServiceHandle::new(service))
        .unwrap();
    services.register(common::natures(source())).unwrap();
    HELD.evaluate(
        &RuleContext {
            project: &project,
            services: &services,
        },
        &rule(),
    )
}

#[test]
fn exact_complete_no_placement_emits_native_circle_issue() {
    let outcome = evaluate(Answer::NoPlacement);
    assert!(outcome.not_evaluated_outcomes().is_empty());
    assert_eq!(outcome.findings().len(), 1);
    let finding = &outcome.findings()[0];
    // The proof's code, then what the absence leaves of the floor (#391):
    // a circle of 1.12 m placed, one of 1.125 m proven absent, the free
    // floor and the chair taking it.
    assert_eq!(
        finding.message,
        "NO_FREE_FLOOR_SPACE_FOR_CIRCLE: the largest free circle is between 1.12 m and \
         1.125 m across, centred at (0 m, 0 m, 0 m); required 1.5 m; the free floor is 6.25 m²"
    );
    assert_eq!(finding.related, [ObjectId::new(source(), "chair").unwrap()]);
    let locators: Vec<&str> = finding
        .evidence
        .iter()
        .map(|evidence| evidence.locator.as_str())
        .collect();
    for cited in [
        "complete-search",
        "placement",
        "free-floor",
        "axioval:free-floor:cad:model/room:largest=[1.12,1.125]:centre=[0,0,0]",
        "axioval:free-floor:cad:model/room:area=[6.25,6.25]",
    ] {
        assert!(locators.contains(&cited), "{cited} in {locators:?}");
    }
    assert!(finding.evidence.iter().all(|evidence| evidence.exact));
}

/// The proven absence of `FakeService`, from a service that also
/// estimates the largest disc (#400), counting the searches for smaller
/// circles.
struct Estimating {
    estimate: (f64, f64),
    smaller: AtomicUsize,
}

impl FreeSpaceService for Estimating {
    fn assess_clearance(
        &self,
        request: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError> {
        FakeService(Answer::NoPlacement).assess_clearance(request)
    }
    fn measure_free_area(
        &self,
        request: &FreeAreaRequest,
    ) -> Result<FreeAreaEvidence, FreeSpaceError> {
        FakeService(Answer::NoPlacement).measure_free_area(request)
    }
    fn measure_free_floor(
        &self,
        request: &PlacementRequest,
    ) -> Result<FreeFloorEvidence, FreeSpaceError> {
        FakeService(Answer::NoPlacement).measure_free_floor(request)
    }
    fn find_placement(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementOutcome, FreeSpaceError> {
        if let PlacementShape::Cylinder(shape) = request.shape()
            && shape.radius_metres() < 0.75
        {
            self.smaller.fetch_add(1, Ordering::SeqCst);
        }
        FakeService(Answer::NoPlacement).find_placement(request)
    }
    fn estimate_largest_disc(
        &self,
        _: &PlacementRequest,
    ) -> Result<LargestDiscEstimate, FreeSpaceError> {
        LargestDiscEstimate::new(self.estimate.0, self.estimate.1)
            .ok_or_else(|| FreeSpaceError::Unavailable("no estimate".into()))
    }
}

/// The largest circle is bracketed from the service's estimate (#400):
/// the sizes it suggests are searched first, a circle of 1.119 m placed
/// and one of 1.126 m proven absent, and one search between them closes
/// the bracket, three searches where the bisection took eight. An
/// estimate far off only orders the searches: the bracket still holds the
/// largest circle, within the searches' budget, from both the template
/// and the implementation it replaced.
#[test]
fn the_largest_circle_is_bracketed_from_the_estimate_in_a_few_searches() {
    let close = Arc::new(Estimating {
        estimate: (0.5599, 0.5626),
        smaller: AtomicUsize::new(0),
    });
    let outcome = evaluate_with(close.clone());
    assert_eq!(
        outcome.findings()[0].message,
        "NO_FREE_FLOOR_SPACE_FOR_CIRCLE: the largest free circle is between 1.119 m and \
         1.123 m across, centred at (0 m, 0 m, 0 m); required 1.5 m; the free floor is 6.25 m²"
    );
    // The template and the implementation it is held to search alike.
    assert_eq!(close.smaller.load(Ordering::SeqCst), 2 * 3);
    let wrong = Arc::new(Estimating {
        estimate: (0.1, 0.2),
        smaller: AtomicUsize::new(0),
    });
    let message = evaluate_with(wrong.clone()).findings()[0].message.clone();
    assert!(
        message.contains("the largest free circle is between 1.1"),
        "{message}"
    );
    assert!(wrong.smaller.load(Ordering::SeqCst) <= 2 * 10);
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

/// A placement is exact only where found on exact evidence: a search the
/// evidence leaves open lists an uncertain placement that is never exact,
/// and an approximate witness (a tessellation's) lists none at all.
#[test]
fn only_a_placement_found_exactly_is_exact() {
    let project = Project::new(vec![
        Object::new(ObjectId::new(source(), "room").unwrap(), "space"),
        Object::new(ObjectId::new(source(), "chair").unwrap(), "furniture"),
    ])
    .unwrap();
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    let measure = |answer: Answer| {
        let mut services = ServiceRegistry::new();
        services
            .register(FreeSpaceServiceHandle::new(Arc::new(FakeService(answer))))
            .unwrap();
        services.register(common::natures(source())).unwrap();
        registry.install_measured(&mut services, &project);
        axioval_engine::measured_members(
            &services,
            &ObjectId::new(source(), "room").unwrap(),
            "free_placements;shape=circle;diameter=1.5;height=2",
        )
        .map(|members| {
            members
                .iter()
                .map(|member| (member.certain, member.exact))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(measure(Answer::Found), Ok(vec![(true, true)]));
    assert_eq!(measure(Answer::Incomplete), Ok(vec![(false, false)]));
    assert!(measure(Answer::Invalid).is_err());
}

/// The capability's own answer, `free_floor_fit`: a fit found or proven
/// absent on exact evidence is exact, a search the evidence leaves open is
/// undecided and never exact, and a refused search refuses the list.
#[test]
fn a_fit_is_exact_only_where_its_answer_rests_on_exact_evidence() {
    let project = Project::new(vec![
        Object::new(ObjectId::new(source(), "room").unwrap(), "space"),
        Object::new(ObjectId::new(source(), "chair").unwrap(), "furniture"),
    ])
    .unwrap();
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    let measure = |answer: Answer| {
        let mut services = ServiceRegistry::new();
        services
            .register(FreeSpaceServiceHandle::new(Arc::new(FakeService(answer))))
            .unwrap();
        services.register(common::natures(source())).unwrap();
        registry.install_measured(&mut services, &project);
        axioval_engine::measured_members(
            &services,
            &ObjectId::new(source(), "room").unwrap(),
            "free_floor_fit;shape=circle;diameter_metres=1.5;height_metres=2",
        )
        .map(|members| {
            members
                .iter()
                .map(|member| {
                    let fits = match &member.fields["fits"] {
                        axioval_engine::MemberValue::Truth { value, .. } => Some(*value),
                        _ => None,
                    };
                    (fits, member.exact)
                })
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(measure(Answer::Found), Ok(vec![(Some(true), true)]));
    assert_eq!(measure(Answer::NoPlacement), Ok(vec![(Some(false), true)]));
    assert_eq!(measure(Answer::Incomplete), Ok(vec![(None, false)]));
    assert!(measure(Answer::Invalid).is_err());
    assert!(measure(Answer::Unavailable).is_err());
}

/// Whether the shape fits as an expression over the measured placements
/// (at least one), held to the parity harness on every answer of the
/// service and without it.
#[test]
fn a_fit_as_a_count_of_placements_reaches_the_verdicts() {
    let fits = serde_json::json!({"kind": "compare", "operator": "greaterThanOrEquals",
        "left": {"kind": "aggregate", "function": "count",
            "over": {"kind": "measured", "name": "free_placements;shape=circle;diameter=1.5;height=2"}},
        "right": {"kind": "literal", "value": {"type": "integer", "value": 1}}});
    let expression = CompiledRule {
        capability: "axioval:capability.expression".into(),
        parameters: BTreeMap::from([(
            "requirement".to_owned(),
            ParameterValue::Expression {
                value: serde_json::from_value(fits).unwrap(),
            },
        )]),
        ..rule()
    };
    let project = Project::new(vec![
        Object::new(ObjectId::new(source(), "room").unwrap(), "space"),
        Object::new(ObjectId::new(source(), "chair").unwrap(), "furniture"),
    ])
    .unwrap();
    let registry =
        axioval_rules::register_builtins(axioval_engine::CapabilityRegistry::new()).unwrap();
    let answers = [
        Some(Answer::Found),
        Some(Answer::NoPlacement),
        Some(Answer::Unavailable),
        Some(Answer::Incomplete),
        Some(Answer::Invalid),
        None,
    ];
    for answer in answers {
        let mut services = ServiceRegistry::new();
        if let Some(answer) = answer {
            services
                .register(FreeSpaceServiceHandle::new(Arc::new(FakeService(answer))))
                .unwrap();
            services.register(common::natures(source())).unwrap();
        }
        let context = RuleContext {
            project: &project,
            services: &services,
        };
        let expected = HELD.evaluate(&context, &rule());
        registry.install_measured(&mut services, &project);
        let evaluation = axioval_rules::ExpressionRequirement.evaluate(
            &RuleContext {
                project: &project,
                services: &services,
            },
            &expression,
        );
        let parity = axioval_rules::parity::compare_evaluations(
            ("free-floor", &expected),
            ("expression", &evaluation),
        );
        assert!(parity.holds(), "{}", parity.diff());
        assert_eq!(
            parity.objects,
            usize::from(answer.is_none_or(|answer| !matches!(answer, Answer::Found)))
        );
    }
}
