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
use crate::space_access::{AccessDeclaration, AccessIndex};
use crate::support::{
    Parameters, PropertyRef, Resolved, Unavailable, display, finding, invalid, resolve, undefined,
};

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
/// lies outside it reaches none of it. With `access_path` (and
/// `door_selector`, `opening_selector`, `space_selector`, read as
/// `space-connection` reads them), travel and sight continue into the
/// spaces the element's doors and openings join it to: the free region
/// also holds their footprints and the doors' and openings', so a
/// sprinkler in the next room covers the element through an open doorway.
/// The union of the effect areas, clipped to the footprint and divided by
/// the footprint's area (or the area `area_property` states), must reach
/// `minimum_ratio`.
///
/// With `capacity_property` and `capacity_multiplier` (a constant) or
/// `capacity_multiplier_property` (read on each source), a second check
/// compares the summed products of the sources whose effect meets the
/// footprint with the element's area: extinguisher rating units times the
/// floor area one unit serves must reach the room's area. A value is read
/// as a number, or a quantity in its SI unit.
///
/// Areas are intervals: the effect areas are bracketed between inner and
/// outer bounds. A source whose selection or touch is undecided counts only
/// towards the upper bound, a blocker whose selection is undecided only
/// narrows the lower bound, and a source whose effect or extent cannot be
/// measured leaves the upper bound at the whole footprint; so does a door
/// or opening whose spaces cannot be read. A ratio straddling the minimum
/// is not evaluated.
///
/// A value that is not stated (absent, null or blank) is a finding of its
/// own, starting `missing value:`: the element's `area_property`, then
/// checked no further, or the capacity or multiplier of a source that
/// surely contributes. A value of another kind is not evaluated.
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

/// What each source's capacity is multiplied by.
#[derive(Clone, Copy)]
enum Multiplier<'a> {
    Constant(f64),
    Property(PropertyRef<'a>),
}

#[derive(Clone, Copy)]
struct Capacity<'a> {
    property: PropertyRef<'a>,
    multiplier: Multiplier<'a>,
}

struct Config<'a> {
    sources: &'a Selector,
    blockers: Option<&'a Selector>,
    mode: Mode,
    range: f64,
    touch: f64,
    minimum: f64,
    capacity: Option<Capacity<'a>>,
    area: Option<PropertyRef<'a>>,
    access: Option<AccessDeclaration<'a>>,
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
            ParameterDescriptor::optional(
                "capacity_multiplier_property",
                ParameterType::PropertyReference,
            ),
            ParameterDescriptor::optional("area_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("access_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
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
        let index = config.access.as_ref().map(|access| access.index(context));
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
                index: index.as_ref(),
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
    let access = AccessDeclaration::parse(parameters)?;
    if access.is_some() && matches!(mode, Mode::Grown | Mode::Touching) {
        return Err(invalid(
            "access_path applies only to modes `travel` and `visible`: a grown effect ignores \
             walls already",
        ));
    }
    Ok(Config {
        sources: parameters.required_selector("sources")?,
        blockers,
        mode,
        range,
        touch: touch.unwrap_or(0.0),
        minimum,
        capacity: capacity(parameters)?,
        area: parameters.property("area_property")?,
        access,
    })
}

