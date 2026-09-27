//! Effective coverage of an element's footprint by the effect areas of
//! sources: how much of a room its sprinklers, detectors or extinguishers
//! reach.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CoverageEvidence, CoverageRequest, EffectMeets,
    EffectReach, NotEvaluatedReason, ObjectBounds, ParameterDescriptor, ParameterType, Participant,
    PlanAreaServiceHandle, ProximityProjection, ProximityRequest, ProximityServiceHandle,
    RuleCapability, RuleContext, projected_candidate_pairs,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::pairs::{reason as proximity_reason, refuse_all};
use crate::plan_area::{Verdict, judge, shown, unavailable};
use crate::selection::select_objects;
use crate::support::{Parameters, PropertyRef, Unavailable, display, finding, invalid, resolve};

const NAME: &str = "effective-coverage";

/// Requires the union of sources' effect areas to cover enough of each
/// selected element's footprint.
///
/// Every object `sources` picks has an effect area in plan, reaching
/// `range` as `mode` says:
///
/// - `grown`: its footprint grown by `range` in every plan direction;
/// - `touching`: the same, counting only sources whose footprint touches
///   the element's, within `touch_tolerance` (0 when not declared);
/// - `travel`: the points of the element's free region within `range` of
///   travel from the centre of the source's footprint, going round
///   obstacles;
/// - `visible`: the points of the free region the source's centre sees,
///   no farther than `range`.
///
/// The free region is the element's footprint less the footprints of the
/// objects `blockers` picks (travel and sight only); a source whose centre
/// lies outside it reaches none of it. The union of the effect areas,
/// clipped to the footprint and divided by the footprint's area, must reach
/// `minimum_ratio`.
///
/// With `capacity_property` and `capacity_multiplier`, a second check
/// compares the summed property of the sources whose effect meets the
/// footprint, times the multiplier, with the footprint's area: extinguisher
/// rating units times the floor area one unit serves must reach the room's
/// area. A value is read as a number, or a quantity in its SI unit.
///
/// Areas are intervals: the effect areas are bracketed between inner and
/// outer bounds. A source whose selection or touch is undecided counts only
/// towards the upper bound, a blocker whose selection is undecided only
/// narrows the lower bound, and a source whose effect or extent cannot be
/// measured leaves the upper bound at the whole footprint. A ratio
/// straddling the minimum is not evaluated.
pub struct EffectiveCoverage;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Grown,
    Touching,
    Travel,
    Visible,
}

impl Mode {
    fn reach(self) -> EffectReach {
        match self {
            Self::Grown | Self::Touching => EffectReach::Grown,
            Self::Travel => EffectReach::Travel,
            Self::Visible => EffectReach::Visible,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Grown => "grown",
            Self::Touching => "touching",
            Self::Travel => "travel",
            Self::Visible => "visible",
        }
    }
}

struct Config<'a> {
    sources: &'a Selector,
    blockers: Option<&'a Selector>,
    mode: Mode,
    range: f64,
    touch: f64,
    minimum: f64,
    capacity: Option<(PropertyRef<'a>, f64)>,
}

