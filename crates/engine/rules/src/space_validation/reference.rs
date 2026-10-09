//! The `space-validation` implementation its template replaced, kept only
//! as the parity reference the template is held to in tests
//! (`axioval_rules::reference::SpaceValidation`). Never register it.

use std::collections::BTreeMap;

use axioval_engine::{
    BoundaryRequest, Cap, CapRequest, CapabilityEvaluation, CompiledRule, Containment, Deviation,
    NotEvaluatedReason, OverlapRequest, ParameterDescriptor, ParameterType, RuleCapability,
    RuleContext, SpaceError, SpaceService, SpaceServiceHandle, UnallocatedRegion,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Finding, ObjectId, Severity};

use super::{SpaceCategory, reason};
use crate::selection::select_objects;

/// Default measurement tolerance, in metres, when a rule declares none. A
/// space is only "low" when it misses the requirement by more than this, and
/// an overlap no thicker than this is contact, not intersection.
const DEFAULT_TOLERANCE_M: f64 = 0.005;
/// A cap this well covered is complete for checking purposes.
const CAP_COMPLETE_RATIO: f64 = 0.98;

/// The message of a finding of `category`.
fn message(category: SpaceCategory, detail: &str) -> String {
    format!("{}: {detail}", category.code())
}

/// Validates spaces against height, duplication, coverage and overlap rules,
/// as the capability judged it before it became a template.
pub struct SpaceValidation;

impl RuleCapability for SpaceValidation {
    fn id(&self) -> &'static str {
        "axioval:capability.space-validation"
    }

    fn grades_deviation(&self) -> bool {
        true
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
            ParameterDescriptor::optional("maximum_unallocated_share", ParameterType::Number),
            ParameterDescriptor::optional("top_cap_elements", ParameterType::Selector),
            ParameterDescriptor::optional("bottom_cap_elements", ParameterType::Selector),
            ParameterDescriptor::optional("boundary_elements", ParameterType::Selector),
            ParameterDescriptor::optional("intersection_elements", ParameterType::Selector),
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
                        reason(&error),
                        error.to_string(),
                    );
                }
                return evaluation;
            }
        };

        // Which elements bound and intersect spaces is the rule's choice,
        // selected once for every space.
        let boundary = element_plan(
            context,
            policy.boundary_elements.as_ref(),
            "boundary",
            BoundaryRequest::new(),
            BoundaryRequest::with_elements,
        );
        let intersection = element_plan(
            context,
            policy.intersection_elements.as_ref(),
            "intersection",
            OverlapRequest::new(),
            OverlapRequest::with_elements,
        );

        for object in selected {
            let space = &object.id;
            check_duplicates(service, space, rule, &evidence, &mut evaluation);
            check_height(service, space, &policy, rule, &evidence, &mut evaluation);
            if let Some(request) = planned(&boundary, space, &mut evaluation) {
                check_boundary(
                    service,
                    space,
                    request,
                    &policy,
                    rule,
                    &evidence,
                    &mut evaluation,
                );
            }
            if let Some(request) = planned(&intersection, space, &mut evaluation) {
                check_overlaps(
                    service,
                    space,
                    request,
                    &policy,
                    rule,
                    &evidence,
                    &mut evaluation,
                );
            }
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
    boundary_elements: Option<Selector>,
    intersection_elements: Option<Selector>,
    required_height_metres: f64,
    uncovered_segment_length_metres: f64,
    check_top_cap: bool,
    check_bottom_cap: bool,
    check_unallocated_area: bool,
    maximum_unallocated_area_square_metres: f64,
    /// The largest share of a storey's gross floor area no space may cover.
    maximum_unallocated_share: Option<f64>,
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
        let maximum_unallocated_share = match rule.parameters.get("maximum_unallocated_share") {
            None => None,
            Some(_) => Some(number("maximum_unallocated_share").filter(|share| *share <= 1.0)?),
        };
        Some(Self {
            maximum_unallocated_share,
            tolerance_metres,
            top_cap_elements: selector("top_cap_elements")?,
            bottom_cap_elements: selector("bottom_cap_elements")?,
            boundary_elements: selector("boundary_elements")?,
            intersection_elements: selector("intersection_elements")?,
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

fn finding(
    rule: &CompiledRule,
    object_id: ObjectId,
    severity: Severity,
    message: String,
    evidence: &Evidence,
) -> Finding {
    Finding {
        explanation: None,
        id: None,
        decision: None,
        rule_id: rule.id.clone(),
        scope: axioval_ir::Scope::Object(object_id),
        related: Vec::new(),
        severity,
        message,
        evidence: vec![evidence.clone()],
        location: None,
        categories: Vec::new(),
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

/// What to do about a sub-check whose elements a rule may select.
enum ElementPlan<R> {
    /// Nothing is selected, so nothing can bound or intersect a space.
    Skip,
    /// Measure with this request.
    Check(R),
    /// The selection could not be decided: an undecided element may be the
    /// one covering a boundary or intersecting a space.
    Undecided(NotEvaluatedReason, String),
}

/// Selects a sub-check's elements, or keeps the service's default without a
/// selector. A selection of nothing skips the sub-check, as a cap selection
/// does: a check with no element to judge by says nothing about the space.
fn element_plan<R>(
    context: &RuleContext<'_>,
    selector: Option<&Selector>,
    name: &str,
    default: R,
    with_elements: impl Fn(R, Vec<ObjectId>) -> R,
) -> ElementPlan<R> {
    let Some(selector) = selector else {
        return ElementPlan::Check(default);
    };
    let (elements, selection) = select_objects(context, selector);
    if let Some(outcome) = selection.not_evaluated_outcomes().first() {
        return ElementPlan::Undecided(
            outcome.reason().clone(),
            format!(
                "{name} elements could not be selected: {}",
                outcome.message()
            ),
        );
    }
    if elements.is_empty() {
        return ElementPlan::Skip;
    }
    ElementPlan::Check(with_elements(
        default,
        elements.iter().map(|object| object.id.clone()).collect(),
    ))
}

/// The request to measure `space` with, or `None` after recording why not.
fn planned<'p, R>(
    plan: &'p ElementPlan<R>,
    space: &ObjectId,
    evaluation: &mut CapabilityEvaluation,
) -> Option<&'p R> {
    match plan {
        ElementPlan::Skip => None,
        ElementPlan::Check(request) => Some(request),
        ElementPlan::Undecided(reason, message) => {
            evaluation.push_object_not_evaluated(space.clone(), reason.clone(), message.clone());
            None
        }
    }
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
            let message = message(
                SpaceCategory::DuplicateSpace,
                &format!(
                    "space body duplicated by {} other space(s)",
                    duplicates.len()
                ),
            );
            evaluation.push_finding(
                finding(rule, space.clone(), Severity::Error, message, evidence)
                    // The coincident spaces, so a reviewer can open them.
                    .with_related(duplicates),
            );
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(&error), error.to_string());
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
            let (lower, upper) = height.bounds_metres();
            let short =
                |metres: f64| metres + policy.tolerance_metres < policy.required_height_metres;
            if short(lower) != short(upper) {
                // A tessellated space's chord deviation straddles the
                // requirement: neither a finding nor a pass.
                evaluation.push_object_not_evaluated(
                    space.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "clear height between {lower:.3} and {upper:.3} straddles the required \
                         {:.3} within the space's chord deviation",
                        policy.required_height_metres
                    ),
                );
            } else if short(upper) {
                evaluation.push_finding(finding(
                    rule,
                    space.clone(),
                    Severity::Warning,
                    message(
                        SpaceCategory::InsufficientHeight,
                        &format!(
                            "clear height {upper:.3} below required {:.3}",
                            policy.required_height_metres
                        ),
                    ),
                    evidence,
                ));
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(&error), error.to_string());
        }
    }
}

