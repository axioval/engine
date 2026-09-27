//! Exact source-neutral space-validation capability.
//!
//! ADR 0004: geometry is measured by a [`SpaceServiceHandle`]; every threshold
//! and severity decision lives here.
//!
//! Each aspect is requested and judged independently, so an adapter that
//! cannot measure one of them costs only that aspect. The source bundled all
//! seven behind one call and failed the whole space when any single branch was
//! missing.
//!
//! Every finding message starts with its sub-check's category code (see
//! [`SpaceCategory`]), so results can be grouped by problem as well as by space.

use axioval_engine::{
    Cap, CapCoverage, CapRequest, CapabilityEvaluation, CompiledRule, Containment,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
    SpaceError, SpaceService, SpaceServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Finding, ObjectId, Severity};

use crate::selection::select_objects;

/// Default measurement tolerance, in metres, when a rule declares none. A
/// space is only "low" when it misses the requirement by more than this, and
/// an overlap no thicker than this is contact, not intersection.
const DEFAULT_TOLERANCE_M: f64 = 0.005;
/// Overlaps smaller than this are contact, not intersection.
const OVERLAP_AREA_EPSILON_M2: f64 = 1.0e-8;
/// A cap this well covered is complete for checking purposes.
const CAP_COMPLETE_RATIO: f64 = 0.98;

/// The sub-check a finding comes from, written as the message's leading code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceCategory {
    /// Another space has the same body.
    DuplicateSpace,
    /// Clear height below the requirement.
    InsufficientHeight,
    /// A run of space boundary no element covers.
    UncoveredBoundary,
    /// The space contains, or is contained by, another body.
    ContainedBody,
    /// The space intersects another space.
    IntersectingSpace,
    /// The space intersects a component.
    IntersectingComponent,
    /// The top cap is not fully covered.
    UncoveredTopCap,
    /// The bottom cap is not fully covered.
    UncoveredBottomCap,
    /// Storey floor area belongs to no space.
    UnallocatedArea,
}

impl SpaceCategory {
    /// The stable code a finding message starts with.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::DuplicateSpace => "duplicate_space",
            Self::InsufficientHeight => "insufficient_height",
            Self::UncoveredBoundary => "uncovered_boundary",
            Self::ContainedBody => "contained_body",
            Self::IntersectingSpace => "intersecting_space",
            Self::IntersectingComponent => "intersecting_component",
            Self::UncoveredTopCap => "uncovered_top_cap",
            Self::UncoveredBottomCap => "uncovered_bottom_cap",
            Self::UnallocatedArea => "unallocated_area",
        }
    }

    fn message(self, detail: &str) -> String {
        format!("{}: {detail}", self.code())
    }
}

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
            ParameterDescriptor::optional("tolerance_metres", ParameterType::Number),
            ParameterDescriptor::optional("top_cap_elements", ParameterType::Selector),
            ParameterDescriptor::optional("bottom_cap_elements", ParameterType::Selector),
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
        let (top, bottom) = match cap_plans(context, service, &policy) {
            Ok(plans) => plans,
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
            check_overlaps(service, space, &policy, rule, &evidence, &mut evaluation);
            for plan in [&top, &bottom] {
                check_cap(service, space, plan, rule, &evidence, &mut evaluation);
            }
        }

        if policy.check_unallocated_area {
            check_residuals(service, &policy, rule, &evidence, &mut evaluation);
        }
        evaluation
    }
}