impl RuleCapability for EffectiveCoverage {
    fn id(&self) -> &'static str {
        "axioval:capability.effective-coverage"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("sources", ParameterType::Selector),
            ParameterDescriptor::required("mode", ParameterType::String),
            ParameterDescriptor::required("range", ParameterType::Quantity),
            ParameterDescriptor::required("minimum_ratio", ParameterType::Number),
            ParameterDescriptor::optional("blockers", ParameterType::Selector),
            ParameterDescriptor::optional("touch_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("capacity_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("capacity_multiplier", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (elements, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context) {
            Ok(services) => services,
            Err((reason, message)) => {
                return refuse_all(&elements, evaluation, &reason, &message);
            }
        };
        let near = match Near::find(context, &config, &services, &elements) {
            Ok(near) => near,
            Err((reason, message)) => {
                return refuse_all(&elements, evaluation, &reason, &message);
            }
        };
        let mut evaluation = evaluation;
        for element in elements {
            if let Some((reason, message)) = near.unbounded.get(&element.id) {
                evaluation.push_object_not_evaluated(
                    element.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
                continue;
            }
            let checks = Element {
                context,
                config: &config,
                services: &services,
                near: &near,
                object: element,
            }
            .checks();
            for check in checks {
                match check {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => {
                        evaluation.push_finding(finding(
                            rule,
                            &element.id,
                            message,
                            evidence,
                            related,
                        ));
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(element.id.clone(), reason, message);
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
        Some((value, QuantityDimension::Length)) if value.is_finite() && value >= 0.0 => {
            Ok(Some(value))
        }
        Some(_) => Err(invalid(format!("{name} must be a non-negative length"))),
    };
    let mode = match parameters.required_string("mode")? {
        "grown" => Mode::Grown,
        "touching" => Mode::Touching,
        "travel" => Mode::Travel,
        "visible" => Mode::Visible,
        other => {
            return Err(invalid(format!(
                "mode `{other}` is unsupported; use `grown`, `touching`, `travel` or `visible`"
            )));
        }
    };
    let range = length("range")?.ok_or_else(|| invalid("range is required"))?;
    let minimum = match parameters.number("minimum_ratio")? {
        Some(minimum) if minimum > 0.0 && minimum <= 1.0 => minimum,
        _ => return Err(invalid("minimum_ratio must lie in (0, 1]")),
    };
    let blockers = parameters.selector("blockers")?;
    if blockers.is_some() && matches!(mode, Mode::Grown | Mode::Touching) {
        return Err(invalid(
            "blockers apply only to modes `travel` and `visible`",
        ));
    }
    let touch = length("touch_tolerance")?;
    if touch.is_some() && mode != Mode::Touching {
        return Err(invalid("touch_tolerance applies only to mode `touching`"));
    }
    let capacity = match (
        parameters.property("capacity_property")?,
        parameters.number("capacity_multiplier")?,
    ) {
        (None, None) => None,
        (Some(property), Some(multiplier)) if multiplier.is_finite() && multiplier > 0.0 => {
            Some((property, multiplier))
        }
        (Some(_), Some(_)) => {
            return Err(invalid("capacity_multiplier must be a positive number"));
        }
        _ => {
            return Err(invalid(
                "capacity_property and capacity_multiplier are declared together",
            ));
        }
    };
    Ok(Config {
        sources: parameters.required_selector("sources")?,
        blockers,
        mode,
        range,
        touch: touch.unwrap_or(0.0),
        minimum,
        capacity,
    })
}

struct Services<'a> {
    areas: &'a PlanAreaServiceHandle,
    proximity: &'a ProximityServiceHandle,
}

impl<'a> Services<'a> {
    fn of(context: &RuleContext<'a>) -> Result<Self, Unavailable> {
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
        })
    }
}

/// The sources and blockers near each element, from the plan broad phase.
struct Near {
    /// Sources the selector picks.
    sources: BTreeSet<ObjectId>,
    /// Blockers the selector picks.
    blockers: BTreeSet<ObjectId>,
    /// Sources within reach of each element, picked or undecided.
    reaching: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Blockers overlapping each element in plan, picked or undecided.
    blocking: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Sources whose extent cannot be read: they may reach any element.
    blind: BTreeSet<ObjectId>,
    /// Blockers whose extent cannot be read: they may block in any element.
    blind_blockers: BTreeSet<ObjectId>,
    /// Elements whose extent cannot be read.
    unbounded: BTreeMap<ObjectId, Unavailable>,
}

/// The objects a selector picks, and those it cannot decide.
fn picked(
    context: &RuleContext<'_>,
    selector: &Selector,
) -> (BTreeSet<ObjectId>, BTreeSet<ObjectId>) {
    let (matched, selection) = select_objects(context, selector);
    (
        matched.iter().map(|object| object.id.clone()).collect(),
        selection
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect(),
    )
}

impl Near {
    fn find(
        context: &RuleContext<'_>,
        config: &Config<'_>,
        services: &Services<'_>,
        elements: &[&Object],
    ) -> Result<Self, Unavailable> {
        let (sources, undecided_sources) = picked(context, config.sources);
        let (blockers, undecided_blockers) = config
            .blockers
            .map(|selector| picked(context, selector))
            .unwrap_or_default();
        let bounds = |object: &ObjectId| match services.proximity.bounds(object) {
            Ok(extent) if extent.object() == object => Ok(extent),
            Ok(_) => Err((
                NotEvaluatedReason::InvalidEvidence,
                "proximity bounds name a different object".to_owned(),
            )),
            Err(error) => Err((proximity_reason(error), error.to_string())),
        };
        let mut unbounded = BTreeMap::new();
        let mut element_bounds: Vec<ObjectBounds> = Vec::new();
        for element in elements {
            match bounds(&element.id) {
                Ok(extent) => element_bounds.push(extent),
                Err((reason, message)) => {
                    unbounded.insert(
                        element.id.clone(),
                        (reason, format!("{message}; its coverage was not checked")),
                    );
                }
            }
        }
        let gather = |candidates: &mut dyn Iterator<Item = &ObjectId>| {
            let mut found = Vec::new();
            let mut blind = BTreeSet::new();
            for candidate in candidates {
                match bounds(candidate) {
                    Ok(extent) => found.push(extent),
                    Err(_) => {
                        blind.insert(candidate.clone());
                    }
                }
            }
            (found, blind)
        };
        let (source_bounds, blind) = gather(&mut sources.iter().chain(&undecided_sources));
        let (blocker_bounds, blind_blockers) = gather(
            &mut blockers
                .iter()
                .chain(&undecided_blockers)
                .filter(|blocker| {
                    !sources.contains(*blocker) && !undecided_sources.contains(*blocker)
                }),
        );
        let margin = match config.mode {
            Mode::Touching => config.touch,
            _ => config.range,
        };
        let pairs = |counterparts: &[ObjectBounds], margin: f64| {
            projected_candidate_pairs(
                &element_bounds,
                counterparts,
                ProximityProjection::Horizontal,
                margin,
            )
            .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))
            .map(|pairs| {
                let mut near: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
                for pair in pairs {
                    if pair.subject() != pair.counterpart() {
                        near.entry(pair.subject().clone())
                            .or_default()
                            .push(pair.counterpart().clone());
                    }
                }
                near
            })
        };
        Ok(Self {
            reaching: pairs(&source_bounds, margin)?,
            blocking: pairs(&blocker_bounds, 0.0)?,
            sources,
            blockers,
            blind,
            blind_blockers,
            unbounded,
        })
    }
}

/// The finding of one check, or `None` when it passes.
type Check = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

struct Element<'s, 'a> {
    context: &'s RuleContext<'a>,
    config: &'s Config<'s>,
    services: &'s Services<'a>,
    near: &'s Near,
    object: &'s Object,
}

impl Element<'_, '_> {
    fn checks(&self) -> Vec<Check> {
        let (request, mut notes) = match self.request() {
            Ok(request) => request,
            Err(error) => return vec![Err(error)],
        };
        let measured = match self.services.areas.measure_coverage(&request) {
            Ok(measured) => measured,
            Err(error) => return vec![Err(unavailable(error))],
        };
        for (source, meets) in measured.effects() {
            if let EffectMeets::Unmeasured(reason) = meets {
                notes.push(format!("the effect of {source} is unmeasured: {reason}"));
            }
        }
        let mut checks = vec![self.coverage(&request, &measured, &notes)];
        if let Some((property, multiplier)) = self.config.capacity {
            checks.push(self.capacity(&request, &measured, property, multiplier));
        }
        checks
    }

