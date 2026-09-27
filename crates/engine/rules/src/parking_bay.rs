//! Parking bays: their size along their own axes, their orientation to the
//! aisle and the obstructions allowed at their ends and sides.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ObjectBounds, ParameterDescriptor,
    ParameterType, PlanRectangle, PlanSpanServiceHandle, ProximityProjection, ProximityRequest,
    ProximityServiceHandle, RuleCapability, RuleContext, VerticalExtentServiceHandle,
    projected_candidate_pairs,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::orientation::{
    Alignment, Tri, aligned, along, angle_tolerance, extent_unavailable, rectangle,
    rectangle_service,
};
use crate::pairs::{reason as proximity_reason, refuse_all};
use crate::plan_area::{Verdict, judge, shown};
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, finding, invalid};

const NAME: &str = "parking-bay";

/// Requires each selected parking bay to have its size along its own axes,
/// its orientation to the aisle, and no more obstructions at its ends and
/// sides than allowed.
///
/// A bay's own axes are those of the least-area rectangle enclosing its
/// footprint; its length is the longer side, its width the shorter, and its
/// height its vertical extent. The ends are the two sides across the long
/// axis, the sides the two along it.
///
/// - **Size**: `min_width`, `max_width`, `min_length`, `max_length`,
///   `min_height`, `max_height`, each an inclusive bound.
/// - **Orientation**: with `orientation` (`parallel`, `perpendicular` or
///   `angled`) the bay's long axis stands so to the long axis of an
///   `aisles` object within `aisle_reach` of it in plan (default 0: meeting
///   it), within `angle_tolerance` of parallel or perpendicular.
/// - **Obstructions**: an `obstacles` object within `obstruction_reach` of
///   the bay in plan obstructs an end when it reaches past the end's line
///   and overlaps the end's span across the bay, likewise a side.
///   `end_obstructions` and `side_obstructions` (`none`, `one` or `both`)
///   say how many ends and sides may be obstructed; an obstacle within
///   the bay's rectangle, past none of its edges, is always a finding.
///
/// Every measure is an interval: a check passes when the whole interval
/// satisfies it, is a finding when none of it does, and is otherwise not
/// evaluated. A bay whose orientation is not unique (several least-area
/// rectangles), whose footprint is tessellated, or whose sides are too
/// close to equal for a long axis is not evaluated where its axes decide.
/// Objects the selectors cannot decide can only add aisles or
/// obstructions: they turn a finding they could remove, or a pass they
/// could break, into not evaluated.
pub struct ParkingBay;

/// How many of a bay's two ends, or two sides, may be obstructed.
#[derive(Clone, Copy)]
struct Allowed(usize);

impl Allowed {
    fn parse(name: &str, value: Option<&str>) -> Result<Option<Self>, Unavailable> {
        match value {
            None => Ok(None),
            Some("none") => Ok(Some(Self(0))),
            Some("one") => Ok(Some(Self(1))),
            Some("both") => Ok(Some(Self(2))),
            Some(other) => Err(invalid(format!(
                "{name} `{other}` is unsupported; use `none`, `one` or `both`"
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self.0 {
            0 => "none",
            1 => "one",
            _ => "both",
        }
    }
}

struct Orientation<'a> {
    aisles: &'a Selector,
    reach: f64,
    alignment: Alignment,
    tolerance: f64,
}

struct Obstructions<'a> {
    obstacles: &'a Selector,
    reach: f64,
    ends: Allowed,
    sides: Allowed,
}

struct Config<'a> {
    width: (Option<f64>, Option<f64>),
    length: (Option<f64>, Option<f64>),
    height: (Option<f64>, Option<f64>),
    orientation: Option<Orientation<'a>>,
    obstructions: Option<Obstructions<'a>>,
}

impl Config<'_> {
    fn sized(&self) -> bool {
        [self.width, self.length]
            .iter()
            .any(|(low, high)| low.is_some() || high.is_some())
    }