fn check_boundary(
    service: &dyn SpaceService,
    space: &ObjectId,
    request: &BoundaryRequest,
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_boundary_gaps(space, request) {
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
                        message(
                            SpaceCategory::UncoveredBoundary,
                            &format!("{total:.3} m of space boundary is uncovered"),
                        ),
                        evidence,
                    )
                    .with_related(related),
                );
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(&error), error.to_string());
        }
    }
}

fn check_overlaps(
    service: &dyn SpaceService,
    space: &ObjectId,
    request: &OverlapRequest,
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_overlaps(space, request) {
        Ok(overlaps) => {
            for overlap in overlaps {
                let message = match overlap.containment() {
                    Containment::SubjectInsideOther => Some(message(
                        SpaceCategory::ContainedBody,
                        "space is contained by another body",
                    )),
                    Containment::OtherInsideSubject => Some(message(
                        SpaceCategory::ContainedBody,
                        "space contains another body",
                    )),
                    // Contact, not intersection, as `SpaceOverlap::intersects`
                    // reads it for the measured `intersection_count` too.
                    Containment::Partial if overlap.intersects(policy.tolerance_metres) => {
                        let (category, other) = if overlap.other_is_space() {
                            (SpaceCategory::IntersectingSpace, "another space")
                        } else {
                            (SpaceCategory::IntersectingComponent, "a component")
                        };
                        Some(message(
                            category,
                            &format!(
                                "space intersects {other} over {:.4} m2",
                                overlap.area_square_metres()
                            ),
                        ))
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
            evaluation.push_object_not_evaluated(space.clone(), reason(&error), error.to_string());
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
            let (lower, upper) = coverage.covered_ratio_bounds();
            // A cap short at both ends is graded by its least coverage, the
            // most severe grade it may have; one complete at only one end is
            // undecided.
            let (low, high) = (cap_shortfall(lower), cap_shortfall(upper));
            if low.is_some() != high.is_some() {
                // The chord deviation of a tessellated space or cap element
                // leaves the coverage on both sides of complete.
                evaluation.push_object_not_evaluated(
                    space.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} cap coverage from {:.1}% straddles complete within the chord \
                         deviation of a tessellated body",
                        cap_name(cap),
                        lower * 100.0
                    ),
                );
                return;
            }
            if let Some((severity, ratio)) = low {
                let related = coverage.elements().to_vec();
                evaluation.push_finding(
                    finding(
                        rule,
                        space.clone(),
                        severity,
                        message(
                            match cap {
                                Cap::Top => SpaceCategory::UncoveredTopCap,
                                Cap::Bottom => SpaceCategory::UncoveredBottomCap,
                            },
                            &format!("{} cap only {:.1}% covered", cap_name(cap), ratio * 100.0),
                        ),
                        evidence,
                    )
                    // The elements covering the cap, so a reviewer can see what
                    // is there and what is missing.
                    .with_related(related),
                );
            }
        }
        Err(error) => {
            evaluation.push_object_not_evaluated(space.clone(), reason(&error), error.to_string());
        }
    }
}

/// Grades cap coverage: almost none is severe, a little is a warning, and
/// nearly complete is informational.
fn cap_shortfall(ratio: f64) -> Option<(Severity, f64)> {
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

/// The share of a storey's gross floor area that its unallocated regions
/// cover together, against `maximum`. A storey whose gross area is
/// unmeasured or empty is not evaluated: its share is undefined.
fn check_unallocated_share(
    regions: &[&UnallocatedRegion],
    maximum: f64,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    let Some(first) = regions.first() else {
        return;
    };
    let storey = first.storey();
    // The share `axioval:measured` `unallocated_share` answers too.
    let Some(measured) = UnallocatedRegion::storey_share(regions) else {
        evaluation.push_object_not_evaluated(
            storey.clone(),
            NotEvaluatedReason::IncompleteEvidence,
            "space-validation: the storey's gross floor area is not measured, so its \
             unallocated share is undefined",
        );
        return;
    };
    let (lower, upper) = measured.share();
    if upper <= maximum {
        return;
    }
    let (area, _) = measured.area_square_metres();
    let gross = measured.gross_floor_area_square_metres();
    if lower <= maximum {
        // Only the rounding of the sum and quotient straddles the maximum.
        evaluation.push_object_not_evaluated(
            storey.clone(),
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "space-validation: the storey's unallocated share ({area:.3} m2 of {gross:.3} \
                 m2) straddles the maximum of {}%",
                maximum * 100.0
            ),
        );
        return;
    }
    evaluation.push_graded_finding(
        finding(
            rule,
            storey.clone(),
            Severity::Warning,
            message(
                SpaceCategory::UnallocatedArea,
                &format!(
                    "{:.3}% of the storey's gross floor area ({area:.3} m2 of {gross:.3} m2) \
                 belongs to no space; required at most {}%",
                    lower * 100.0,
                    maximum * 100.0
                ),
            ),
            evidence,
        )
        .with_related(
            regions
                .iter()
                .flat_map(|region| region.elements().iter().cloned()),
        ),
        Deviation::above(maximum, lower, upper),
    );
}

/// Judges each unallocated region on its own against the allowance, so a
/// large hole is found while small shafts beside it pass. The deviation is
/// the region's excess over the allowance, so severity bands can grade it.
fn check_residuals(
    service: &dyn SpaceService,
    policy: &Policy,
    rule: &CompiledRule,
    evidence: &Evidence,
    evaluation: &mut CapabilityEvaluation,
) {
    match service.measure_unallocated_regions() {
        Ok(regions) => {
            let allowance = policy.maximum_unallocated_area_square_metres;
            for region in &regions {
                let area = region.area_square_metres();
                if area > allowance {
                    evaluation.push_graded_finding(
                        finding(
                            rule,
                            region.storey().clone(),
                            Severity::Warning,
                            message(
                                SpaceCategory::UnallocatedArea,
                                &format!(
                                    "a region of {area:.3} m2 of storey floor belongs to no space \
                                 (allowed {allowance:.3} m2)"
                                ),
                            ),
                            evidence,
                        )
                        // The bodies around the region, so a reviewer can
                        // find where it lies.
                        .with_related(region.elements().iter().cloned()),
                        Deviation::above(allowance, area, area),
                    );
                }
            }
            if let Some(maximum) = policy.maximum_unallocated_share {
                let mut per_storey: BTreeMap<&ObjectId, Vec<&UnallocatedRegion>> = BTreeMap::new();
                for region in &regions {
                    per_storey.entry(region.storey()).or_default().push(region);
                }
                for storey in per_storey.values() {
                    check_unallocated_share(storey, maximum, rule, evidence, evaluation);
                }
            }
        }
        Err(error) => {
            evaluation.push_not_evaluated(reason(&error), error.to_string());
        }
    }
}
