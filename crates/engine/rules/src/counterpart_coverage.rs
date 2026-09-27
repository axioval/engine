//! Coverage and conformity of one set of elements by another: how much of
//! each architectural wall no structural wall stands under, in plan and in
//! height, or in the wall's own elevation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, ElevationCover, ElevationRequest,
    NotEvaluatedReason, ObjectBounds, ParameterDescriptor, ParameterType, PlanArea,
    PlanAreaServiceHandle, PlanRectangle, PlanSpanServiceHandle, ProximityProjection,
    ProximityServiceHandle, RuleCapability, RuleContext, VerticalExtent, VerticalExtentError,
    VerticalExtentServiceHandle, projected_candidate_pairs,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension, Severity};

use crate::orientation::{Alignment, Tri, aligned, angle_tolerance, rectangle, rectangle_service};
use crate::pairs::{reason as proximity_reason, refuse_all};
use crate::plan_area::{footprint, shown, unavailable};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

const NAME: &str = "counterpart-coverage";

/// Requires each selected element to be covered by its counterparts, in plan
/// and in height, graded by the share left uncovered.
///
/// The counterparts are the objects `counterparts` picks, typically another
/// discipline's elements of matching kinds (a `discipline` selector with an
/// entity type): architectural walls against structural walls.
///
/// - **Plan**: the share of the element's footprint outside the union of the
///   counterparts' footprints, each grown by the horizontal tolerance, through
///   the plan-area service's uncovered area.
/// - **Height**: the share of the element's vertical extent outside the union
///   of the vertical extents, each grown by the vertical tolerance, of the
///   counterparts that overlap it in plan (their footprint, grown by the
///   horizontal tolerance, covers some of the element's).
///
/// Two variants differ only in their tolerances: coverage declares one
/// `tolerance` for both checks; conformity declares `horizontal_tolerance`
/// and `vertical_tolerance` separately. A negative tolerance switches its
/// check off; switching every check off is an invalid declaration.
///
/// An uncovered share above `info_above`, `warning_above` or `error_above`
/// (at least one, ascending in that order, each in `[0, 1)`) is a finding of
/// the most severe band it exceeds; the rule's own severity is not used. A
/// share at or below the lowest declared threshold passes.
///
/// Shares are intervals. A share straddling the lowest threshold is not
/// evaluated; one above it that straddles a higher threshold is graded by
/// the most severe band it may reach, and the message says so. Counterparts
/// the selector cannot decide, or whose extent or footprint cannot be read,
/// can only cover more: a pass stands, anything else is not evaluated.
///
/// With `axis_tolerance`, only axis-compatible counterparts count: those
/// whose long axis (from the least-area rectangle of the footprint) lies
/// within that angle of parallel to the element's. A counterpart surely at
/// another angle is left out; one whose angle straddles the tolerance, or
/// whose axes or the element's are not their own (a square, several
/// least-area rectangles, a tessellated footprint), may count: it can only
/// cover more, so it leaves a finding it could remove not evaluated.
/// Without `axis_tolerance`, a perpendicular wall meeting the element
/// within the horizontal tolerance overlaps it in plan and counts towards
/// its height.
///
/// With `measure` `elevation`, plan and height are one check: the share of
/// the element's elevation (its projection onto the vertical plane along the
/// long axis of its footprint's least-area rectangle) that the counterparts'
/// projections leave uncovered. A counterpart counts by its part within the
/// element's depth across the axis, widened by the horizontal tolerance;
/// its projection grows by the horizontal tolerance along the axis and the
/// vertical tolerance in height. A wall under a full-height counterpart on
/// one half and a half-height one on the other passes plan and height, but
/// a quarter of its face is uncovered. An element without a long axis (a
/// square, several least-area rectangles, a tessellated footprint) is not
/// evaluated; both tolerances must be switched on.
///
/// `infill_counterparts` (elevation only) are the members of a frame, such
/// as columns and beams: when more than `infill_above` (a share in `[0, 1)`,
/// half by default) of the elevation is uncovered by the counterparts, the
/// frame's infill (the convex hull of the members' projections, grown the
/// same way) covers too, so a wall filling a column-and-beam bay passes.
/// Whether the infill applies is decided on the share the counterparts
/// leave; a share straddling `infill_above` is judged from both answers.
pub struct CounterpartCoverage;

