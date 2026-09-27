//! Judgements over plan-projected areas: area ranges, ratios and plan coverage.

use axioval_engine::{
    BodyVolume, CapabilityEvaluation, CompiledRule, Deviation, FacadeArea, FacadeAreaError,
    FacadeAreaServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanArea,
    PlanAreaError, PlanAreaServiceHandle, ProximityServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::{
    Evidence, Object, ObjectId, PropertyValue, QuantityDimension, ReportColumn, ReportTable,
    ReportValue, RuleId,
};

use crate::counts::{Population, relation_text, tally};
use crate::light_area::LightArea;
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
pub(crate) fn unavailable(error: PlanAreaError) -> Unavailable {
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

    /// The report-table column holding an area of this measure.
    fn column(self) -> &'static str {
        match self {
            Self::Footprint => "plan_area",
            Self::Facade => "facade_area",
        }
    }
}

/// A report table with valid, fixed columns.
fn table(rule: &RuleId, name: &str, columns: Vec<ReportColumn>) -> ReportTable {
    ReportTable::new(rule.clone(), name, columns).expect("fixed table columns are valid")
}

fn area_column(id: &str) -> ReportColumn {
    ReportColumn::quantity(id, QuantityDimension::Area)
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

/// How far an interval failing [`judge`] misses the bound it fails,
/// relative to that bound; `None` when it fails neither.
pub(crate) fn deviation(
    lower: f64,
    upper: f64,
    minimum: Option<f64>,
    maximum: Option<f64>,
) -> Option<Deviation> {
    if let Some(minimum) = minimum
        && upper < minimum
    {
        return Some(Deviation::below(minimum, lower, upper));
    }
    if let Some(maximum) = maximum
        && lower > maximum
    {
        return Some(Deviation::above(maximum, lower, upper));
    }
    None
}

/// Where a measured interval stands against its bounds.
pub(crate) enum Verdict {
    Pass,
    Fail(String),
    Undecided(String),
}

pub(crate) fn judge(lower: f64, upper: f64, minimum: Option<f64>, maximum: Option<f64>) -> Verdict {
    judge_bounds(
        lower,
        upper,
        minimum.map(Bound::inclusive),
        maximum.map(Bound::inclusive),
    )
}

/// A bound, and whether it excludes its own value.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bound {
    pub(crate) value: f64,
    pub(crate) exclusive: bool,
}

impl Bound {
    pub(crate) fn inclusive(value: f64) -> Self {
        Self {
            value,
            exclusive: false,
        }
    }
}

