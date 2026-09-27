//! Judgements over plan-projected areas: area ranges, ratios and plan coverage.

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, FacadeAreaError, FacadeAreaServiceHandle,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanArea, PlanAreaError,
    PlanAreaServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::counts::{Population, relation_text, tally};
use crate::selection::select_objects;
use crate::support::{
    Parameters, PropertyRef, Unavailable, display, finding, invalid, resolve, traversal_parameters,
};

fn service<'a>(context: &RuleContext<'a>) -> Result<&'a PlanAreaServiceHandle, Unavailable> {
    context.services.get::<PlanAreaServiceHandle>().ok_or((
        NotEvaluatedReason::MissingService,
        "plan-area service is not registered".into(),
    ))
}

#[allow(clippy::needless_pass_by_value)]
fn unavailable(error: PlanAreaError) -> Unavailable {
    let reason = match error {
        PlanAreaError::Unavailable(_) | PlanAreaError::UnknownObject(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        PlanAreaError::InvalidMeasurement | PlanAreaError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

fn facade_service<'a>(
    context: &RuleContext<'a>,
) -> Result<&'a FacadeAreaServiceHandle, Unavailable> {
    context.services.get::<FacadeAreaServiceHandle>().ok_or((
        NotEvaluatedReason::MissingService,
        "facade-area service is not registered".into(),
    ))
}

#[allow(clippy::needless_pass_by_value)]
fn facade_unavailable(error: FacadeAreaError) -> Unavailable {
    let reason = match error {
        FacadeAreaError::Unavailable(_) | FacadeAreaError::UnknownObject(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        FacadeAreaError::InvalidMeasurement | FacadeAreaError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

/// Which area of an object is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Measure {
    /// The plan footprint, from the plan-area service.
    Footprint,
    /// The outward-facing surface, from the facade-area service.
    Facade,
}

impl Measure {
    /// The `measure` parameter: `footprint` (the default) or `facade`.
    fn parse(parameters: &Parameters<'_>) -> Result<Self, Unavailable> {
        match parameters.string("measure")? {
            None | Some("footprint") => Ok(Self::Footprint),
            Some("facade") => Ok(Self::Facade),
            Some(other) => Err(invalid(format!(
                "measure `{other}` is unsupported; use `footprint` or `facade`"
            ))),
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Footprint => "plan area",
            Self::Facade => "facade area",
        }
    }
}

/// A sum of areas as an interval, with every measurement's evidence.
#[derive(Default)]
pub(crate) struct Sum {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
    pub(crate) evidence: Vec<Evidence>,
}

impl Sum {
    pub(crate) fn add(&mut self, area: &PlanArea) {
        self.lower += area.lower_square_metres();
        self.upper += area.upper_square_metres();
        self.evidence.push(area.evidence().clone());
    }

    fn facades(context: &RuleContext<'_>, objects: &[ObjectId]) -> Result<Self, Unavailable> {
        let service = facade_service(context)?;
        let mut sum = Self::default();
        for object in objects {
            let area = service
                .measure_facade_area(object)
                .map_err(facade_unavailable)?;
            sum.lower += area.lower_square_metres();
            sum.upper += area.upper_square_metres();
            sum.evidence.push(area.evidence().clone());
        }
        Ok(sum)
    }

    fn footprints(
        service: &PlanAreaServiceHandle,
        objects: &[ObjectId],
    ) -> Result<Self, Unavailable> {
        let mut sum = Self::default();
        for object in objects {
            sum.add(&service.measure_footprint(object).map_err(unavailable)?);
        }
        Ok(sum)
    }

    /// The summed areas of `objects`: stated by `property` when declared,
    /// otherwise measured as plan footprints.
    pub(crate) fn areas(
        context: &RuleContext<'_>,
        property: Option<PropertyRef<'_>>,
        objects: &[ObjectId],
    ) -> Result<Self, Unavailable> {
        Self::measured(context, property, Measure::Footprint, objects)
    }

    /// The summed areas of `objects`: stated by `property` when declared,
    /// otherwise measured as `measure` says.
    fn measured(
        context: &RuleContext<'_>,
        property: Option<PropertyRef<'_>>,
        measure: Measure,
        objects: &[ObjectId],
    ) -> Result<Self, Unavailable> {
        let Some(property) = property else {
            return match measure {
                Measure::Footprint => Self::footprints(service(context)?, objects),
                Measure::Facade => Self::facades(context, objects),
            };
        };
        let mut sum = Self::default();
        for id in objects {
            let object = context
                .project
                .object(id)
                .ok_or_else(|| invalid(format!("{id} is not in the project")))?;
            let resolved = resolve(context, object, property)?;
            let Some(PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Area,
            }) = resolved.value()
            else {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{id} states no area {property} ({})",
                        display(resolved.value())
                    ),
                ));
            };
            sum.lower += value;
            sum.upper += value;
            sum.evidence.extend(resolved.evidence());
        }
        Ok(sum)
    }
}