/// A declared threshold and the severity of the band above it.
type Band = (f64, Severity);

struct Config<'a> {
    counterparts: &'a Selector,
    /// Growth of each counterpart's footprint, `None` when the plan check is off.
    horizontal: Option<f64>,
    /// Growth of each counterpart's extent, `None` when the height check is off.
    vertical: Option<f64>,
    /// Ascending thresholds.
    bands: Vec<Band>,
    /// Largest angle, in degrees, between compatible long axes; `None`
    /// counts counterparts at any angle.
    axis: Option<f64>,
    /// Whether plan and height are measured together in the elevation.
    elevation: bool,
    /// Frame members whose infill covers, and the uncovered share above
    /// which it does.
    infill: Option<(&'a Selector, f64)>,
}

impl RuleCapability for CounterpartCoverage {
    fn id(&self) -> &'static str {
        "axioval:capability.counterpart-coverage"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("counterparts", ParameterType::Selector),
            ParameterDescriptor::optional("tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("horizontal_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("vertical_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("info_above", ParameterType::Number),
            ParameterDescriptor::optional("warning_above", ParameterType::Number),
            ParameterDescriptor::optional("error_above", ParameterType::Number),
            ParameterDescriptor::optional("axis_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("measure", ParameterType::String),
            ParameterDescriptor::optional("infill_counterparts", ParameterType::Selector),
            ParameterDescriptor::optional("infill_above", ParameterType::Number),
        ]
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (subjects, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context, &config) {
            Ok(services) => services,
            Err((reason, message)) => return refuse_all(&subjects, evaluation, &reason, &message),
        };
        // A counterpart within the tolerance along and across the axis may
        // lie that much farther away in plan.
        let margin = config.horizontal.unwrap_or(0.0)
            * if config.elevation {
                std::f64::consts::SQRT_2
            } else {
                1.0
            };
        let found = |selector: &Selector| {
            Counterparts::find(context, selector, margin, &services, &subjects)
        };
        let counterparts = match found(config.counterparts) {
            Ok(counterparts) => counterparts,
            Err((reason, message)) => return refuse_all(&subjects, evaluation, &reason, &message),
        };
        let frame = match config
            .infill
            .map(|(selector, _)| found(selector))
            .transpose()
        {
            Ok(frame) => frame,
            Err((reason, message)) => return refuse_all(&subjects, evaluation, &reason, &message),
        };
        let mut evaluation = evaluation;
        for subject in subjects {
            if let Some((reason, message)) = counterparts.unbounded.get(&subject.id) {
                evaluation.push_object_not_evaluated(
                    subject.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
                continue;
            }
            let checks = Subject {
                context,
                config: &config,
                services: &services,
                counterparts: &counterparts,
                frame: frame.as_ref(),
                object: subject,
            }
            .checks();
            for check in checks {
                match check {
                    Ok(None) => {}
                    Ok(Some(Graded {
                        severity,
                        message,
                        evidence,
                        related,
                        deviation,
                    })) => {
                        let mut found = finding(rule, &subject.id, message, evidence, related);
                        found.severity = severity;
                        evaluation.push_graded_finding(found, deviation);
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(subject.id.clone(), reason, message);
                    }
                }
            }
        }
        evaluation
    }
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
    let length = |name: &str| match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) => Ok(Some(value)),
        Some(_) => Err(invalid(format!("{name} must be a length"))),
    };
    let (horizontal, vertical) = match (
        length("tolerance")?,
        length("horizontal_tolerance")?,
        length("vertical_tolerance")?,
    ) {
        (Some(both), None, None) => (both, both),
        (None, Some(horizontal), Some(vertical)) => (horizontal, vertical),
        (None, None, None) => {
            return Err(invalid(
                "declare `tolerance`, or `horizontal_tolerance` and `vertical_tolerance`",
            ));
        }
        (Some(_), _, _) => {
            return Err(invalid(
                "`tolerance` cannot be combined with `horizontal_tolerance` or \
                 `vertical_tolerance`",
            ));
        }
        _ => {
            return Err(invalid(
                "`horizontal_tolerance` and `vertical_tolerance` are declared together",
            ));
        }
    };
    let on = |tolerance: f64| (tolerance >= 0.0).then_some(tolerance);
    let (horizontal, vertical) = (on(horizontal), on(vertical));
    if horizontal.is_none() && vertical.is_none() {
        return Err(invalid(
            "a negative tolerance switches its check off, and every check is off",
        ));
    }
    let mut bands: Vec<Band> = Vec::new();
    for (name, severity) in [
        ("info_above", Severity::Info),
        ("warning_above", Severity::Warning),
        ("error_above", Severity::Error),
    ] {
        let Some(threshold) = parameters.number(name)? else {
            continue;
        };
        if !(0.0..1.0).contains(&threshold) {
            return Err(invalid(format!("{name} must lie in [0, 1)")));
        }
        if bands.last().is_some_and(|(below, _)| *below >= threshold) {
            return Err(invalid(format!(
                "{name} must exceed the thresholds of less severe bands"
            )));
        }
        bands.push((threshold, severity));
    }
    if bands.is_empty() {
        return Err(invalid(
            "declare at least one of `info_above`, `warning_above` and `error_above`",
        ));
    }
    let elevation = match parameters.string("measure")? {
        None | Some("plan_and_height") => false,
        Some("elevation") => true,
        Some(other) => {
            return Err(invalid(format!(
                "measure `{other}` is unsupported; use `plan_and_height` or `elevation`"
            )));
        }
    };
    if elevation && (horizontal.is_none() || vertical.is_none()) {
        return Err(invalid(
            "the elevation is one check measured with both tolerances; neither may be negative",
        ));
    }
    Ok(Config {
        counterparts: parameters.required_selector("counterparts")?,
        horizontal,
        vertical,
        bands,
        axis: angle_tolerance(parameters, "axis_tolerance")?,
        elevation,
        infill: infill(parameters, elevation)?,
    })
}