fn capacity<'a>(parameters: &Parameters<'a>) -> Result<Option<Capacity<'a>>, Unavailable> {
    let property = parameters.property("capacity_property")?;
    let constant = parameters.number("capacity_multiplier")?;
    let per_source = parameters.property("capacity_multiplier_property")?;
    let multiplier = match (constant, per_source) {
        (None, None) => None,
        (Some(multiplier), None) if multiplier.is_finite() && multiplier > 0.0 => {
            Some(Multiplier::Constant(multiplier))
        }
        (Some(_), None) => return Err(invalid("capacity_multiplier must be a positive number")),
        (None, Some(property)) => Some(Multiplier::Property(property)),
        (Some(_), Some(_)) => {
            return Err(invalid(
                "declare capacity_multiplier or capacity_multiplier_property, not both",
            ));
        }
    };
    match (property, multiplier) {
        (None, None) => Ok(None),
        (Some(property), Some(multiplier)) => Ok(Some(Capacity {
            property,
            multiplier,
        })),
        _ => Err(invalid(
            "capacity_property is declared together with capacity_multiplier or \
             capacity_multiplier_property",
        )),
    }
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
    /// Blockers that may matter to each element, picked or undecided: those
    /// overlapping it in plan, or with connections those within range.
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
        // A walk or a sight line reaching the element within the range
        // stays within the range of it, so only blockers there can cut it.
        let blocker_margin = if config.access.is_some() {
            config.range
        } else {
            0.0
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
            blocking: pairs(&blocker_bounds, blocker_margin)?,
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

/// A value read from an object.
enum Read {
    Value(f64, Vec<Evidence>),
    /// Not stated: absent, null or blank.
    Missing(String, Vec<Evidence>),
    Unknown(String),
}

/// The area the share and the capacity are measured against.
struct Area {
    lower: f64,
    upper: f64,
    /// How messages name it.
    named: String,
    evidence: Vec<Evidence>,
}

/// The coverage request, the connections' evidence, and why the covered
/// area may be larger than measured.
struct Asked {
    request: CoverageRequest,
    notes: Vec<String>,
    /// Whether the upper bound must stay at the whole footprint.
    open: bool,
    evidence: Vec<Evidence>,
}

struct Element<'s, 'a> {
    context: &'s RuleContext<'a>,
    config: &'s Config<'s>,
    services: &'s Services<'a>,
    near: &'s Near,
    index: Option<&'s AccessIndex>,
    object: &'s Object,
}

impl Element<'_, '_> {
    fn checks(&self) -> Vec<Check> {
        let stated = match self.config.area {
            None => None,
            Some(property) => match self.read(&self.object.id, property, true) {
                Read::Value(value, evidence) => Some((property, value, evidence)),
                Read::Missing(message, evidence) => {
                    return vec![Ok(Some((message, evidence, Vec::new())))];
                }
                Read::Unknown(why) => {
                    return vec![Err((NotEvaluatedReason::IncompleteEvidence, why))];
                }
            },
        };
        let asked = match self.request() {
            Ok(asked) => asked,
            Err(error) => return vec![Err(error)],
        };
        let measured = match self.services.areas.measure_coverage(&asked.request) {
            Ok(measured) => measured,
            Err(error) => return vec![Err(unavailable(error))],
        };
        let mut asked = asked;
        for (source, meets) in measured.effects() {
            if let EffectMeets::Unmeasured(reason) = meets {
                asked
                    .notes
                    .push(format!("the effect of {source} is unmeasured: {reason}"));
            }
        }
        let area = if let Some((property, value, evidence)) = stated {
            Area {
                lower: value,
                upper: value,
                named: format!("the stated area ({property})"),
                evidence,
            }
        } else {
            let footprint = measured.footprint();
            Area {
                lower: footprint.lower_square_metres(),
                upper: footprint.upper_square_metres(),
                named: "the footprint".to_owned(),
                evidence: vec![footprint.evidence().clone()],
            }
        };
        let mut checks = vec![self.coverage(&asked, &measured, &area)];
        if let Some(capacity) = self.config.capacity {
            checks.extend(self.capacity(&asked, &measured, &area, capacity));
        }
        checks
    }

    /// The coverage request, with the element's connections.
    fn request(&self) -> Result<Asked, Unavailable> {
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
        let mut request = CoverageRequest::try_new(
            own.clone(),
            self.config.mode.reach(),
            self.config.range,
            sources,
            blockers,
        )
        .map_err(unavailable)?;
        let mut open = !self.near.blind.is_empty();
        let mut evidence = Vec::new();
        if let Some(index) = self.index {
            let (joined, unknown) = index.connections(own);
            if !unknown.is_empty() {
                open = true;
                notes.extend(
                    unknown
                        .into_iter()
                        .map(|why| format!("a door or opening may join more spaces: {why}")),
                );
            }
            let (mut spaces, mut passages) = (Vec::new(), Vec::new());
            for connection in joined {
                if connection.certain {
                    evidence.extend(connection.evidence);
                }
                spaces.push(Participant::new(connection.space, connection.certain));
                passages.push(Participant::new(connection.via, connection.certain));
            }
            request = request
                .with_connections(spaces, passages)
                .map_err(unavailable)?;
        }
        Ok(Asked {
            request,
            notes,
            open,
            evidence,
        })
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

    fn coverage(&self, asked: &Asked, measured: &CoverageEvidence, area: &Area) -> Check {
        let footprint = measured.footprint();
        let covered = measured.covered();
        let (lower, mut upper) = (covered.lower_square_metres(), covered.upper_square_metres());
        if asked.open {
            upper = footprint.upper_square_metres();
        }
        let lower = if self.near.blind_blockers.is_empty() {
            lower
        } else {
            0.0
        };
        let share = ratio((lower, upper), (area.lower, area.upper));
        let mut evidence: Vec<Evidence> = measured.evidence().into_iter().cloned().collect();
        if self.config.area.is_some() {
            evidence.extend(area.evidence.iter().cloned());
        }
        evidence.extend(asked.evidence.iter().cloned());
        let what = format!(
            "{} of {} ({} of {} m²) lies within the sources' effect areas ({} by {} m)",
            shown(share.0, share.1),
            area.named,
            shown(lower, upper),
            shown(area.lower, area.upper),
            self.config.mode.name(),
            self.config.range,
        );
        let mut unknown = asked.notes.clone();
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
                if asked.request.sources().is_empty() {
                    message.push_str("; no source reaches it");
                }
                Ok(Some((
                    message,
                    evidence,
                    contributing(&asked.request, measured, true),
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
    /// each times its multiplier, against the element's area; and a
    /// missing-value finding per surely contributing source that states no
    /// capacity or multiplier.
    fn capacity(
        &self,
        asked: &Asked,
        measured: &CoverageEvidence,
        area: &Area,
        capacity: Capacity<'_>,
    ) -> Vec<Check> {
        let mut checks = Vec::new();
        let mut lower = 0.0;
        let mut upper = if asked.open { f64::INFINITY } else { 0.0 };
        let mut evidence: Vec<Evidence> = area.evidence.clone();
        let mut unknown: Vec<String> = Vec::new();
        let sure = contributing(&asked.request, measured, true);
        for source in contributing(&asked.request, measured, false) {
            let certain = sure.contains(&source);
            let factor = match capacity.multiplier {
                Multiplier::Constant(multiplier) => Read::Value(multiplier, Vec::new()),
                Multiplier::Property(property) => self.read(&source, property, false),
            };
            match (self.read(&source, capacity.property, false), factor) {
                (Read::Value(value, found), Read::Value(factor, cited)) => {
                    evidence.extend(found);
                    evidence.extend(cited);
                    upper += value * factor;
                    if certain {
                        lower += value * factor;
                    }
                }
                (value, factor) => {
                    upper = f64::INFINITY;
                    for read in [value, factor] {
                        match read {
                            Read::Value(..) => {}
                            Read::Missing(message, cited) => {
                                if certain {
                                    checks.push(Ok(Some((
                                        message.clone(),
                                        cited,
                                        vec![source.clone()],
                                    ))));
                                }
                                unknown.push(message);
                            }
                            Read::Unknown(why) => unknown.push(why),
                        }
                    }
                }
            }
        }
        let (need_low, need_high) = (area.lower, area.upper);
        let against = if self.config.area.is_some() {
            area.named.as_str()
        } else {
            "a footprint"
        };
        let summed = match capacity.multiplier {
            Multiplier::Constant(multiplier) => format!(
                "{} summed over the sources reaching it, times {multiplier},",
                capacity.property
            ),
            Multiplier::Property(multiplier) => format!(
                "{} times {multiplier} summed over the sources reaching it",
                capacity.property
            ),
        };
        let what = format!(
            "capacity: {summed} is {} m² for {against} of {} m²",
            shown(lower, upper),
            shown(need_low, need_high),
        );
        let check = if lower >= need_high {
            Ok(None)
        } else if upper < need_low {
            Ok(Some((what, evidence, sure)))
        } else {
            let mut message = format!("{what}, which cannot be decided");
            for note in unknown.iter().take(3) {
                let _ = write!(message, "; {note}");
            }
            Err((NotEvaluatedReason::IncompleteEvidence, message))
        };
        checks.insert(0, check);
        checks
    }

    /// A non-negative number an object states: a number, or a quantity in
    /// its SI unit, which must be an `area`.
    fn read(&self, holder: &ObjectId, property: PropertyRef<'_>, area: bool) -> Read {
        let Some(object) = self.context.project.object(holder) else {
            return Read::Unknown(format!("{holder} is not in the project"));
        };
        let resolved = match resolve(self.context, object, property) {
            Ok(resolved) => resolved,
            Err((_, message)) => {
                return Read::Unknown(format!("{property} of {holder}: {message}"));
            }
        };
        let cited = resolved.evidence();
        let value = match &resolved {
            Resolved::Absent(_) => None,
            Resolved::Present(stated) => Some(&stated.value),
        };
        if undefined(value) {
            let what = if *holder == self.object.id {
                "its".to_owned()
            } else {
                format!("{holder}'s")
            };
            return Read::Missing(
                format!("missing value: {what} {property} is not stated"),
                cited,
            );
        }
        let number = match value {
            Some(PropertyValue::Integer(value)) => crate::support::exact_f64(*value),
            Some(PropertyValue::Decimal(value)) => Some(*value),
            Some(PropertyValue::Quantity { value, dimension })
                if !area || *dimension == QuantityDimension::Area =>
            {
                Some(*value)
            }
            _ => None,
        }
        .filter(|value| value.is_finite() && *value >= 0.0);
        match number {
            Some(number) => Read::Value(number, cited),
            None => Read::Unknown(format!(
                "{holder} states no non-negative {}{property} ({})",
                if area { "area " } else { "" },
                display(value)
            )),
        }
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