/// A ratio interval as a reviewer reads it.
pub(crate) fn shown(lower: f64, upper: f64) -> String {
    let round = |value: f64| (value * 1e4).round() / 1e4;
    #[allow(clippy::float_cmp)]
    if round(lower) == round(upper) {
        format!("{}", round(lower))
    } else {
        format!("between {} and {}", round(lower), round(upper))
    }
}

/// Where a ratio interval stands against inclusive bounds.
pub(crate) enum Verdict {
    Pass,
    Fail(String),
    Undecided(String),
}

pub(crate) fn judge(lower: f64, upper: f64, minimum: Option<f64>, maximum: Option<f64>) -> Verdict {
    if let Some(minimum) = minimum {
        if upper < minimum {
            return Verdict::Fail(format!("at least {minimum}"));
        }
        if lower < minimum {
            return Verdict::Undecided(format!("at least {minimum}"));
        }
    }
    if let Some(maximum) = maximum {
        if lower > maximum {
            return Verdict::Fail(format!("at most {maximum}"));
        }
        if upper > maximum {
            return Verdict::Undecided(format!("at most {maximum}"));
        }
    }
    Verdict::Pass
}

/// Requires the plan area of one population to stand in a ratio to another.
///
/// For each anchor, the footprints of the objects `numerator_selector` picks
/// are summed and divided by the summed footprints of those
/// `denominator_selector` picks, or by the anchor's own footprint when that
/// is not declared. Members are reached as in `related-count`: through the
/// declared relationship, or everywhere in the anchor's source. The ratio
/// must lie within `minimum` and `maximum`, inclusive; at least one is
/// required. Footprints are summed, so overlapping members count twice;
/// select members that do not overlap, such as spaces.
///
/// `numerator_property` or `denominator_property` takes that population's
/// areas from an area-quantity property instead of geometry: a window's
/// glazing area is not its plan footprint.
///
/// `measure: facade` measures the outward-facing surface of each object
/// instead of its footprint, through the facade-area service: the
/// window-to-wall ratio of a storey is the facade area of its windows over
/// that of its external walls and windows (walls are measured with their
/// openings cut out, so the windows belong in the denominator too).
///
/// Areas are intervals, so the ratio is too. An anchor is judged only when
/// the whole interval is on one side of a bound; one straddling it, an
/// undecided member, or a zero denominator is not evaluated.
pub struct AreaRatio;