/// The frame members whose infill covers, and the share above which.
fn infill<'a>(
    parameters: &Parameters<'a>,
    elevation: bool,
) -> Result<Option<(&'a Selector, f64)>, Unavailable> {
    match (
        parameters.selector("infill_counterparts")?,
        parameters.number("infill_above")?,
    ) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(invalid("`infill_above` needs `infill_counterparts`")),
        (Some(_), _) if !elevation => Err(invalid(
            "`infill_counterparts` applies only to `measure` `elevation`",
        )),
        (Some(selector), above) => {
            let above = above.unwrap_or(0.5);
            if (0.0..1.0).contains(&above) {
                Ok(Some((selector, above)))
            } else {
                Err(invalid("infill_above must lie in [0, 1)"))
            }
        }
    }
}

struct Services<'a> {
    areas: &'a PlanAreaServiceHandle,
    proximity: &'a ProximityServiceHandle,
    extents: Option<&'a VerticalExtentServiceHandle>,
    rectangles: Option<&'a PlanSpanServiceHandle>,
}

impl<'a> Services<'a> {
    fn of(context: &RuleContext<'a>, config: &Config<'_>) -> Result<Self, Unavailable> {
        let missing = |what: &str| {
            (
                NotEvaluatedReason::MissingService,
                format!("{what} service is not registered"),
            )
        };
        Ok(Self {
            areas: context
                .services
                .get::<PlanAreaServiceHandle>()
                .ok_or_else(|| missing("plan-area"))?,
            proximity: context
                .services
                .get::<ProximityServiceHandle>()
                .ok_or_else(|| missing("proximity"))?,
            extents: match config.vertical {
                Some(_) if !config.elevation => Some(
                    context
                        .services
                        .get::<VerticalExtentServiceHandle>()
                        .ok_or_else(|| missing("vertical-extent"))?,
                ),
                _ => None,
            },
            rectangles: if config.axis.is_some() || config.elevation {
                Some(rectangle_service(context)?)
            } else {
                None
            },
        })
    }
}

/// The counterparts near each subject, from the plan broad phase.
struct Counterparts {
    /// Counterparts the selector picks.
    matched: BTreeSet<ObjectId>,
    /// Counterparts near each subject, matched or undecided.
    near: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Selected or undecided counterparts whose extent cannot be read: they
    /// may stand near any subject.
    blind: BTreeSet<ObjectId>,
    /// Subjects whose extent cannot be read.
    unbounded: BTreeMap<ObjectId, Unavailable>,
}