    /// The coverage request, and why the covered area may be larger than
    /// measured.
    fn request(&self) -> Result<(CoverageRequest, Vec<String>), Unavailable> {
        let own = &self.object.id;
        let mut notes = Vec::new();
        let blind = self.near.blind.len();
        if blind > 0 {
            notes.push(format!(
                "{blind} source(s) have no readable extent, so they may cover it"
            ));
        }
        let near = |map: &BTreeMap<ObjectId, Vec<ObjectId>>| {
            map.get(own).map_or(&[][..], Vec::as_slice).to_vec()
        };
        let mut sources = Vec::new();
        for source in near(&self.near.reaching) {
            let selected = self.near.sources.contains(&source);
            let touching = if self.config.mode == Mode::Touching {
                match self.touches(&source) {
                    Some(true) => true,
                    Some(false) => continue,
                    None => false,
                }
            } else {
                true
            };
            sources.push(Participant::new(source, selected && touching));
        }
        let blockers = near(&self.near.blocking)
            .into_iter()
            .map(|blocker| {
                let certain = self.near.blockers.contains(&blocker);
                Participant::new(blocker, certain)
            })
            .collect();
        let request = CoverageRequest::try_new(
            own.clone(),
            self.config.mode.reach(),
            self.config.range,
            sources,
            blockers,
        )
        .map_err(unavailable)?;
        Ok((request, notes))
    }

