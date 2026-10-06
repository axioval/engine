//! Judgements over plan-projected areas: area ranges and plan coverage, and
//! the area readers `area-ratio` shares.

use axioval_engine::template::Template;
use axioval_engine::{
    BodyVolume, CapabilityEvaluation, CompiledRule, Deviation, FacadeArea, FacadeAreaError,
    FacadeAreaServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanArea,
    PlanAreaError, PlanAreaServiceHandle, ProximityServiceHandle, RuleCapability, RuleContext,
};
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};
#[cfg(feature = "parity-reference")]
use axioval_ir::{ReportColumn, ReportTable, RuleId};

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::AreaMeasures;

use crate::counts::{Population, tally};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Measure {
    /// The plan footprint, from the plan-area service.
    Footprint,
    /// The outward-facing surface, from the facade-area service.
    Facade,
}

impl Measure {
    /// The `measure` parameter: `footprint` (the default) or `facade`.
    #[cfg(feature = "parity-reference")]
    fn parse(parameters: &Parameters<'_>) -> Result<Self, Unavailable> {
        Ok(Self::named(parameters, "measure")?.unwrap_or(Self::Footprint))
    }

    /// A measure parameter named `name`, if declared.
    #[cfg(feature = "parity-reference")]
    fn named(parameters: &Parameters<'_>, name: &str) -> Result<Option<Self>, Unavailable> {
        match parameters.string(name)? {
            None => Ok(None),
            Some("footprint") => Ok(Some(Self::Footprint)),
            Some("facade") => Ok(Some(Self::Facade)),
            Some(other) => Err(invalid(format!(
                "{name} `{other}` is unsupported; use `footprint` or `facade`"
            ))),
        }
    }

    /// The numerator's and the denominator's measures: `measure` for both,
    /// or `numerator_measure` and `denominator_measure` each (defaulting to
    /// the footprint), never `measure` beside them.
    #[cfg(feature = "parity-reference")]
    pub(crate) fn sides(parameters: &Parameters<'_>) -> Result<(Self, Self), Unavailable> {
        let both = Self::named(parameters, "measure")?;
        let top = Self::named(parameters, "numerator_measure")?;
        let bottom = Self::named(parameters, "denominator_measure")?;
        if both.is_some() && (top.is_some() || bottom.is_some()) {
            return Err(invalid(
                "`measure` applies to both sides; declare it or `numerator_measure` and \
                 `denominator_measure`, not both",
            ));
        }
        let both = both.unwrap_or(Self::Footprint);
        Ok((top.unwrap_or(both), bottom.unwrap_or(both)))
    }

    #[cfg(feature = "parity-reference")]
    pub(crate) fn noun(self) -> &'static str {
        match self {
            Self::Footprint => "plan area",
            Self::Facade => "facade area",
        }
    }

    /// The report-table column holding an area of this measure.
    #[cfg(feature = "parity-reference")]
    fn column(self) -> &'static str {
        match self {
            Self::Footprint => "plan_area",
            Self::Facade => "facade_area",
        }
    }
}

/// A report table with valid, fixed columns.
#[cfg(feature = "parity-reference")]
pub(crate) fn table(rule: &RuleId, name: &str, columns: Vec<ReportColumn>) -> ReportTable {
    ReportTable::new(rule.clone(), name, columns).expect("fixed table columns are valid")
}

#[cfg(feature = "parity-reference")]
pub(crate) fn area_column(id: &str) -> ReportColumn {
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
    pub(crate) fn measured(
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
            let object = crate::selection::object_by_id(context, id)
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
/// fails the value it names. Each bound is the one comparison every rule
/// decides with (`axioval_engine::comparison::numbers`, exact): the end
/// nearest to passing fails the bound or the interval does not, and the
/// other end decides whether it passes or straddles. A value that is no
/// number leaves the interval undecided.
pub(crate) fn judge_bounds(
    lower: f64,
    upper: f64,
    minimum: Option<Bound>,
    maximum: Option<Bound>,
) -> Verdict {
    use axioval_engine::comparison::{Order, Tolerance, numbers};
    // The bound is worded only for a verdict that names it.
    let bounds = [
        minimum.map(|Bound { value, exclusive }| {
            let (order, words) = if exclusive {
                (Order::Greater, "more than")
            } else {
                (Order::GreaterOrEqual, "at least")
            };
            (order, value, words, upper, lower)
        }),
        maximum.map(|Bound { value, exclusive }| {
            let (order, words) = if exclusive {
                (Order::Less, "less than")
            } else {
                (Order::LessOrEqual, "at most")
            };
            (order, value, words, lower, upper)
        }),
    ];
    let holds =
        |order, end: f64, value: f64| numbers(order, (end, end), (value, value), &Tolerance::EXACT);
    for (order, value, words, nearest, farthest) in bounds.into_iter().flatten() {
        match (holds(order, nearest, value), holds(order, farthest, value)) {
            (Ok(false), _) => return Verdict::Fail(format!("{words} {value}")),
            (Ok(true), Ok(true)) => {}
            _ => return Verdict::Undecided(format!("{words} {value}")),
        }
    }
    Verdict::Pass
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
    traversal: Option<&crate::support::Traversal>,
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
///
/// It runs as a template ([`axioval_engine::template`]): the measured
/// `plan_area` of the object, or summed by an aggregate over the anchor's
/// members, judged by the generic range judge and graded.
pub struct PlanAreaRange;

static TEMPLATE: std::sync::LazyLock<Template> = std::sync::LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for PlanAreaRange {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn grades_deviation(&self) -> bool {
        TEMPLATE.grades
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
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
pub(crate) type Measured = (Sum, Option<(Vec<ObjectId>, usize)>);

/// The summed areas of the members `anchor` reaches, with the decided
/// members and the number of undecided ones.
pub(crate) fn member_areas(
    context: &RuleContext<'_>,
    traversal: Option<&crate::support::Traversal>,
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

#[cfg(test)]
mod tests {
    use super::{Bound, Verdict, judge_bounds};

    /// The range judge before it decided through the shared comparison.
    fn replaced(lower: f64, upper: f64, minimum: Option<Bound>, maximum: Option<Bound>) -> Verdict {
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

    fn shown(verdict: &Verdict) -> String {
        match verdict {
            Verdict::Pass => "pass".into(),
            Verdict::Fail(bound) => format!("fail {bound}"),
            Verdict::Undecided(bound) => format!("undecided {bound}"),
        }
    }

    /// Every interval (reversed and unbounded ones included) against
    /// every pair of bounds is judged as before.
    #[test]
    fn the_shared_comparison_judges_bounds_as_the_range_judge_did() {
        let values = [
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            0.5,
            1.0,
            1.0 + f64::EPSILON,
            2.0,
            f64::INFINITY,
        ];
        let bounds: Vec<Option<Bound>> = std::iter::once(None)
            .chain(values.iter().flat_map(|value| {
                [false, true].map(|exclusive| {
                    Some(Bound {
                        value: *value,
                        exclusive,
                    })
                })
            }))
            .collect();
        for lower in values {
            for upper in values {
                for minimum in &bounds {
                    for maximum in &bounds {
                        assert_eq!(
                            shown(&judge_bounds(lower, upper, *minimum, *maximum)),
                            shown(&replaced(lower, upper, *minimum, *maximum)),
                            "{lower} {upper} {minimum:?} {maximum:?}"
                        );
                    }
                }
            }
        }
    }
}