impl Counterparts {
    fn find(
        context: &RuleContext<'_>,
        selector: &Selector,
        margin: f64,
        services: &Services<'_>,
        subjects: &[&Object],
    ) -> Result<Self, Unavailable> {
        let (matched, selection) = select_objects(context, selector);
        let matched: BTreeSet<ObjectId> = matched.iter().map(|object| object.id.clone()).collect();
        let undecided: BTreeSet<ObjectId> = selection
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect();
        let bounds = |object: &ObjectId| match services.proximity.bounds(object) {
            Ok(extent) if extent.object() == object => Ok(extent),
            Ok(_) => Err((
                NotEvaluatedReason::InvalidEvidence,
                "proximity bounds name a different object".to_owned(),
            )),
            Err(error) => Err((proximity_reason(error), error.to_string())),
        };
        let mut unbounded = BTreeMap::new();
        let mut subject_bounds: Vec<ObjectBounds> = Vec::new();
        for subject in subjects {
            match bounds(&subject.id) {
                Ok(extent) => subject_bounds.push(extent),
                Err((reason, message)) => {
                    unbounded.insert(
                        subject.id.clone(),
                        (reason, format!("{message}; its coverage was not checked")),
                    );
                }
            }
        }
        let mut blind = BTreeSet::new();
        let mut counterpart_bounds: Vec<ObjectBounds> = Vec::new();
        for counterpart in matched.iter().chain(&undecided) {
            match bounds(counterpart) {
                Ok(extent) => counterpart_bounds.push(extent),
                Err(_) => {
                    blind.insert(counterpart.clone());
                }
            }
        }
        let pairs = projected_candidate_pairs(
            &subject_bounds,
            &counterpart_bounds,
            ProximityProjection::Horizontal,
            margin,
        )
        .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))?;
        let mut near: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
        for pair in pairs {
            if pair.subject() != pair.counterpart() {
                near.entry(pair.subject().clone())
                    .or_default()
                    .push(pair.counterpart().clone());
            }
        }
        Ok(Self {
            matched,
            near,
            blind,
            unbounded,
        })
    }
}

/// Whether a counterpart's grown footprint covers part of the subject's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlap {
    Sure,
    Maybe,
}

/// A finding graded into a band, with how far it misses the lowest.
struct Graded {
    severity: Severity,
    message: String,
    evidence: Vec<Evidence>,
    related: Vec<ObjectId>,
    deviation: Deviation,
}

/// The finding of one check, or `None` when it passes.
type Check = Result<Option<Graded>, Unavailable>;

/// Counterparts that surely cover part of the subject, and those that may.
struct Cover {
    /// Selected and surely overlapping: the least the cover can be.
    least: Vec<ObjectId>,
    /// Selected or undecided, surely or possibly overlapping: the most.
    most: Vec<ObjectId>,
    /// Why the cover may be larger still than `most`.
    unknown: Vec<String>,
    /// Why counterparts in `most` only may be axis-compatible.
    axes: Vec<String>,
    evidence: Vec<Evidence>,
}

struct Subject<'s, 'a> {
    context: &'s RuleContext<'a>,
    config: &'s Config<'s>,
    services: &'s Services<'a>,
    counterparts: &'s Counterparts,
    frame: Option<&'s Counterparts>,
    object: &'s Object,
}

impl Subject<'_, '_> {
    fn checks(&self) -> Vec<Check> {
        if self.config.elevation {
            return vec![self.elevation()];
        }
        let area = match footprint(self.context, &self.object.id) {
            Ok(area) => area,
            Err(error) => return vec![Err(error)],
        };
        let cover = self.cover(&area);
        let mut checks = Vec::new();
        if let Some(growth) = self.config.horizontal {
            checks.push(self.plan(&area, &cover, growth));
        }
        if let (Some(growth), Some(extents)) = (self.config.vertical, self.services.extents) {
            checks.push(self.height(extents, &cover, growth));
        }
        checks
    }