    /// Whether a source's footprint touches the element's: `None` when the
    /// measured distance straddles the tolerance or cannot be measured.
    fn touches(&self, source: &ObjectId) -> Option<bool> {
        let request = ProximityRequest::projected(
            self.object.id.clone(),
            source.clone(),
            ProximityProjection::Horizontal,
        )
        .ok()?;
        let (lower, upper) = self
            .services
            .proximity
            .measure_distance(&request)
            .ok()?
            .interval_metres();
        if upper <= self.config.touch {
            Some(true)
        } else if lower > self.config.touch {
            Some(false)
        } else {
            None
        }
    }

    fn coverage(
        &self,
        request: &CoverageRequest,
        measured: &CoverageEvidence,
        notes: &[String],
    ) -> Check {
        let footprint = measured.footprint();
        let covered = measured.covered();
        let whole = (
            footprint.lower_square_metres(),
            footprint.upper_square_metres(),
        );
        let (lower, mut upper) = (covered.lower_square_metres(), covered.upper_square_metres());
        if !self.near.blind.is_empty() {
            upper = whole.1;
        }
        let lower = if self.near.blind_blockers.is_empty() {
            lower
        } else {
            0.0
        };
        let share = ratio((lower, upper), whole);
        let evidence: Vec<Evidence> = measured.evidence().into_iter().cloned().collect();
        let what = format!(
            "{} of the footprint ({} of {} m²) lies within the sources' effect areas ({} by {} m)",
            shown(share.0, share.1),
            shown(lower, upper),
            shown(whole.0, whole.1),
            self.config.mode.name(),
            self.config.range,
        );
        let mut unknown = notes.to_vec();
        if !self.near.blind_blockers.is_empty() {
            unknown.push(format!(
                "{} blocker(s) have no readable extent, so they may block it",
                self.near.blind_blockers.len()
            ));
        }
        match judge(share.0, share.1, Some(self.config.minimum), None) {
            Verdict::Pass => Ok(None),
            Verdict::Fail(bound) => {
                let mut message = format!("{what}; required {bound}");
                if request.sources().is_empty() {
                    message.push_str("; no source reaches it");
                }
                Ok(Some((
                    message,
                    evidence,
                    contributing(request, measured, true),
                )))
            }
            Verdict::Undecided(bound) => {
                let mut message = format!("{what}, which straddles the bound {bound}");
                for note in unknown.iter().take(3) {
                    let _ = write!(message, "; {note}");
                }
                Err((NotEvaluatedReason::IncompleteEvidence, message))
            }
        }
    }