impl RuleCapability for AreaRatio {
    fn id(&self) -> &'static str {
        "axioval:capability.area-ratio"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("numerator_selector", ParameterType::Selector),
            ParameterDescriptor::optional("denominator_selector", ParameterType::Selector),
            ParameterDescriptor::optional("minimum", ParameterType::Number),
            ParameterDescriptor::optional("maximum", ParameterType::Number),
            ParameterDescriptor::optional("numerator_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("denominator_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("measure", ParameterType::String),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.number("minimum")?;
            let maximum = parameters.number("maximum")?;
            if minimum.is_none() && maximum.is_none() {
                return Err(invalid("minimum or maximum is required"));
            }
            if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
                return Err(invalid("minimum exceeds maximum"));
            }
            Ok::<_, Unavailable>((
                parameters.required_selector("numerator_selector")?,
                parameters.selector("denominator_selector")?,
                parameters.property("numerator_property")?,
                parameters.property("denominator_property")?,
                (minimum, maximum),
                Measure::parse(&parameters)?,
                parameters.traversal()?,
            ))
        })();
        let (numerator, denominator, top_area, bottom_area, (minimum, maximum), measure, traversal) =
            match parsed {
                Ok(parsed) => parsed,
                Err((reason, message)) => {
                    return CapabilityEvaluation::not_evaluated(
                        reason,
                        format!("area-ratio: {message}"),
                    );
                }
            };
        let numerator = Population::of(context, numerator);
        let denominator = denominator.map(|selector| Population::of(context, selector));
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let via = relation_text(traversal.as_ref());
        for anchor in anchors {
            let judged = (|| {
                let over = tally(context, traversal.as_ref(), anchor, &numerator)?;
                let under = match &denominator {
                    Some(population) => {
                        Some(tally(context, traversal.as_ref(), anchor, population)?)
                    }
                    None => None,
                };
                let undecided = over.undecided + under.as_ref().map_or(0, |under| under.undecided);
                if undecided > 0 {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("{undecided} related object(s) {via} cannot be assigned"),
                    ));
                }
                let mut top = Sum::measured(context, top_area, measure, &over.decided)?;
                top.evidence.extend(over.evidence);
                let mut bottom = match &under {
                    Some(under) => Sum::measured(context, bottom_area, measure, &under.decided)?,
                    None => Sum::measured(
                        context,
                        bottom_area,
                        measure,
                        std::slice::from_ref(&anchor.id),
                    )?,
                };
                if let Some(under) = under {
                    bottom.evidence.extend(under.evidence);
                }
                if bottom.upper <= 0.0 {
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("the denominator has no {}", measure.noun()),
                    ));
                }
                let lower = top.lower / bottom.upper;
                let upper = if bottom.lower > 0.0 {
                    top.upper / bottom.lower
                } else {
                    f64::INFINITY
                };
                let mut evidence = top.evidence;
                evidence.extend(bottom.evidence);
                Ok((
                    lower,
                    upper,
                    top.lower,
                    bottom.lower,
                    evidence,
                    over.decided,
                ))
            })();
            let (lower, upper, area, of, evidence, members) = match judged {
                Ok(judged) => judged,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                    continue;
                }
            };
            match judge(lower, upper, minimum, maximum) {
                Verdict::Pass => {}
                Verdict::Fail(bound) => evaluation.push_finding(finding(
                    rule,
                    &anchor.id,
                    format!(
                        "{} ratio is {} ({} m² of {} m²); required {bound}",
                        measure.noun(),
                        shown(lower, upper),
                        (area * 100.0).round() / 100.0,
                        (of * 100.0).round() / 100.0,
                    ),
                    evidence,
                    members,
                )),
                Verdict::Undecided(bound) => evaluation.push_object_not_evaluated(
                    anchor.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{} ratio is {}, which straddles the bound {bound}",
                        measure.noun(),
                        shown(lower, upper)
                    ),
                ),
            }
        }
        evaluation
    }
}

/// Requires each subject's footprint to lie mostly within one candidate.
///
/// A space must lie within a fire compartment: for each subject, the share
/// of its footprint that overlaps a candidate (an object `candidate_selector`
/// picks, reached through the declared relationship or anywhere in the
/// subject's source) must reach `minimum_ratio` for at least one candidate.
/// The subject fails when no candidate can reach it, and is not evaluated
/// when one might, given the areas' intervals.
pub struct PlanCoverage;

