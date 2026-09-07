//! Exact source-neutral space-validation capability.
//!
//! ADR 0004: geometry is measured by a [`SpaceServiceHandle`]; every threshold
//! and severity decision lives here.
//!
//! Each aspect is requested and judged independently, so an adapter that
//! cannot measure one of them costs only that aspect. The source bundled all
//! seven behind one call and failed the whole space when any single branch was
//! missing.

use axioval_engine::{
    Cap, CapCoverage, CapabilityEvaluation, CompiledRule, Containment, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, RuleCapability, RuleContext, SpaceError, SpaceService,
    SpaceServiceHandle,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::{Evidence, Finding, ObjectId, Severity};

use crate::selection::select_objects;

/// Height tolerance, in metres. A space is only "low" when it misses the
/// requirement by more than measurement noise.
const HEIGHT_TOLERANCE_M: f64 = 0.005;
/// Overlaps thinner or smaller than these are contact, not intersection.
const OVERLAP_AREA_EPSILON_M2: f64 = 1.0e-8;
const OVERLAP_HEIGHT_EPSILON_M: f64 = 0.005;
/// A cap this well covered is complete for checking purposes.
const CAP_COMPLETE_RATIO: f64 = 0.98;

/// Validates spaces against height, duplication, coverage and overlap rules.
pub struct SpaceValidation;

impl RuleCapability for SpaceValidation {
    fn id(&self) -> &'static str {
        "axioval:capability.space-validation"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("required_height_metres", ParameterType::Number),
            ParameterDescriptor::required("uncovered_segment_length_metres", ParameterType::Number),
            ParameterDescriptor::required("check_top_cap", ParameterType::Boolean),
            ParameterDescriptor::required("check_bottom_cap", ParameterType::Boolean),
            ParameterDescriptor::required("check_unallocated_area", ParameterType::Boolean),
            ParameterDescriptor::required(
                "maximum_unallocated_area_square_metres",
                ParameterType::Number,
            ),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let (selected, mut evaluation) = select_objects(context, &rule.selector);

        let Some(policy) = Policy::from_rule(rule) else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::InvalidDeclaration,
                    "space-validation declaration is missing or not realisable",
                );
            }
            return evaluation;
        };

        let Some(handle) = context.services.get::<SpaceServiceHandle>() else {
            for object in selected {
                evaluation.push_object_not_evaluated(
                    object.id.clone(),
                    NotEvaluatedReason::MissingService,
                    "space service is not registered",
                );
            }
            return evaluation;
        };
        let service = handle.get();
        let evidence = service.evidence();

        // Which caps can be checked at all is a model-wide question: a cap
        // check is meaningless when the model contains no elements that could
        // form that cap. Asked once, not per space.
        let (top_enabled, bottom_enabled) = match cap_availability(service, &policy) {
            Ok(enabled) => enabled,
            Err(error) => {
                for object in selected {
                    evaluation.push_object_not_evaluated(
                        object.id.clone(),
                        reason(error),
                        error.to_string(),
                    );
                }
                return evaluation;
            }
        };

        for object in selected {
            let space = &object.id;
            check_duplicates(service, space, rule, &evidence, &mut evaluation);
            check_height(service, space, &policy, rule, &evidence, &mut evaluation);
            check_boundary(service, space, &policy, rule, &evidence, &mut evaluation);
            check_overlaps(service, space, rule, &evidence, &mut evaluation);
            if top_enabled {
                check_cap(service, space, Cap::Top, rule, &evidence, &mut evaluation);
            }
            if bottom_enabled {
                check_cap(
                    service,
                    space,
                    Cap::Bottom,
                    rule,
                    &evidence,
                    &mut evaluation,
                );
            }
        }

        if policy.check_unallocated_area {
            check_residuals(service, &policy, rule, &evidence, &mut evaluation);
        }
        evaluation
    }
}

struct Policy {
    required_height_metres: f64,
    uncovered_segment_length_metres: f64,
    check_top_cap: bool,
    check_bottom_cap: bool,
    check_unallocated_area: bool,
    maximum_unallocated_area_square_metres: f64,
}

impl Policy {
    fn from_rule(rule: &CompiledRule) -> Option<Self> {
        let number = |key: &str| match rule.parameters.get(key)? {
            ParameterValue::Number { value } if value.is_finite() && *value >= 0.0 => Some(*value),
            _ => None,
        };
        let boolean = |key: &str| match rule.parameters.get(key)? {
            ParameterValue::Boolean { value } => Some(*value),
            _ => None,
        };
        Some(Self {
            required_height_metres: number("required_height_metres")?,
            uncovered_segment_length_metres: number("uncovered_segment_length_metres")?,
            check_top_cap: boolean("check_top_cap")?,
            check_bottom_cap: boolean("check_bottom_cap")?,
            check_unallocated_area: boolean("check_unallocated_area")?,
            maximum_unallocated_area_square_metres: number(
                "maximum_unallocated_area_square_metres",
            )?,
        })
    }
}

fn reason(error: SpaceError) -> NotEvaluatedReason {
    match error {
        SpaceError::Unavailable => NotEvaluatedReason::IncompleteEvidence,
        SpaceError::InexactEvidence | SpaceError::InvalidQuantity => {
            NotEvaluatedReason::InvalidEvidence
        }
    }
}