struct Policy {
    tolerance_metres: f64,
    top_cap_elements: Option<Selector>,
    bottom_cap_elements: Option<Selector>,
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
        // Optional, but a value of the wrong type is an invalid declaration,
        // never "absent".
        let tolerance_metres = match rule.parameters.get("tolerance_metres") {
            None => DEFAULT_TOLERANCE_M,
            Some(_) => number("tolerance_metres")?,
        };
        let selector = |key: &str| match rule.parameters.get(key) {
            None => Some(None),
            Some(ParameterValue::Selector { value }) => Some(Some((**value).clone())),
            Some(_) => None,
        };
        Some(Self {
            tolerance_metres,
            top_cap_elements: selector("top_cap_elements")?,
            bottom_cap_elements: selector("bottom_cap_elements")?,
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

/// What to do about one cap for every space.
enum CapPlan {
    /// The cap is not checked: disabled, or nothing could form it.
    Skip,
    /// Measure the cap with this request.
    Check(CapRequest),
    /// The rule's cap-element selection could not be decided, so no cap
    /// coverage can be judged: an undecided element may be the one covering.
    Undecided(NotEvaluatedReason, String),
}

/// A cap check needs elements that could form that cap. Without any, every
/// space would be reported uncovered, which says nothing about the spaces and
/// everything about the model.
///
/// A rule that selects its cap elements states them; otherwise the host's
/// declared slabs (and, for the top, roofs) are counted. The support counts
/// are asked for only when a cap falls back to the host's declaration.
fn cap_plans(
    context: &RuleContext<'_>,
    service: &dyn SpaceService,
    policy: &Policy,
) -> Result<(CapPlan, CapPlan), SpaceError> {
    let needs_counts = (policy.check_top_cap && policy.top_cap_elements.is_none())
        || (policy.check_bottom_cap && policy.bottom_cap_elements.is_none());
    let counts = if needs_counts {
        Some(service.measure_support_counts()?)
    } else {
        None
    };
    let plan = |cap: Cap, enabled: bool, selector: Option<&Selector>| -> CapPlan {
        if !enabled {
            return CapPlan::Skip;
        }
        if let Some(selector) = selector {
            let (elements, selection) = select_objects(context, selector);
            if let Some(outcome) = selection.not_evaluated_outcomes().first() {
                return CapPlan::Undecided(
                    outcome.reason().clone(),
                    format!(
                        "{} cap elements could not be selected: {}",
                        cap_name(cap),
                        outcome.message()
                    ),
                );
            }
            if elements.is_empty() {
                return CapPlan::Skip;
            }
            return CapPlan::Check(
                CapRequest::new(cap)
                    .with_elements(elements.iter().map(|object| object.id.clone()).collect()),
            );
        }
        let Some(counts) = &counts else {
            return CapPlan::Skip;
        };
        let available = match cap {
            Cap::Top => counts.slabs() > 0 || counts.roofs() > 0,
            Cap::Bottom => counts.slabs() > 0,
        };
        if available {
            CapPlan::Check(CapRequest::new(cap))
        } else {
            CapPlan::Skip
        }
    };
    Ok((
        plan(
            Cap::Top,
            policy.check_top_cap,
            policy.top_cap_elements.as_ref(),
        ),
        plan(
            Cap::Bottom,
            policy.check_bottom_cap,
            policy.bottom_cap_elements.as_ref(),
        ),
    ))
}

fn cap_name(cap: Cap) -> &'static str {
    match cap {
        Cap::Top => "top",
        Cap::Bottom => "bottom",
    }
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
            let message = SpaceCategory::DuplicateSpace.message(&format!(
                "space body duplicated by {} other space(s)",
                duplicates.len()
            ));
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
            if height.metres() + policy.tolerance_metres < policy.required_height_metres {
                evaluation.push_finding(finding(
                    rule,
                    space.clone(),
                    Severity::Warning,
                    SpaceCategory::InsufficientHeight.message(&format!(
                        "clear height {:.3} below required {:.3}",
                        height.metres(),
                        policy.required_height_metres
                    )),
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
                // The elements along the uncovered runs, so a reviewer can
                // see which boundary is short.
                let related = gaps
                    .iter()
                    .filter(|gap| gap.length_metres() >= policy.uncovered_segment_length_metres)
                    .flat_map(|gap| gap.elements().iter().cloned());
                evaluation.push_finding(
                    finding(
                        rule,
                        space.clone(),
                        Severity::Warning,
                        SpaceCategory::UncoveredBoundary
                            .message(&format!("{total:.3} m of space boundary is uncovered")),
                        evidence,
                    )
                    .with_related(related),
                );
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
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_overlaps(space) {
        Ok(overlaps) => {
            for overlap in overlaps {
                let message = match overlap.containment() {
                    Containment::SubjectInsideOther => Some(
                        SpaceCategory::ContainedBody.message("space is contained by another body"),
                    ),
                    Containment::OtherInsideSubject => {
                        Some(SpaceCategory::ContainedBody.message("space contains another body"))
                    }
                    Containment::Partial
                        if overlap.area_square_metres() >= OVERLAP_AREA_EPSILON_M2
                            && overlap.height_metres() > policy.tolerance_metres =>
                    {
                        let (category, other) = if overlap.other_is_space() {
                            (SpaceCategory::IntersectingSpace, "another space")
                        } else {
                            (SpaceCategory::IntersectingComponent, "a component")
                        };
                        Some(category.message(&format!(
                            "space intersects {other} over {:.4} m2",
                            overlap.area_square_metres()
                        )))
                    }
                    Containment::Partial => None,
                };
                if let Some(message) = message {
                    evaluation.push_finding(
                        finding(rule, space.clone(), Severity::Error, message, evidence)
                            // The body it overlaps, so a reviewer can open it.
                            .with_related([overlap.other().clone()]),
                    );
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
    plan: &CapPlan,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    let request = match plan {
        CapPlan::Skip => return,
        CapPlan::Undecided(reason, message) => {
            evaluation.push_object_not_evaluated(space.clone(), reason.clone(), message.clone());
            return;
        }
        CapPlan::Check(request) => request,
    };
    let cap = request.cap();
    match service.measure_cap_coverage(space, request) {
        Ok(coverage) => {
            if let Some((severity, ratio)) = cap_shortfall(&coverage) {
                let related = coverage.elements().to_vec();
                evaluation.push_finding(
                    finding(
                        rule,
                        space.clone(),
                        severity,
                        match cap {
                            Cap::Top => SpaceCategory::UncoveredTopCap,
                            Cap::Bottom => SpaceCategory::UncoveredBottomCap,
                        }
                        .message(&format!(
                            "{} cap only {:.1}% covered",
                            cap_name(cap),
                            ratio * 100.0
                        )),
                        evidence,
                    )
                    // The elements covering the cap, so a reviewer can see what
                    // is there and what is missing.
                    .with_related(related),
                );
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
                        SpaceCategory::UnallocatedArea.message(&format!(
                            "{:.3} m2 of storey floor belongs to no space",
                            residual.area_square_metres()
                        )),
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