impl RuleCapability for PlanCoverage {
    fn id(&self) -> &'static str {
        "axioval:capability.plan-coverage"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("candidate_selector", ParameterType::Selector),
            ParameterDescriptor::required("minimum_ratio", ParameterType::Number),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.number("minimum_ratio")?;
            match minimum {
                Some(minimum) if minimum > 0.0 && minimum <= 1.0 => {}
                _ => return Err(invalid("minimum_ratio must lie in (0, 1]")),
            }
            Ok::<_, Unavailable>((
                parameters.required_selector("candidate_selector")?,
                minimum.unwrap_or(1.0),
                parameters.traversal()?,
            ))
        })();
        let (candidates, minimum, traversal) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("plan-coverage: {message}"),
                );
            }
        };
        let candidates = Population::of(context, candidates);
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        for subject in subjects {
            match coverage(context, traversal.as_ref(), subject, &candidates, minimum) {
                Ok(None) => {}
                Ok(Some((message, evidence, best))) => evaluation.push_finding(finding(
                    rule,
                    &subject.id,
                    message,
                    evidence,
                    best.into_iter().collect(),
                )),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

type Failure = (String, Vec<Evidence>, Option<ObjectId>);

/// `None` when covered; the finding when conclusively not.
fn coverage(
    context: &RuleContext<'_>,
    traversal: Option<&crate::support::Traversal<'_>>,
    subject: &Object,
    candidates: &Population,
    minimum: f64,
) -> Result<Option<Failure>, Unavailable> {
    let service = service(context)?;
    let reached = tally(context, traversal, subject, candidates)?;
    let footprint = service
        .measure_footprint(&subject.id)
        .map_err(unavailable)?;
    if footprint.upper_square_metres() <= 0.0 {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            "the subject has no plan footprint".into(),
        ));
    }
    let mut evidence = reached.evidence;
    evidence.push(footprint.evidence().clone());
    let mut best: Option<(f64, ObjectId)> = None;
    let mut undecided = reached.undecided > 0;
    for candidate in &reached.decided {
        let overlap = service
            .measure_plan_overlap(&subject.id, candidate)
            .map_err(unavailable)?;
        evidence.push(overlap.evidence().clone());
        let lower = overlap.lower_square_metres() / footprint.upper_square_metres();
        let upper = if footprint.lower_square_metres() > 0.0 {
            overlap.upper_square_metres() / footprint.lower_square_metres()
        } else {
            f64::INFINITY
        };
        if lower >= minimum {
            return Ok(None);
        }
        if upper >= minimum {
            undecided = true;
        }
        if best.as_ref().is_none_or(|(held, _)| upper > *held) {
            best = Some((upper, candidate.clone()));
        }
    }
    if undecided {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("coverage of {minimum} cannot be decided from the measured areas"),
        ));
    }
    let share = best.as_ref().map_or(0.0, |(share, _)| *share);
    Ok(Some((
        format!(
            "at most {} of the footprint lies within any {}; required {minimum}",
            (share.min(1.0) * 1e4).round() / 1e4,
            if reached.decided.is_empty() {
                "candidate (there are none)".to_owned()
            } else {
                "candidate".to_owned()
            }
        ),
        evidence,
        best.map(|(_, candidate)| candidate),
    )))
}

/// Requires measured plan areas to lie within a range, in square metres.
///
/// Without `member_selector`, each selected object's own footprint must lie
/// within `minimum` and `maximum`, inclusive; at least one is required: a
/// space of at least 8 m², a fire compartment of at most 400 m².
///
/// With `member_selector`, each selected object is an anchor, and the summed
/// footprints of the members it reaches (through the declared relationship,
/// or everywhere in its source, as in `related-count`) must lie within the
/// range: the space area of each storey. Footprints are summed, so
/// overlapping members count twice; select members that do not overlap.
///
/// Areas are intervals. A verdict needs the whole interval on one side of a
/// bound; one straddling it is not evaluated. An object with no plan
/// footprint (no body) is not evaluated, and so is an anchor with such a
/// member. An anchor with members whose selection is undecided is judged
/// only when they cannot change the verdict: they can only add area, so a
/// sum already above the maximum stands.
///
/// `measure: facade` bounds the outward-facing surface instead, through the
/// facade-area service: the facade area of each storey, summed over the
/// external walls it contains. A facade area may be zero; a footprint may
/// not, since an empty one means the object has no body.
pub struct PlanAreaRange;