    fn high(&self) -> bool {
        self.height.0.is_some() || self.height.1.is_some()
    }
}

impl RuleCapability for ParkingBay {
    fn id(&self) -> &'static str {
        "axioval:capability.parking-bay"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters: Vec<ParameterDescriptor> = [
            "min_width",
            "max_width",
            "min_length",
            "max_length",
            "min_height",
            "max_height",
        ]
        .into_iter()
        .map(|name| ParameterDescriptor::optional(name, ParameterType::Quantity))
        .collect();
        parameters.extend([
            ParameterDescriptor::optional("aisles", ParameterType::Selector),
            ParameterDescriptor::optional("aisle_reach", ParameterType::Quantity),
            ParameterDescriptor::optional("orientation", ParameterType::String),
            ParameterDescriptor::optional("angle_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("obstacles", ParameterType::Selector),
            ParameterDescriptor::optional("obstruction_reach", ParameterType::Quantity),
            ParameterDescriptor::optional("end_obstructions", ParameterType::String),
            ParameterDescriptor::optional("side_obstructions", ParameterType::String),
        ]);
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (bays, evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context, &config) {
            Ok(services) => services,
            Err((reason, message)) => return refuse_all(&bays, evaluation, &reason, &message),
        };
        let near = |selector: &Selector, reach: f64| {
            Nearby::find(context, &services, selector, &bays, reach)
        };
        let aisles = match config.orientation.as_ref().map(|o| near(o.aisles, o.reach)) {
            Some(Err((reason, message))) => {
                return refuse_all(&bays, evaluation, &reason, &message);
            }
            other => other.map(Result::unwrap),
        };
        let obstacles = match config
            .obstructions
            .as_ref()
            .map(|o| near(o.obstacles, o.reach))
        {
            Some(Err((reason, message))) => {
                return refuse_all(&bays, evaluation, &reason, &message);
            }
            other => other.map(Result::unwrap),
        };
        let mut evaluation = evaluation;
        for bay in bays {
            let judged = Bay {
                config: &config,
                services: &services,
                object: bay,
            };
            for check in judged.checks(aisles.as_ref(), obstacles.as_ref()) {
                match check {
                    Ok(None) => {}
                    Ok(Some((message, evidence, related))) => {
                        evaluation.push_finding(finding(rule, &bay.id, message, evidence, related));
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(bay.id.clone(), reason, message);
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
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some((_, QuantityDimension::Length)) => Err(invalid(format!("{name} is negative"))),
        Some(_) => Err(invalid(format!("{name} must be a length"))),
    };
    let range = |name: &str| -> Result<(Option<f64>, Option<f64>), Unavailable> {
        let (low, high) = (
            length(&format!("min_{name}"))?,
            length(&format!("max_{name}"))?,
        );
        if let (Some(low), Some(high)) = (low, high)
            && low > high
        {
            return Err(invalid(format!("min_{name} exceeds max_{name}")));
        }
        Ok((low, high))
    };
    let aisles = parameters.selector("aisles")?;
    let orientation = match (aisles, parameters.string("orientation")?) {
        (Some(aisles), Some(alignment)) => Some(Orientation {
            aisles,
            reach: length("aisle_reach")?.unwrap_or(0.0),
            alignment: Alignment::parse(alignment)?,
            tolerance: angle_tolerance(parameters, "angle_tolerance")?
                .ok_or_else(|| invalid("`orientation` needs an `angle_tolerance`"))?,
        }),
        (None, None) => {
            if parameters.quantity("aisle_reach")?.is_some()
                || parameters.quantity("angle_tolerance")?.is_some()
            {
                return Err(invalid(
                    "`aisle_reach` and `angle_tolerance` need `aisles` and `orientation`",
                ));
            }
            None
        }
        _ => return Err(invalid("`aisles` and `orientation` are declared together")),
    };
    let obstacles = parameters.selector("obstacles")?;
    let ends = Allowed::parse("end_obstructions", parameters.string("end_obstructions")?)?;
    let sides = Allowed::parse("side_obstructions", parameters.string("side_obstructions")?)?;
    let reach = length("obstruction_reach")?;
    let obstructions = match (obstacles, reach, ends, sides) {
        (None, None, None, None) => None,
        (Some(obstacles), Some(reach), Some(ends), Some(sides)) => Some(Obstructions {
            obstacles,
            reach,
            ends,
            sides,
        }),
        _ => {
            return Err(invalid(
                "`obstacles`, `obstruction_reach`, `end_obstructions` and `side_obstructions` \
                 are declared together",
            ));
        }
    };
    let config = Config {
        width: range("width")?,
        length: range("length")?,
        height: range("height")?,
        orientation,
        obstructions,
    };
    if !config.sized()
        && !config.high()
        && config.orientation.is_none()
        && config.obstructions.is_none()
    {
        return Err(invalid(
            "declare a size bound, an `orientation` or `obstacles`: nothing is checked",
        ));
    }
    Ok(config)
}

struct Services<'a> {
    rectangles: Option<&'a PlanSpanServiceHandle>,
    extents: Option<&'a VerticalExtentServiceHandle>,
    proximity: Option<&'a ProximityServiceHandle>,
}

impl<'a> Services<'a> {
    fn of(context: &RuleContext<'a>, config: &Config<'_>) -> Result<Self, Unavailable> {
        let missing = |what: &str| {
            (
                NotEvaluatedReason::MissingService,
                format!("{what} service is not registered"),
            )
        };
        let axes = config.sized() || config.orientation.is_some() || config.obstructions.is_some();
        let near = config.orientation.is_some() || config.obstructions.is_some();
        Ok(Self {
            rectangles: if axes {
                Some(rectangle_service(context)?)
            } else {
                None
            },
            extents: if config.high() || config.obstructions.is_some() {
                Some(
                    context
                        .services
                        .get::<VerticalExtentServiceHandle>()
                        .ok_or_else(|| missing("vertical-extent"))?,
                )
            } else {
                None
            },
            proximity: if near {
                Some(
                    context
                        .services
                        .get::<ProximityServiceHandle>()
                        .ok_or_else(|| missing("proximity"))?,
                )
            } else {
                None
            },
        })
    }
}

/// The objects of one selector near each bay, from the plan broad phase.
struct Nearby {
    /// Objects the selector picks.
    matched: BTreeSet<ObjectId>,
    /// Objects near each bay, matched or undecided.
    near: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Matched or undecided objects whose extent cannot be read: they may
    /// stand near any bay.
    blind: BTreeSet<ObjectId>,
    /// Bays whose extent cannot be read.
    unbounded: BTreeMap<ObjectId, Unavailable>,
    /// Enclosing boxes, for every object with a readable extent.
    bounds: BTreeMap<ObjectId, ObjectBounds>,
}

impl Nearby {
    fn find(
        context: &RuleContext<'_>,
        services: &Services<'_>,
        selector: &Selector,
        bays: &[&Object],
        reach: f64,
    ) -> Result<Self, Unavailable> {
        let proximity = services.proximity.ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "proximity service is not registered".to_owned(),
            )
        })?;
        let (matched, selection) = select_objects(context, selector);
        let matched: BTreeSet<ObjectId> = matched.iter().map(|object| object.id.clone()).collect();
        let undecided: BTreeSet<ObjectId> = selection
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect();
        let read = |object: &ObjectId| match proximity.bounds(object) {
            Ok(extent) if extent.object() == object => Ok(extent),
            Ok(_) => Err((
                NotEvaluatedReason::InvalidEvidence,
                "proximity bounds name a different object".to_owned(),
            )),
            Err(error) => Err((proximity_reason(error), error.to_string())),
        };
        let mut found = Self {
            matched,
            near: BTreeMap::new(),
            blind: BTreeSet::new(),
            unbounded: BTreeMap::new(),
            bounds: BTreeMap::new(),
        };
        let mut bay_bounds = Vec::new();
        for bay in bays {
            match read(&bay.id) {
                Ok(extent) => {
                    found.bounds.insert(bay.id.clone(), extent.clone());
                    bay_bounds.push(extent);
                }
                Err(unavailable) => {
                    found.unbounded.insert(bay.id.clone(), unavailable);
                }
            }
        }
        let mut others = Vec::new();
        for object in found.matched.iter().chain(&undecided) {
            match read(object) {
                Ok(extent) => {
                    found.bounds.insert(object.clone(), extent.clone());
                    others.push(extent);
                }
                Err(_) => {
                    found.blind.insert(object.clone());
                }
            }
        }
        let pairs =
            projected_candidate_pairs(&bay_bounds, &others, ProximityProjection::Horizontal, reach)
                .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))?;
        for pair in pairs {
            if pair.subject() != pair.counterpart() {
                found
                    .near
                    .entry(pair.subject().clone())
                    .or_default()
                    .push(pair.counterpart().clone());
            }
        }
        Ok(found)
    }

    fn near(&self, bay: &ObjectId) -> &[ObjectId] {
        self.near.get(bay).map_or(&[], Vec::as_slice)
    }
}