    /// Sorts the near counterparts by whether their grown footprint covers
    /// part of the subject's: surely when the subject's uncovered area is
    /// surely smaller than its footprint, not at all when it surely is not.
    fn cover(&self, area: &PlanArea) -> Cover {
        let growth = self.config.horizontal.unwrap_or(0.0);
        let mut cover = Cover {
            least: Vec::new(),
            most: Vec::new(),
            unknown: Vec::new(),
            axes: Vec::new(),
            evidence: vec![area.evidence().clone()],
        };
        let own = self.services.rectangles.map(|service| {
            rectangle(service, &self.object.id).map_err(|(_, message)| {
                format!("the axes of {} are unknown: {message}", self.object.id)
            })
        });
        let blind = self.counterparts.blind.len();
        if blind > 0 {
            cover.unknown.push(format!(
                "{blind} counterpart(s) have no readable extent, so they may cover it"
            ));
        }
        let near = self
            .counterparts
            .near
            .get(&self.object.id)
            .map_or(&[][..], Vec::as_slice);
        for counterpart in near {
            let uncovered = match self.services.areas.measure_uncovered_area(
                &self.object.id,
                std::slice::from_ref(counterpart),
                growth,
            ) {
                Ok(uncovered) => uncovered,
                Err(error) => {
                    cover.unknown.push(format!(
                        "whether {counterpart} covers it is unknown: {}",
                        unavailable(error).1
                    ));
                    continue;
                }
            };
            let overlap = if uncovered.upper_square_metres() < area.lower_square_metres() {
                Overlap::Sure
            } else if uncovered.lower_square_metres() >= area.upper_square_metres() {
                continue;
            } else {
                Overlap::Maybe
            };
            let overlap = match (&own, self.config.axis) {
                (Some(own), Some(tolerance)) => {
                    match self.compatible(own, counterpart, tolerance, &mut cover) {
                        Tri::No => continue,
                        Tri::Yes => overlap,
                        Tri::Maybe => Overlap::Maybe,
                    }
                }
                _ => overlap,
            };
            cover.evidence.push(uncovered.evidence().clone());
            let selected = self.counterparts.matched.contains(counterpart);
            if selected && overlap == Overlap::Sure {
                cover.least.push(counterpart.clone());
            }
            cover.most.push(counterpart.clone());
        }
        cover
    }

    /// Whether `counterpart`'s long axis lies within `tolerance` degrees of
    /// parallel to the subject's.
    fn compatible(
        &self,
        own: &Result<PlanRectangle, String>,
        counterpart: &ObjectId,
        tolerance: f64,
        cover: &mut Cover,
    ) -> Tri {
        let Some(service) = self.services.rectangles else {
            return Tri::Maybe;
        };
        let theirs = rectangle(service, counterpart)
            .map_err(|(_, message)| format!("the axes of {counterpart} are unknown: {message}"));
        let (answer, why) = match (own, &theirs) {
            (Ok(own), Ok(theirs)) => {
                cover
                    .evidence
                    .extend([own.evidence().clone(), theirs.evidence().clone()]);
                aligned(own, theirs, Alignment::Parallel, tolerance)
            }
            (Err(why), _) | (_, Err(why)) => (Tri::Maybe, Some(why.clone())),
        };
        if let Some(why) = why {
            cover.axes.push(why);
        }
        answer
    }

    fn plan(&self, area: &PlanArea, cover: &Cover, growth: f64) -> Check {
        let measure = |objects: &[ObjectId]| {
            self.services
                .areas
                .measure_uncovered_area(&self.object.id, objects, growth)
                .map_err(unavailable)
        };
        let least = measure(&cover.least)?;
        let mut evidence = cover.evidence.clone();
        evidence.push(least.evidence().clone());
        let upper = least.upper_square_metres();
        let mut lower = least.lower_square_metres();
        if cover.most != cover.least {
            let most = measure(&cover.most)?;
            evidence.push(most.evidence().clone());
            lower = most.lower_square_metres();
        }
        if !cover.unknown.is_empty() {
            lower = 0.0;
        }
        let share = ratio(
            (lower, upper),
            (area.lower_square_metres(), area.upper_square_metres()),
        );
        let what = format!(
            "plan: {} of the footprint ({} of {} m²) lies outside every counterpart grown by \
             {growth} m",
            shown(share.0, share.1),
            shown(lower, upper),
            shown(area.lower_square_metres(), area.upper_square_metres()),
        );
        self.grade(share, what, evidence, cover)
    }