fn finding(
    rule: &CompiledRule,
    object_id: ObjectId,
    severity: Severity,
    message: String,
    evidence: &Evidence,
) -> Finding {
    Finding {
        rule_id: rule.id.clone(),
        object_id,
        related: Vec::new(),
        severity,
        message,
        evidence: vec![evidence.clone()],
    }
}

/// A cap check needs elements that could form that cap. Without any slab,
/// every space would be reported uncovered, which says nothing about the
/// spaces and everything about the model.
fn cap_availability(
    service: &dyn SpaceService,
    policy: &Policy,
) -> Result<(bool, bool), SpaceError> {
    if !policy.check_top_cap && !policy.check_bottom_cap {
        return Ok((false, false));
    }
    let counts = service.measure_support_counts()?;
    Ok((
        policy.check_top_cap && (counts.slabs() > 0 || counts.roofs() > 0),
        policy.check_bottom_cap && counts.slabs() > 0,
    ))
}

fn check_duplicates(
    service: &dyn SpaceService,
    space: &ObjectId,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_duplicates(space) {
        Ok(duplicates) if duplicates.is_empty() => {}
        Ok(duplicates) => {
            let message = format!(
                "space body duplicated by {} other space(s)",
                duplicates.len()
            );
            evaluation.push_finding(
                finding(rule, space.clone(), Severity::Error, message, evidence)
                    // The coincident spaces, so a reviewer can open them.
                    .with_related(duplicates),
            );
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(error), error.to_string());
        }
    }
}

fn check_height(
    service: &dyn SpaceService,
    space: &ObjectId,
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_clear_height(space) {
        Ok(height) => {
            if height.metres() + HEIGHT_TOLERANCE_M < policy.required_height_metres {
                evaluation.push_finding(finding(
                    rule,
                    space.clone(),
                    Severity::Warning,
                    format!(
                        "clear height {:.3} below required {:.3}",
                        height.metres(),
                        policy.required_height_metres
                    ),
                    evidence,
                ));
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(error), error.to_string());
        }
    }
}

fn check_boundary(
    service: &dyn SpaceService,
    space: &ObjectId,
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_boundary_gaps(space) {
        Ok(gaps) => {
            // Only gaps at least as long as the declared segment count; a
            // shorter gap is a modelling artefact, not an uncovered wall.
            let total: f64 = gaps
                .iter()
                .filter(|gap| gap.length_metres() >= policy.uncovered_segment_length_metres)
                .map(axioval_engine::BoundaryGap::length_metres)
                .sum();
            if total > 0.0 {
                evaluation.push_finding(finding(
                    rule,
                    space.clone(),
                    Severity::Warning,
                    format!("{total:.3} m of space boundary is uncovered"),
                    evidence,
                ));
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(error), error.to_string());
        }
    }
}

fn check_overlaps(
    service: &dyn SpaceService,
    space: &ObjectId,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_overlaps(space) {
        Ok(overlaps) => {
            for overlap in overlaps {
                let message = match overlap.containment() {
                    Containment::SubjectInsideOther | Containment::OtherInsideSubject => {
                        Some("space is contained by another body".to_string())
                    }
                    Containment::Partial
                        if overlap.area_square_metres() >= OVERLAP_AREA_EPSILON_M2
                            && overlap.height_metres() > OVERLAP_HEIGHT_EPSILON_M =>
                    {
                        Some(format!(
                            "space intersects {} over {:.4} m2",
                            if overlap.other_is_space() {
                                "another space"
                            } else {
                                "a component"
                            },
                            overlap.area_square_metres()
                        ))
                    }
                    Containment::Partial => None,
                };
                if let Some(message) = message {
                    evaluation.push_finding(finding(
                        rule,
                        space.clone(),
                        Severity::Error,
                        message,
                        evidence,
                    ));
                }
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(error), error.to_string());
        }
    }
}

fn check_cap(
    service: &dyn SpaceService,
    space: &ObjectId,
    cap: Cap,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_cap_coverage(space, cap) {
        Ok(coverage) => {
            if let Some((severity, ratio)) = cap_shortfall(&coverage) {
                evaluation.push_finding(finding(
                    rule,
                    space.clone(),
                    severity,
                    format!(
                        "{} cap only {:.1}% covered",
                        match cap {
                            Cap::Top => "top",
                            Cap::Bottom => "bottom",
                        },
                        ratio * 100.0
                    ),
                    evidence,
                ));
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(error), error.to_string());
        }
    }
}

/// Grades cap coverage: almost none is severe, a little is a warning, and
/// nearly complete is informational.
fn cap_shortfall(coverage: &CapCoverage) -> Option<(Severity, f64)> {
    let ratio = coverage.covered_ratio();
    if ratio >= CAP_COMPLETE_RATIO {
        return None;
    }
    let severity = if ratio < 0.01 {
        Severity::Error
    } else if ratio <= 0.15 {
        Severity::Warning
    } else {
        Severity::Info
    };
    Some((severity, ratio))
}

fn check_residuals(
    service: &dyn SpaceService,
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_storey_residuals() {
        Ok(residuals) => {
            for residual in residuals {
                if residual.area_square_metres() > policy.maximum_unallocated_area_square_metres {
                    evaluation.push_finding(finding(
                        rule,
                        residual.storey().clone(),
                        Severity::Warning,
                        format!(
                            "{:.3} m2 of storey floor belongs to no space",
                            residual.area_square_metres()
                        ),
                        evidence,
                    ));
                }
            }
        }
        Err(error) => {
            evaluation.push_not_evaluated(reason(error), error.to_string());
        }
    }
}