    /// The summed capacity of the sources whose effect meets the footprint,
    /// times the multiplier, against the footprint's area.
    fn capacity(
        &self,
        request: &CoverageRequest,
        measured: &CoverageEvidence,
        property: PropertyRef<'_>,
        multiplier: f64,
    ) -> Check {
        let mut lower = 0.0;
        let mut upper = if self.near.blind.is_empty() {
            0.0
        } else {
            f64::INFINITY
        };
        let mut evidence: Vec<Evidence> = vec![measured.footprint().evidence().clone()];
        let mut unknown: Vec<String> = Vec::new();
        let sure = contributing(request, measured, true);
        for source in contributing(request, measured, false) {
            let certain = sure.contains(&source);
            match self.value(&source, property) {
                Ok((value, found)) => {
                    evidence.extend(found);
                    upper += value;
                    if certain {
                        lower += value;
                    }
                }
                Err(why) => {
                    upper = f64::INFINITY;
                    unknown.push(why);
                }
            }
        }
        let footprint = measured.footprint();
        let (need_low, need_high) = (
            footprint.lower_square_metres(),
            footprint.upper_square_metres(),
        );
        let (supplied_low, supplied_high) = (lower * multiplier, upper * multiplier);
        let what = format!(
            "capacity: {property} summed over the sources reaching it, times {multiplier}, is {} \
             m² for a footprint of {} m²",
            shown(supplied_low, supplied_high),
            shown(need_low, need_high),
        );
        if supplied_low >= need_high {
            Ok(None)
        } else if supplied_high < need_low {
            Ok(Some((what, evidence, sure)))
        } else {
            let mut message = format!("{what}, which cannot be decided");
            for note in unknown.iter().take(3) {
                let _ = write!(message, "; {note}");
            }
            Err((NotEvaluatedReason::IncompleteEvidence, message))
        }
    }

    /// A source's capacity as a non-negative number, with its evidence.
    fn value(
        &self,
        source: &ObjectId,
        property: PropertyRef<'_>,
    ) -> Result<(f64, Vec<Evidence>), String> {
        let object = self
            .context
            .project
            .object(source)
            .ok_or_else(|| format!("{source} is not in the project"))?;
        let resolved = resolve(self.context, object, property)
            .map_err(|(_, message)| format!("{property} of {source}: {message}"))?;
        let value = match resolved.value() {
            Some(PropertyValue::Integer(value)) => crate::support::exact_f64(*value),
            Some(PropertyValue::Decimal(value) | PropertyValue::Quantity { value, .. }) => {
                Some(*value)
            }
            _ => None,
        }
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| {
            format!(
                "{source} states no non-negative {property} ({})",
                display(resolved.value())
            )
        })?;
        Ok((value, resolved.evidence()))
    }
}

/// The sources whose effect meets the footprint: surely and certainly, or
/// possibly (surely, possibly or unmeasured, certain or not).
fn contributing(
    request: &CoverageRequest,
    measured: &CoverageEvidence,
    sure: bool,
) -> Vec<ObjectId> {
    request
        .sources()
        .iter()
        .zip(measured.effects())
        .filter(|(participant, (_, meets))| {
            if sure {
                participant.is_certain() && *meets == EffectMeets::Surely
            } else {
                *meets != EffectMeets::No
            }
        })
        .map(|(participant, _)| participant.object().clone())
        .collect()
}

/// `part / whole` over intervals, within `[0, 1]`.
fn ratio(part: (f64, f64), whole: (f64, f64)) -> (f64, f64) {
    let lower = if whole.1 > 0.0 { part.0 / whole.1 } else { 0.0 };
    let upper = if whole.0 > 0.0 { part.1 / whole.0 } else { 1.0 };
    (lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0))
}