/// A finding (message, evidence, related objects), or `None` for a pass.
type Check = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

struct Bay<'s, 'a> {
    config: &'s Config<'s>,
    services: &'s Services<'a>,
    object: &'s Object,
}

/// The four edges of a bay: along its long axis (ends) or across it
/// (sides), on the high or low side.
const EDGES: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

impl Bay<'_, '_> {
    fn id(&self) -> &ObjectId {
        &self.object.id
    }

    fn checks(&self, aisles: Option<&Nearby>, obstacles: Option<&Nearby>) -> Vec<Check> {
        let mut checks = Vec::new();
        let rectangle = self
            .services
            .rectangles
            .map(|service| rectangle(service, self.id()));
        if self.config.sized() {
            checks.extend(self.size(rectangle.as_ref()));
        }
        if self.config.high() {
            checks.push(self.height());
        }
        if let (Some(orientation), Some(aisles)) = (&self.config.orientation, aisles) {
            checks.push(self.orientation(orientation, aisles, rectangle.as_ref()));
        }
        if let (Some(obstructions), Some(obstacles)) = (&self.config.obstructions, obstacles) {
            checks.extend(self.obstructions(obstructions, obstacles, rectangle.as_ref()));
        }
        checks
    }

    fn size(&self, rectangle: Option<&Result<PlanRectangle, Unavailable>>) -> Vec<Check> {
        let rectangle = match rectangle {
            Some(Ok(rectangle)) => rectangle,
            Some(Err(error)) => return vec![Err(error.clone())],
            None => return Vec::new(),
        };
        let sides = match rectangle.width_and_length() {
            Ok(sides) => sides,
            Err(reason) => {
                return vec![Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("its own axes are unknown: {reason}"),
                ))];
            }
        };
        [
            ("width", sides[0], self.config.width),
            ("length", sides[1], self.config.length),
        ]
        .into_iter()
        .filter(|(_, _, (low, high))| low.is_some() || high.is_some())
        .map(|(name, measured, (low, high))| {
            bounded(
                &format!("{name} along the bay's own axes"),
                measured,
                low,
                high,
                vec![rectangle.evidence().clone()],
            )
        })
        .collect()
    }

    fn height(&self) -> Check {
        let Some(extents) = self.services.extents else {
            return Ok(None);
        };
        let extent = extents
            .measure_vertical_extent(self.id())
            .map_err(|error| extent_unavailable(&error))?;
        let (low, high) = self.config.height;
        bounded(
            "height",
            extent.height_metres(),
            low,
            high,
            vec![extent.evidence().clone()],
        )
    }

    fn orientation(
        &self,
        orientation: &Orientation<'_>,
        aisles: &Nearby,
        rectangle: Option<&Result<PlanRectangle, Unavailable>>,
    ) -> Check {
        if let Some(unavailable) = aisles.unbounded.get(self.id()) {
            return Err(unavailable.clone());
        }
        let (Some(rectangles), Some(proximity)) =
            (self.services.rectangles, self.services.proximity)
        else {
            return Ok(None);
        };
        let own = match rectangle {
            Some(Ok(rectangle)) => rectangle,
            Some(Err(error)) => return Err(error.clone()),
            None => return Ok(None),
        };
        let wanted = orientation.alignment;
        let mut unknown: Vec<String> = aisles
            .blind
            .iter()
            .map(|aisle| format!("{aisle} has no readable extent and may be near"))
            .collect();
        let mut evidence = vec![own.evidence().clone()];
        let mut seen = Vec::new();
        for aisle in aisles.near(self.id()) {
            let within = match within_reach(proximity, self.id(), aisle, orientation.reach) {
                Ok((within, cited)) => {
                    evidence.push(cited);
                    within
                }
                Err((_, message)) => {
                    unknown.push(format!("whether {aisle} is near is unknown: {message}"));
                    continue;
                }
            };
            if within == Tri::No {
                continue;
            }
            let selected = Tri::of(aisles.matched.contains(aisle), false);
            let theirs = match rectangle_of(rectangles, aisle) {
                Ok(theirs) => theirs,
                Err(message) => {
                    unknown.push(message);
                    continue;
                }
            };
            evidence.push(theirs.evidence().clone());
            let (holds, why) = aligned(own, &theirs, wanted, orientation.tolerance);
            if within == Tri::Yes && selected == Tri::Yes && holds == Tri::Yes {
                return Ok(None);
            }
            if holds.possible() {
                unknown.push(why.unwrap_or_else(|| {
                    format!("{aisle} may be the aisle: it is near or selected only possibly")
                }));
            }
            seen.push(aisle.clone());
        }
        if !unknown.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether the bay is {} to an aisle is unknown: {}",
                    wanted.name(),
                    unknown.join("; ")
                ),
            ));
        }
        let message = if seen.is_empty() {
            format!(
                "no aisle lies within {} m of the bay, so it is not {} to one",
                orientation.reach,
                wanted.name()
            )
        } else {
            format!(
                "the bay is not {} to any aisle within {} m, within {} degrees",
                wanted.name(),
                orientation.reach,
                orientation.tolerance
            )
        };
        Ok(Some((message, evidence, seen)))
    }

    #[allow(clippy::too_many_lines)]
    fn obstructions(
        &self,
        obstructions: &Obstructions<'_>,
        obstacles: &Nearby,
        rectangle: Option<&Result<PlanRectangle, Unavailable>>,
    ) -> Vec<Check> {
        if let Some(unavailable) = obstacles.unbounded.get(self.id()) {
            return vec![Err(unavailable.clone())];
        }
        let (Some(proximity), Some(extents)) = (self.services.proximity, self.services.extents)
        else {
            return Vec::new();
        };
        let blind = obstacles.blind.len();
        let mut unknown = Vec::new();
        if blind > 0 {
            unknown.push(format!(
                "{blind} obstacle(s) have no readable extent and may stand anywhere"
            ));
        }
        let mut evidence = Vec::new();
        let mut candidates = Vec::new();
        for obstacle in obstacles.near(self.id()) {
            match within_reach(proximity, self.id(), obstacle, obstructions.reach) {
                Ok((Tri::No, _)) => {}
                Ok((within, cited)) => {
                    evidence.push(cited);
                    let selected = Tri::of(obstacles.matched.contains(obstacle), false);
                    candidates.push((obstacle.clone(), within.and(selected)));
                }
                Err((_, message)) => {
                    unknown.push(format!("whether {obstacle} is near is unknown: {message}"));
                }
            }
        }
        if candidates.is_empty() && unknown.is_empty() {
            return Vec::new();
        }
        let own = match rectangle {
            Some(Ok(rectangle)) => rectangle,
            Some(Err(error)) => return vec![Err(error.clone())],
            None => return Vec::new(),
        };
        let long = match own.long_axis() {
            Ok(long) => long,
            Err(reason) => {
                return vec![Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("its ends and sides are unknown: {reason}"),
                ))];
            }
        };
        evidence.push(own.evidence().clone());
        let axes = own.axes();
        let (along_axis, across_axis) = (axes[long], axes[1 - long]);
        let positions = |object: &ObjectId| -> Result<[[(f64, f64); 2]; 2], Unavailable> {
            Ok([
                along(extents, object, along_axis)?,
                along(extents, object, across_axis)?,
            ])
        };
        let bay = match positions(self.id()) {
            Ok(bay) => bay,
            Err(error) => return vec![Err(error)],
        };
        // Positions along an axis turned by up to the rectangle's axis error
        // move by at most that angle times the distance from the centre.
        let margin = |object: &ObjectId| {
            let centre = own.centre();
            let reach = [self.id(), object]
                .into_iter()
                .filter_map(|id| obstacles.bounds.get(id))
                .flat_map(|bounds| {
                    let (min, max) = (bounds.enclosing().min(), bounds.enclosing().max());
                    [min[0], max[0]].into_iter().flat_map(move |x| {
                        [min[1], max[1]]
                            .into_iter()
                            .map(move |y| (x - centre[0]).hypot(y - centre[1]))
                    })
                })
                .fold(0.0, f64::max);
            2.0 * reach * own.axis_error_radians().sin() + own.centre_radius_metres()
        };
        // Per edge: obstacles surely and possibly obstructing it.
        let mut sure: [BTreeSet<ObjectId>; 4] = Default::default();
        let mut possible: [BTreeSet<ObjectId>; 4] = Default::default();
        let mut inside = Vec::new();
        let mut maybe_inside = Vec::new();
        for (obstacle, near) in candidates {
            let theirs = match positions(&obstacle) {
                Ok(theirs) => theirs,
                Err((_, message)) => {
                    unknown.push(format!("where {obstacle} stands is unknown: {message}"));
                    continue;
                }
            };
            let m = margin(&obstacle);
            let mut past_none = Tri::Yes;
            for (index, &(ends, high)) in EDGES.iter().enumerate() {
                let (normal, span) = if ends { (0, 1) } else { (1, 0) };
                let past = beyond(bay[normal], theirs[normal], high, m);
                past_none = past_none.and(past.not());
                match near.and(past).and(overlaps(bay[span], theirs[span], m)) {
                    Tri::Yes => sure[index].insert(obstacle.clone()),
                    Tri::Maybe => possible[index].insert(obstacle.clone()),
                    Tri::No => false,
                };
            }
            match near.and(past_none) {
                Tri::Yes => inside.push(obstacle),
                Tri::Maybe => maybe_inside.push(obstacle),
                Tri::No => {}
            }
        }
        let mut checks = Vec::new();
        if !inside.is_empty() {
            checks.push(Ok(Some((
                format!(
                    "{} within the bay, past none of its edges",
                    names(&inside, "stands", "stand")
                ),
                evidence.clone(),
                inside,
            ))));
        } else if !maybe_inside.is_empty() || !unknown.is_empty() {
            let mut reasons = unknown.clone();
            if !maybe_inside.is_empty() {
                reasons.push(format!("{} may stand within the bay", list(&maybe_inside)));
            }
            checks.push(Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether an obstacle stands within the bay is unknown: {}",
                    reasons.join("; ")
                ),
            )));
        }
        for (label, first, allowed) in [
            ("ends", 0, obstructions.ends),
            ("sides", 2, obstructions.sides),
        ] {
            let edges = first..first + 2;
            let surely = edges.clone().filter(|&edge| !sure[edge].is_empty()).count();
            // An obstacle that cannot be placed may obstruct either edge.
            let maybe = if unknown.is_empty() {
                edges
                    .clone()
                    .filter(|&edge| !sure[edge].is_empty() || !possible[edge].is_empty())
                    .count()
            } else {
                2
            };
            let related: Vec<ObjectId> = sure[edges.clone()]
                .iter()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if surely > allowed.0 {
                checks.push(Ok(Some((
                    format!(
                        "{surely} of its {label} obstructed by {}, {} allowed",
                        list(&related),
                        allowed.name()
                    ),
                    evidence.clone(),
                    related,
                ))));
            } else if maybe > allowed.0 {
                let mut reasons = unknown.clone();
                let open: Vec<ObjectId> = possible[edges]
                    .iter()
                    .flatten()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if !open.is_empty() {
                    reasons.push(format!("{} may obstruct them", list(&open)));
                }
                checks.push(Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "how many of its {label} are obstructed is unknown, {} allowed: {}",
                        allowed.name(),
                        reasons.join("; ")
                    ),
                )));
            }
        }
        checks
    }
}