    fn height(&self, extents: &VerticalExtentServiceHandle, cover: &Cover, growth: f64) -> Check {
        let measure = |object: &ObjectId| {
            extents
                .measure_vertical_extent(object)
                .map_err(|error| extent_unavailable(&error))
        };
        let own = measure(&self.object.id)?;
        let mut evidence = cover.evidence.clone();
        evidence.push(own.evidence().clone());
        let mut measured: BTreeMap<&ObjectId, VerticalExtent> = BTreeMap::new();
        for counterpart in &cover.most {
            let extent = measure(counterpart)?;
            evidence.push(extent.evidence().clone());
            measured.insert(counterpart, extent);
        }
        let of = |objects: &[ObjectId]| -> Vec<&VerticalExtent> {
            objects.iter().filter_map(|id| measured.get(id)).collect()
        };
        let invalid_growth = |error: VerticalExtentError| extent_unavailable(&error);
        let (_, upper) = own
            .uncovered_height(&of(&cover.least), growth)
            .map_err(invalid_growth)?;
        let (mut lower, _) = own
            .uncovered_height(&of(&cover.most), growth)
            .map_err(invalid_growth)?;
        if !cover.unknown.is_empty() {
            lower = 0.0;
        }
        let height = own.height_metres();
        if height.1 <= 0.0 {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("{} has no height", self.object.id),
            ));
        }
        let share = ratio((lower, upper), height);
        let what = format!(
            "height: {} of the height ({} of {} m) lies outside every counterpart overlapping it \
             in plan, grown by {growth} m",
            shown(share.0, share.1),
            shown(lower, upper),
            shown(height.0, height.1),
        );
        self.grade(share, what, evidence, cover)
    }

    /// Grades an uncovered share against the declared bands.
    fn grade(
        &self,
        (lower, upper): (f64, f64),
        what: String,
        evidence: Vec<Evidence>,
        cover: &Cover,
    ) -> Check {
        let bands = &self.config.bands;
        let lowest = bands[0].0;
        let reached = |share: f64| {
            bands
                .iter()
                .rev()
                .find(|(threshold, _)| share > *threshold)
                .map(|(_, severity)| severity.clone())
        };
        if upper <= lowest {
            return Ok(None);
        }
        let Some(surely) = reached(lower) else {
            let mut message = format!("{what}, which straddles the threshold {lowest}");
            for unknown in cover.unknown.iter().chain(&cover.axes) {
                message.push_str("; ");
                message.push_str(unknown);
            }
            return Err((NotEvaluatedReason::IncompleteEvidence, message));
        };
        let severity = reached(upper).unwrap_or_else(|| surely.clone());
        let mut message = what;
        if cover.most.is_empty() && cover.unknown.is_empty() {
            message.push_str("; no counterpart overlaps it");
        }
        if severity != surely {
            let _ = write!(
                message,
                "; graded {} by its upper bound, at least {}",
                label(&severity),
                label(&surely)
            );
        }
        Ok(Some(Graded {
            severity,
            message,
            evidence,
            related: cover.least.clone(),
            deviation: Deviation::above(lowest, lower, upper),
        }))
    }

    /// The near counterparts of `found` as a cover of the elevation: every
    /// selected one surely, undecided ones and those only possibly
    /// axis-compatible possibly. Axes are compared only when `own` holds
    /// the element's rectangle.
    fn elevation_cover(
        &self,
        found: &Counterparts,
        own: Option<&Result<PlanRectangle, String>>,
    ) -> Cover {
        let mut cover = Cover {
            least: Vec::new(),
            most: Vec::new(),
            unknown: Vec::new(),
            axes: Vec::new(),
            evidence: Vec::new(),
        };
        let blind = found.blind.len();
        if blind > 0 {
            cover.unknown.push(format!(
                "{blind} counterpart(s) have no readable extent, so they may cover it"
            ));
        }
        let near = found
            .near
            .get(&self.object.id)
            .map_or(&[][..], Vec::as_slice);
        for counterpart in near {
            let sure = match (own, self.config.axis) {
                (Some(own), Some(tolerance)) => {
                    match self.compatible(own, counterpart, tolerance, &mut cover) {
                        Tri::No => continue,
                        Tri::Yes => true,
                        Tri::Maybe => false,
                    }
                }
                _ => true,
            };
            if sure && found.matched.contains(counterpart) {
                cover.least.push(counterpart.clone());
            }
            cover.most.push(counterpart.clone());
        }
        cover
    }

    /// The uncovered share of the element's elevation.
    fn elevation(&self) -> Check {
        let (Some(rectangles), Some(along), Some(vertical)) = (
            self.services.rectangles,
            self.config.horizontal,
            self.config.vertical,
        ) else {
            unreachable!("an elevation check has its tolerances and axes")
        };
        let own = rectangle(rectangles, &self.object.id)?;
        let axis = own.long_axis().map_err(|why| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "elevation: {} has no long axis to measure its elevation along: {why}",
                    self.object.id
                ),
            )
        })?;
        let axis = own.axes()[axis];
        let mut cover = self.elevation_cover(self.counterparts, Some(&Ok(own.clone())));
        cover.evidence.push(own.evidence().clone());
        let measure = |objects: &[ObjectId], frame: &[ObjectId]| {
            ElevationRequest::try_new(
                self.object.id.clone(),
                axis,
                objects,
                frame,
                along,
                vertical,
            )
            .and_then(|request| self.services.areas.measure_elevation_cover(&request))
            .map_err(unavailable)
        };
        // The area left by the least cover bounds it from above, the area
        // left by the most from below, and something unread may cover all.
        let share = |least: &ElevationCover, most: &ElevationCover, unknown: bool| {
            let upper = least.uncovered_square_metres().1;
            let lower = if unknown {
                0.0
            } else {
                most.uncovered_square_metres().0
            };
            (
                (lower, upper),
                ratio((lower, upper), least.area_square_metres()),
            )
        };
        let least = measure(&cover.least, &[])?;
        let most = if cover.most == cover.least {
            least.clone()
        } else {
            measure(&cover.most, &[])?
        };
        cover
            .evidence
            .extend([least.evidence().clone(), most.evidence().clone()]);
        let area = least.area_square_metres();
        let (mut uncovered, mut shares) = share(&least, &most, !cover.unknown.is_empty());
        let mut infill = String::new();
        if let (Some(frame), Some((_, above))) = (self.frame, self.config.infill) {
            let members = self.elevation_cover(frame, None);
            let applies = Tri::of(shares.0 > above, shares.1 <= above);
            if applies != Tri::No && !(members.most.is_empty() && members.unknown.is_empty()) {
                let least_framed = measure(&cover.least, &members.least)?;
                let most_framed = measure(&cover.most, &members.most)?;
                cover.evidence.extend([
                    least_framed.evidence().clone(),
                    most_framed.evidence().clone(),
                ]);
                let unknown = !cover.unknown.is_empty() || !members.unknown.is_empty();
                let (framed, framed_shares) = share(&least_framed, &most_framed, unknown);
                cover.unknown.extend(members.unknown.iter().cloned());
                if applies == Tri::Yes {
                    (uncovered, shares) = (framed, framed_shares);
                    cover.least.extend(members.least.iter().cloned());
                    infill = format!(" or the infill of the frame of {}", named(&members.most));
                } else {
                    uncovered.0 = framed.0;
                    shares.0 = framed_shares.0;
                    infill = format!(
                        " or, should more than {above} be uncovered (undecided), the infill of \
                         the frame of {}",
                        named(&members.most)
                    );
                }
            }
        }
        let what = format!(
            "elevation: {} of the elevation ({} of {} m²) lies outside every counterpart{infill}, \
             grown by {along} m along its axis and {vertical} m in height",
            shown(shares.0, shares.1),
            shown(uncovered.0, uncovered.1),
            shown(area.0, area.1),
        );
        let evidence = cover.evidence.clone();
        self.grade(shares, what, evidence, &cover)
    }
}

/// Object identities for a message, comma-separated.
fn named(objects: &[ObjectId]) -> String {
    objects
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// `part / whole` over intervals, within `[0, 1]`.
fn ratio(part: (f64, f64), whole: (f64, f64)) -> (f64, f64) {
    let lower = if whole.1 > 0.0 { part.0 / whole.1 } else { 0.0 };
    let upper = if whole.0 > 0.0 { part.1 / whole.0 } else { 1.0 };
    (lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0))
}

fn extent_unavailable(error: &VerticalExtentError) -> Unavailable {
    let reason = match error {
        VerticalExtentError::UnknownObject(_) | VerticalExtentError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        VerticalExtentError::InvalidMeasurement | VerticalExtentError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}