/// Where an interval stands against bounds that may exclude their value: a
/// verdict needs the whole interval on one side, and an exclusive bound
/// fails the value it names.
pub(crate) fn judge_bounds(
    lower: f64,
    upper: f64,
    minimum: Option<Bound>,
    maximum: Option<Bound>,
) -> Verdict {
    if let Some(Bound { value, exclusive }) = minimum {
        let text = if exclusive {
            format!("more than {value}")
        } else {
            format!("at least {value}")
        };
        let below = |measured: f64| measured < value || (exclusive && measured <= value);
        if below(upper) {
            return Verdict::Fail(text);
        }
        if below(lower) {
            return Verdict::Undecided(text);
        }
    }
    if let Some(Bound { value, exclusive }) = maximum {
        let text = if exclusive {
            format!("less than {value}")
        } else {
            format!("at most {value}")
        };
        let above = |measured: f64| measured > value || (exclusive && measured >= value);
        if above(lower) {
            return Verdict::Fail(text);
        }
        if above(upper) {
            return Verdict::Undecided(text);
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
/// With `numerator_derivation` `light-area`, each numerator member's area is
/// its light-transmitting area, taken from the first step of a fallback
/// that produces one: the area `numerator_property` states, else the
/// `light_area` of the most specific `light_area_table` row whose `width`
/// and `height` equal the member's `overall_width` and `overall_height`
/// (within `light_size_tolerance`) and whose `type` pattern matches the
/// `light_type` name (read from the member or, with `light_type_path`, from
/// the objects that path reaches), else the overall width × height less the
/// frame allowance 2·(W+H)·`frame_width`. A step is skipped only when its
/// input is exactly absent; a value of the wrong kind, an unknown type name
/// a row tests, or tied rows stop the chain, and a member the chain cannot
/// give an area leaves its anchor not evaluated. Evidence records the step
/// behind each area. A stated light area larger than the member's overall
/// area is a finding against the member, and its anchor is not evaluated.
///
/// With `empty_numerator_finding`, an anchor that reaches no numerator
/// object (a space with no window) is a finding of its own instead of a
/// ratio of 0.
///
/// `measure: facade` together with `light-area` is an invalid declaration:
/// a light area over facade areas is no defined ratio, and a
/// window-to-wall ratio measures its windows' facade areas instead.
///
/// Areas are intervals, so the ratio is too. An anchor is judged only when
/// the whole interval is on one side of a bound; one straddling it, an
/// undecided member, or a zero denominator is not evaluated.
///
/// Every run reports the table `ratios`, one row per anchor whose ratio was
/// measured, passing or not: `numerator_area`, `denominator_area` and
/// `ratio` (unknown when the denominator may be zero).
pub struct AreaRatio;

impl RuleCapability for AreaRatio {
    fn id(&self) -> &'static str {
        "axioval:capability.area-ratio"
    }

    fn grades_deviation(&self) -> bool {
        true
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
            ParameterDescriptor::optional("numerator_derivation", ParameterType::String),
            ParameterDescriptor::optional("empty_numerator_finding", ParameterType::Boolean),
        ]
        .into_iter()
        .chain(crate::light_area::parameters())
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
            let top_area = parameters.property("numerator_property")?;
            Ok::<_, Unavailable>((
                parameters.required_selector("numerator_selector")?,
                parameters.selector("denominator_selector")?,
                top_area,
                LightArea::parse(&parameters, top_area)?,
                parameters
                    .boolean("empty_numerator_finding")?
                    .unwrap_or(false),
                parameters.property("denominator_property")?,
                (minimum, maximum),
                Measure::parse(&parameters)?,
                parameters.traversal()?,
            ))
        })();
        let (
            numerator,
            denominator,
            top_area,
            light,
            report_empty,
            bottom_area,
            (minimum, maximum),
            measure,
            traversal,
        ) = match parsed {
            Ok(parsed) => parsed,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("area-ratio: {message}"),
                );
            }
        };
        if light.is_some() && measure == Measure::Facade {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::InvalidDeclaration,
                "area-ratio: `measure` `facade` does not combine with `numerator_derivation` \
                 `light-area`"
                    .to_owned(),
            );
        }
        let numerator = Population::of(context, numerator);
        // Members already reported, so one reached by several anchors is
        // reported once.
        let mut reported = std::collections::BTreeSet::new();
        let denominator = denominator.map(|selector| Population::of(context, selector));
        let (anchors, mut evaluation) = select_objects(context, &rule.selector);
        let via = relation_text(traversal.as_ref());
        let mut ratios = table(
            &rule.id,
            "ratios",
            vec![
                area_column("numerator_area"),
                area_column("denominator_area"),
                ReportColumn::number("ratio"),
            ],
        );
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
                if report_empty && over.decided.is_empty() {
                    return Ok(Judged::Empty(over.evidence));
                }
                let (mut top, provenance) = match &light {
                    None => (
                        Sum::measured(context, top_area, measure, &over.decided)?,
                        String::new(),
                    ),
                    Some(light) => {
                        let summed = light.sum(context, &over.decided);
                        for (member, message, evidence) in &summed.oversized {
                            if reported.insert(member.clone()) {
                                evaluation.push_finding(finding(
                                    rule,
                                    member,
                                    message.clone(),
                                    evidence.clone(),
                                    vec![anchor.id.clone()],
                                ));
                            }
                        }
                        for (member, message) in &summed.unchecked {
                            if reported.insert(member.clone()) {
                                evaluation.push_object_not_evaluated(
                                    member.clone(),
                                    NotEvaluatedReason::IncompleteEvidence,
                                    message.clone(),
                                );
                            }
                        }
                        if let Some(failure) = summed.failure {
                            return Err(failure);
                        }
                        if let Some((member, _, _)) = summed.oversized.first() {
                            return Err((
                                NotEvaluatedReason::InvalidEvidence,
                                format!(
                                    "{} member(s), first {member}, state a light area larger \
                                     than the element",
                                    summed.oversized.len()
                                ),
                            ));
                        }
                        let provenance = summed.provenance();
                        (
                            Sum {
                                lower: summed.lower,
                                upper: summed.upper,
                                evidence: summed.evidence,
                            },
                            provenance,
                        )
                    }
                };
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
                Ok(Judged::Ratio {
                    lower,
                    upper,
                    numerator: (top.lower, top.upper),
                    denominator: (bottom.lower, bottom.upper),
                    provenance,
                    evidence,
                    members: over.decided,
                })
            })();
            let (lower, upper, area, of, provenance, evidence, members) = match judged {
                Ok(Judged::Ratio {
                    lower,
                    upper,
                    numerator,
                    denominator,
                    provenance,
                    evidence,
                    members,
                }) => {
                    // Anchors are distinct objects, so rows never collide.
                    let _ = ratios.push_row(
                        anchor.id.clone(),
                        vec![
                            ReportValue::measured(numerator.0, numerator.1),
                            ReportValue::measured(denominator.0, denominator.1),
                            ReportValue::measured(lower, upper),
                        ],
                    );
                    (
                        lower,
                        upper,
                        numerator.0,
                        denominator.0,
                        provenance,
                        evidence,
                        members,
                    )
                }
                Ok(Judged::Empty(evidence)) => {
                    evaluation.push_finding(finding(
                        rule,
                        &anchor.id,
                        format!("no numerator object is reached {via}; the ratio is 0"),
                        evidence,
                        Vec::new(),
                    ));
                    continue;
                }
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(anchor.id.clone(), reason, message);
                    continue;
                }
            };
            match judge(lower, upper, minimum, maximum) {
                Verdict::Pass => {}
                Verdict::Fail(bound) => evaluation.push_finding_deviating(
                    finding(
                        rule,
                        &anchor.id,
                        format!(
                            "{} ratio is {} ({} m² of {} m²); required {bound}{provenance}",
                            measure.noun(),
                            shown(lower, upper),
                            (area * 100.0).round() / 100.0,
                            (of * 100.0).round() / 100.0,
                        ),
                        evidence,
                        members,
                    ),
                    deviation(lower, upper, minimum, maximum),
                ),
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
        evaluation.push_table(ratios);
        evaluation
    }
}

