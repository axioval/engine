//! Exact source-neutral free-floor-circle capability.

use axioval_engine::{
    CapabilityEvaluation, ClearanceShape, CompiledRule, CylinderClearance, FreeSpaceError,
    FreeSpaceServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PlacementDomain, PlacementOutcome, PlacementRequest, RuleCapability, RuleContext,
    SupportedPlacement,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Finding, Object, Severity};

use crate::selector_matches;

/// Exact free-floor circle placement using a trusted free-space service.
pub struct FreeFloorCircle;

impl RuleCapability for FreeFloorCircle {
    fn id(&self) -> &'static str {
        "axioval:capability.free-floor-circle"
    }
    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("diameter_metres", ParameterType::Number),
            ParameterDescriptor::required("height_metres", ParameterType::Number),
        ]
    }
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let selected: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| selector_matches(&rule.selector, object))
            .collect();
        let Some((diameter, height)) = dimensions(rule) else {
            return invalid_parameters(selected, "free-floor circle dimensions are invalid");
        };
        let Ok(shape) = CylinderClearance::try_new(diameter / 2.0, height) else {
            return invalid_parameters(
                selected,
                "free-floor circle dimensions must be positive and finite",
            );
        };
        let Some(service) = context.services.get::<FreeSpaceServiceHandle>() else {
            return unavailable(
                selected,
                &NotEvaluatedReason::MissingService,
                "free-space service is not registered",
            );
        };
        let all_objects: Vec<_> = context
            .project
            .objects()
            .map(|object| object.id.clone())
            .collect();
        let mut evaluation = CapabilityEvaluation::default();
        for space in selected {
            let obstacles = all_objects
                .iter()
                .filter(|id| *id != &space.id)
                .cloned()
                .collect();
            let support = match SupportedPlacement::try_new(space.id.clone(), 0.0) {
                Ok(support) => support,
                Err(error) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::InvalidEvidence,
                        error.to_string(),
                    );
                    continue;
                }
            };
            let request = match PlacementRequest::new_in_domain(
                space.id.clone(),
                ClearanceShape::Cylinder(shape),
                obstacles,
                PlacementDomain::Supported(support),
            ) {
                Ok(request) => request,
                Err(error) => {
                    evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        NotEvaluatedReason::InvalidEvidence,
                        error.to_string(),
                    );
                    continue;
                }
            };
            match service.find_placement(&request) {
                Ok(PlacementOutcome::Found(_)) => {}
                Ok(PlacementOutcome::NoPlacement(proof)) => evaluation.push_finding(Finding {
                    rule_id: rule.id.clone(),
                    object_id: space.id.clone(),
                    severity: severity(rule),
                    message: "NO_FREE_FLOOR_SPACE_FOR_CIRCLE".into(),
                    evidence: vec![proof.evidence().clone()],
                }),
                Err(error) => evaluation.push_object_not_evaluated(
                    space.id.clone(),
                    reason(&error),
                    error.to_string(),
                ),
            }
        }
        evaluation
    }
}

fn dimensions(rule: &CompiledRule) -> Option<(f64, f64)> {
    let number = |name: &str| match rule.parameters.get(name)? {
        ParameterValue::Number { value } => Some(*value),
        _ => None,
    };
    Some((number("diameter_metres")?, number("height_metres")?))
}
fn severity(rule: &CompiledRule) -> Severity {
    match rule.severity {
        axioval_ir::contract::Severity::Error => Severity::Error,
        axioval_ir::contract::Severity::Warning => Severity::Warning,
        axioval_ir::contract::Severity::Info => Severity::Info,
    }
}
fn invalid_parameters(selected: Vec<&Object>, message: &str) -> CapabilityEvaluation {
    unavailable(selected, &NotEvaluatedReason::InvalidEvidence, message)
}
fn unavailable(
    selected: Vec<&Object>,
    reason: &NotEvaluatedReason,
    message: &str,
) -> CapabilityEvaluation {
    let mut evaluation = CapabilityEvaluation::default();
    for object in selected {
        evaluation.push_object_not_evaluated(object.id.clone(), reason.clone(), message);
    }
    evaluation
}
fn reason(error: &FreeSpaceError) -> NotEvaluatedReason {
    match error {
        FreeSpaceError::MissingGeometry(_) | FreeSpaceError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        FreeSpaceError::IncompleteClearanceEvidence => NotEvaluatedReason::IncompleteEvidence,
        _ => NotEvaluatedReason::InvalidEvidence,
    }
}