impl RuleCapability for PlanAreaRange {
    fn id(&self) -> &'static str {
        "axioval:capability.plan-area"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::optional("minimum", ParameterType::Number),
            ParameterDescriptor::optional("maximum", ParameterType::Number),
            ParameterDescriptor::optional("member_selector", ParameterType::Selector),
            ParameterDescriptor::optional("measure", ParameterType::String),
        ]
        .into_iter()
        .chain(traversal_parameters())
        .collect()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let parameters = Parameters(rule);
        let parsed = (|| {
            let minimum = parameters.number("minimum")?;
            let maximum = parameters.number("maximum")?;
            if minimum.is_none() && maximum.is_none() {
                return Err(invalid("minimum or maximum is required"));
            }
            if minimum.is_some_and(|value| value < 0.0) || maximum.is_some_and(|value| value < 0.0)
            {
                return Err(invalid("an area bound is negative"));
            }
            if matches!((minimum, maximum), (Some(low), Some(high)) if low > high) {
                return Err(invalid("minimum exceeds maximum"));
            }
            let members = parameters.selector("member_selector")?;
            let traversal = parameters.traversal()?;
            if members.is_none() && traversal.is_some() {
                return Err(invalid(
                    "a relationship reaches members only with `member_selector`",
                ));
            }
            let measure = Measure::parse(&parameters)?;
            Ok::<_, Unavailable>((minimum, maximum, members, traversal, measure))
        })();
        let (minimum, maximum, members, traversal, measure) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("plan-area: {message}"),
                );
            }
        };
        let members = members.map(|selector| Population::of(context, selector));
        let what = if members.is_some() {
            format!("summed {} of the members", measure.noun())
        } else {
            measure.noun().to_owned()
        };
        let (subjects, mut evaluation) = select_objects(context, &rule.selector);
        for subject in subjects {
            let measured = match &members {
                None => own_area(context, measure, &subject.id).map(|sum| (sum, None)),
                Some(population) => {
                    member_areas(context, traversal.as_ref(), subject, population, measure)
                }
            };
            let (sum, reached) = match measured {
                Ok(measured) => measured,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                    continue;
                }
            };
            let (related, undecided) = reached.unwrap_or_default();
            // Undecided members can only add area: only an excess stands.
            if undecided > 0 && !maximum.is_some_and(|maximum| sum.lower > maximum) {
                evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{undecided} member(s) {} cannot be assigned",
                        relation_text(traversal.as_ref())
                    ),
                );
                continue;
            }
            match judge(sum.lower, sum.upper, minimum, maximum) {
                Verdict::Pass => {}
                Verdict::Fail(bound) => evaluation.push_finding(finding(
                    rule,
                    &subject.id,
                    format!(
                        "{what} is {} m²; required {bound} m²",
                        shown(sum.lower, sum.upper)
                    ),
                    sum.evidence,
                    related,
                )),
                Verdict::Undecided(bound) => evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "{what} is {} m², which straddles the bound {bound} m²",
                        shown(sum.lower, sum.upper)
                    ),
                ),
            }
        }
        evaluation
    }
}

/// The area of one object: a footprint that is not empty, or its facade.
fn own_area(
    context: &RuleContext<'_>,
    measure: Measure,
    object: &ObjectId,
) -> Result<Sum, Unavailable> {
    match measure {
        Measure::Footprint => {
            let mut sum = Sum::default();
            sum.add(&footprint(context, object)?);
            Ok(sum)
        }
        Measure::Facade => Sum::facades(context, std::slice::from_ref(object)),
    }
}

/// A footprint that is not empty: an empty one means the object has no body.
pub(crate) fn footprint(
    context: &RuleContext<'_>,
    object: &ObjectId,
) -> Result<PlanArea, Unavailable> {
    let area = service(context)?
        .measure_footprint(object)
        .map_err(unavailable)?;
    if area.upper_square_metres() <= 0.0 {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{object} has no plan footprint (no body)"),
        ));
    }
    Ok(area)
}

/// A summed area, with the members summed and the number left undecided
/// when members were summed.
type Measured = (Sum, Option<(Vec<ObjectId>, usize)>);

/// The summed areas of the members `anchor` reaches, with the decided
/// members and the number of undecided ones.
fn member_areas(
    context: &RuleContext<'_>,
    traversal: Option<&crate::support::Traversal<'_>>,
    anchor: &Object,
    population: &Population,
    measure: Measure,
) -> Result<Measured, Unavailable> {
    let reached = tally(context, traversal, anchor, population)?;
    let mut sum = Sum::default();
    for member in &reached.decided {
        let area = own_area(context, measure, member)?;
        sum.lower += area.lower;
        sum.upper += area.upper;
        sum.evidence.extend(area.evidence);
    }
    sum.evidence.extend(reached.evidence);
    Ok((sum, Some((reached.decided, reached.undecided))))
}