/// What one anchor of `area-ratio` comes to before the bounds are applied.
enum Judged {
    Ratio {
        lower: f64,
        upper: f64,
        /// The summed numerator and denominator areas, as intervals.
        numerator: (f64, f64),
        denominator: (f64, f64),
        /// Which light-area steps produced the numerator, for the message.
        provenance: String,
        evidence: Vec<Evidence>,
        members: Vec<ObjectId>,
    },
    /// The anchor reaches no numerator object, and that is reported.
    Empty(Vec<Evidence>),
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
///
/// Every run reports the table `areas`, one row per subject whose area was
/// measured, passing or not, in the column `plan_area` or, with `measure:
/// facade`, `facade_area`. A subject with undecided members has no row: its
/// area is known only from below.
pub struct PlanAreaRange;

impl RuleCapability for PlanAreaRange {
    fn id(&self) -> &'static str {
        "axioval:capability.plan-area"
    }

    fn grades_deviation(&self) -> bool {
        true
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
        let mut areas = table(&rule.id, "areas", vec![area_column(measure.column())]);
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
            if undecided == 0 {
                // Subjects are distinct objects, so rows never collide.
                let _ = areas.push_row(
                    subject.id.clone(),
                    vec![ReportValue::measured(sum.lower, sum.upper)],
                );
            }
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
                Verdict::Fail(bound) => evaluation.push_finding_deviating(
                    finding(
                        rule,
                        &subject.id,
                        format!(
                            "{what} is {} m²; required {bound} m²",
                            shown(sum.lower, sum.upper)
                        ),
                        sum.evidence,
                        related,
                    ),
                    deviation(sum.lower, sum.upper, minimum, maximum),
                ),
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
        evaluation.push_table(areas);
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

/// The certified volume an object's closed body encloses.
pub(crate) fn body_volume(
    context: &RuleContext<'_>,
    object: &ObjectId,
) -> Result<BodyVolume, Unavailable> {
    let service = context.services.get::<ProximityServiceHandle>().ok_or((
        NotEvaluatedReason::MissingService,
        "proximity service is not registered".into(),
    ))?;
    service.measure_body_volume(object).map_err(|error| {
        (
            crate::pairs::reason(error),
            format!("the volume of {object} is unavailable: {error}"),
        )
    })
}

/// The area of an object's largest plane face.
pub(crate) fn face_area(
    context: &RuleContext<'_>,
    object: &ObjectId,
) -> Result<FacadeArea, Unavailable> {
    facade_service(context)?
        .measure_face_area(object)
        .map_err(facade_unavailable)
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