/// Whether the obstacle reaches past the bay's edge on the `high` or low
/// side along one axis: past its highest (or lowest) position, by more than
/// `margin`.
fn beyond(bay: [(f64, f64); 2], obstacle: [(f64, f64); 2], high: bool, margin: f64) -> Tri {
    if high {
        let (edge, reach) = (bay[1], obstacle[1]);
        Tri::of(reach.0 > edge.1 + margin, reach.1 <= edge.0 - margin)
    } else {
        let (edge, reach) = (bay[0], obstacle[0]);
        Tri::of(reach.1 < edge.0 - margin, reach.0 >= edge.1 + margin)
    }
}

/// Whether the obstacle's span overlaps the bay's across one axis, openly.
fn overlaps(bay: [(f64, f64); 2], obstacle: [(f64, f64); 2], margin: f64) -> Tri {
    Tri::of(
        obstacle[1].0 > bay[0].1 + margin && obstacle[0].1 < bay[1].0 - margin,
        obstacle[1].1 <= bay[0].0 - margin || obstacle[0].0 >= bay[1].1 + margin,
    )
}

/// Whether `other` lies within `reach` of `bay` in plan.
fn within_reach(
    proximity: &ProximityServiceHandle,
    bay: &ObjectId,
    other: &ObjectId,
    reach: f64,
) -> Result<(Tri, Evidence), Unavailable> {
    let request =
        ProximityRequest::projected(bay.clone(), other.clone(), ProximityProjection::Horizontal)
            .map_err(|error| (proximity_reason(error), error.to_string()))?;
    let measured = proximity
        .measure_distance(&request)
        .map_err(|error| (proximity_reason(error), error.to_string()))?;
    let (low, high) = measured.interval_metres();
    Ok((
        Tri::of(high <= reach, low > reach),
        measured.evidence().clone(),
    ))
}

/// Another object's rectangle, or why it is unknown.
fn rectangle_of(
    service: &PlanSpanServiceHandle,
    object: &ObjectId,
) -> Result<PlanRectangle, String> {
    rectangle(service, object)
        .map_err(|(_, message)| format!("the axes of {object} are unknown: {message}"))
}

/// A measured length against inclusive bounds.
fn bounded(
    what: &str,
    (low, high): (f64, f64),
    minimum: Option<f64>,
    maximum: Option<f64>,
    evidence: Vec<Evidence>,
) -> Check {
    let measured = format!("{what} is {} m", shown(low, high));
    match judge(low, high, minimum, maximum) {
        Verdict::Pass => Ok(None),
        Verdict::Fail(bound) => Ok(Some((
            format!("{measured}; {bound} m"),
            evidence,
            Vec::new(),
        ))),
        Verdict::Undecided(bound) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{measured}, which straddles {bound} m"),
        )),
    }
}

fn list(objects: &[ObjectId]) -> String {
    objects
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn names(objects: &[ObjectId], one: &str, many: &str) -> String {
    format!(
        "{} {}",
        list(objects),
        if objects.len() == 1 { one } else { many }
    )
}
