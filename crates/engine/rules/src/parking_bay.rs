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
///   and overlaps the end's span across the bay, likewise a side; with
///   `side_zone_length`, a side only when it overlaps the central stretch
///   of the side that long. `end_obstructions` and `side_obstructions`
///   (`none`, `one` or `both`) say how many ends and sides may be
///   obstructed; an obstacle within the bay's rectangle, past none of its
///   edges, is always a finding.
///
/// With `applies_when` `filter`, orientation and obstructions are no
/// findings: they select the bays the size bounds apply to. `orientations`
/// (any of `parallel`, `perpendicular`, `angled`, `unclear`), `end_states`
/// and `side_states` (any of `none`, `one`, `both`) are the states a bay
/// must be in. A bay's orientation is its long axis's to the aisles within
/// reach, or, with `neighbour_reach` instead of `aisles`, the direction to
/// its neighbouring bays whose long axes are parallel to its own: beside it
/// perpendicular, before or behind it parallel, in between angled. No such
/// aisle or neighbour, or ones that disagree, make it `unclear`.
///
/// Every measure is an interval: a check passes when the whole interval
/// satisfies it, is a finding when none of it does, and is otherwise not
/// evaluated. A bay whose orientation is not unique (several least-area
/// rectangles), whose footprint is tessellated, or whose sides are too
/// close to equal for a long axis is not evaluated where its axes decide.
/// Objects the selectors cannot decide can only add aisles, neighbours or
/// obstructions: they turn a finding they could remove, or a pass they
/// could break, into not evaluated, and a bay whose states they leave open
/// has a size finding not evaluated.
pub struct ParkingBay;

/// How many of a bay's two ends, or two sides, may be obstructed.
#[derive(Clone, Copy)]
struct Allowed(usize);

impl Allowed {
    fn count(name: &str, value: &str) -> Result<usize, Unavailable> {
        match value {
            "none" => Ok(0),
            "one" => Ok(1),
            "both" => Ok(2),
            other => Err(invalid(format!(
                "{name} `{other}` is unsupported; use `none`, `one` or `both`"
            ))),
        }
    }

    fn parse(name: &str, value: Option<&str>) -> Result<Option<Self>, Unavailable> {
        value
            .map(|value| Self::count(name, value).map(Self))
            .transpose()
    }

    fn name(self) -> &'static str {
        count_name(self.0)
    }
}

fn count_name(count: usize) -> &'static str {
    match count {
        0 => "none",
        1 => "one",
        _ => "both",
    }
}

/// A bay's orientation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum State {
    Parallel,
    Perpendicular,
    Angled,
    Unclear,
}

impl State {
    const ALL: [Self; 4] = [
        Self::Parallel,
        Self::Perpendicular,
        Self::Angled,
        Self::Unclear,
    ];

    fn parse(value: &str) -> Result<Self, Unavailable> {
        match value {
            "unclear" => Ok(Self::Unclear),
            other => Alignment::parse(other).map(Self::of).map_err(|_| {
                invalid(format!(
                    "orientation state `{other}` is unsupported; use `parallel`, \
                         `perpendicular`, `angled` or `unclear`"
                ))
            }),
        }
    }

    fn of(alignment: Alignment) -> Self {
        match alignment {
            Alignment::Parallel => Self::Parallel,
            Alignment::Perpendicular => Self::Perpendicular,
            Alignment::Angled => Self::Angled,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Perpendicular => "perpendicular",
            Self::Angled => "angled",
            Self::Unclear => "unclear",
        }
    }
}

const ALIGNMENTS: [Alignment; 3] = [
    Alignment::Parallel,
    Alignment::Perpendicular,
    Alignment::Angled,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Applies {
    Findings,
    Filter,
}

/// The objects a bay's orientation is read against.
enum Reference<'a> {
    /// Aisles within `reach`.
    Aisles { aisles: &'a Selector, reach: f64 },
    /// Neighbouring bays within `reach`.
    Neighbours { reach: f64 },
}

struct Orientation<'a> {
    reference: Reference<'a>,
    tolerance: f64,
    /// The alignment a finding is judged against, in findings mode.
    wanted: Option<Alignment>,
}

struct Obstructions<'a> {
    obstacles: &'a Selector,
    reach: f64,
    /// How many may be obstructed, in findings mode.
    ends: Option<Allowed>,
    sides: Option<Allowed>,
    /// The central stretch of a side an obstruction must overlap.
    side_zone: Option<f64>,
}

/// The states a bay must be in for its size bounds to apply.
#[derive(Default)]
struct Filters {
    orientations: Option<BTreeSet<State>>,
    ends: Option<BTreeSet<usize>>,
    sides: Option<BTreeSet<usize>>,
}

struct Config<'a> {
    width: (Option<f64>, Option<f64>),
    length: (Option<f64>, Option<f64>),
    height: (Option<f64>, Option<f64>),
    applies: Applies,
    orientation: Option<Orientation<'a>>,
    obstructions: Option<Obstructions<'a>>,
    filters: Filters,
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
            ParameterDescriptor::optional("applies_when", ParameterType::String),
            ParameterDescriptor::optional("orientations", ParameterType::StringList),
            ParameterDescriptor::optional("end_states", ParameterType::StringList),
            ParameterDescriptor::optional("side_states", ParameterType::StringList),
            ParameterDescriptor::optional("side_zone_length", ParameterType::Quantity),
            ParameterDescriptor::optional("neighbour_reach", ParameterType::Quantity),
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
        let references = match config.orientation.as_ref().map(|o| match &o.reference {
            Reference::Aisles { aisles, reach } => near(aisles, *reach),
            Reference::Neighbours { reach } => near(&rule.selector, *reach),
        }) {
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
            for check in judged.checks(references.as_ref(), obstacles.as_ref()) {
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

fn length_of(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some((_, QuantityDimension::Length)) => Err(invalid(format!("{name} is negative"))),
        Some(_) => Err(invalid(format!("{name} must be a length"))),
    }
}

/// A non-empty set of states from a string list.
fn states<T: Ord>(
    parameters: &Parameters<'_>,
    name: &str,
    parse: impl Fn(&str) -> Result<T, Unavailable>,
) -> Result<Option<BTreeSet<T>>, Unavailable> {
    let Some(values) = parameters.strings(name)? else {
        return Ok(None);
    };
    if values.is_empty() {
        return Err(invalid(format!("{name} states nothing")));
    }
    values
        .iter()
        .map(|value| parse(value.trim()))
        .collect::<Result<BTreeSet<T>, _>>()
        .map(Some)
}

fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
    let length = |name: &str| length_of(parameters, name);
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
    let applies = match parameters.string("applies_when")?.unwrap_or("findings") {
        "findings" => Applies::Findings,
        "filter" => Applies::Filter,
        other => {
            return Err(invalid(format!(
                "applies_when `{other}` is unsupported; use `findings` or `filter`"
            )));
        }
    };
    let filters = Filters {
        orientations: states(parameters, "orientations", State::parse)?,
        ends: states(parameters, "end_states", |value| {
            Allowed::count("end_states", value)
        })?,
        sides: states(parameters, "side_states", |value| {
            Allowed::count("side_states", value)
        })?,
    };
    let filtered =
        filters.orientations.is_some() || filters.ends.is_some() || filters.sides.is_some();
    match applies {
        Applies::Findings if filtered => {
            return Err(invalid(
                "`orientations`, `end_states` and `side_states` need `applies_when` `filter`",
            ));
        }
        Applies::Filter if !filtered => {
            return Err(invalid(
                "`applies_when` `filter` needs `orientations`, `end_states` or `side_states`",
            ));
        }
        _ => {}
    }
    let config = Config {
        width: range("width")?,
        length: range("length")?,
        height: range("height")?,
        applies,
        orientation: orientation(parameters, applies, filters.orientations.is_some())?,
        obstructions: obstructions(parameters, applies, &filters)?,
        filters,
    };
    let size = config.sized() || config.high();
    if applies == Applies::Filter && !size {
        return Err(invalid(
            "`applies_when` `filter` selects the bays a size bound applies to: declare one",
        ));
    }
    if !size && config.orientation.is_none() && config.obstructions.is_none() {
        return Err(invalid(
            "declare a size bound, an `orientation` or `obstacles`: nothing is checked",
        ));
    }
    Ok(config)
}

fn orientation<'a>(
    parameters: &Parameters<'a>,
    applies: Applies,
    filtered: bool,
) -> Result<Option<Orientation<'a>>, Unavailable> {
    let aisles = parameters.selector("aisles")?;
    let aisle_reach = length_of(parameters, "aisle_reach")?;
    let neighbour_reach = length_of(parameters, "neighbour_reach")?;
    let wanted = parameters.string("orientation")?;
    let tolerance = angle_tolerance(parameters, "angle_tolerance")?;
    if applies == Applies::Filter && wanted.is_some() {
        return Err(invalid(
            "`orientation` is a finding; with `applies_when` `filter` declare `orientations`",
        ));
    }
    if neighbour_reach.is_some() && (aisles.is_some() || !filtered) {
        return Err(invalid(
            "`neighbour_reach` infers the orientation for `orientations` when no `aisles` \
             are declared",
        ));
    }
    if aisle_reach.is_some() && aisles.is_none() {
        return Err(invalid("`aisle_reach` needs `aisles`"));
    }
    let reference = match (aisles, neighbour_reach) {
        (Some(aisles), None) => Some(Reference::Aisles {
            aisles,
            reach: aisle_reach.unwrap_or(0.0),
        }),
        (None, Some(reach)) => Some(Reference::Neighbours { reach }),
        _ => None,
    };
    let used = wanted.is_some() || filtered;
    match (reference, used) {
        (None, false) => {
            if tolerance.is_some() {
                return Err(invalid(
                    "`angle_tolerance` needs `aisles` and `orientation`",
                ));
            }
            Ok(None)
        }
        (Some(reference), true) => Ok(Some(Orientation {
            reference,
            tolerance: tolerance
                .ok_or_else(|| invalid("an orientation needs an `angle_tolerance`"))?,
            wanted: wanted.map(Alignment::parse).transpose()?,
        })),
        (None, true) if filtered => Err(invalid(
            "`orientations` needs `aisles` or `neighbour_reach`",
        )),
        _ => Err(invalid("`aisles` and `orientation` are declared together")),
    }
}

fn obstructions<'a>(
    parameters: &Parameters<'a>,
    applies: Applies,
    filters: &Filters,
) -> Result<Option<Obstructions<'a>>, Unavailable> {
    let obstacles = parameters.selector("obstacles")?;
    let ends = Allowed::parse("end_obstructions", parameters.string("end_obstructions")?)?;
    let sides = Allowed::parse("side_obstructions", parameters.string("side_obstructions")?)?;
    let reach = length_of(parameters, "obstruction_reach")?;
    let side_zone = length_of(parameters, "side_zone_length")?;
    if side_zone == Some(0.0) {
        return Err(invalid("side_zone_length must be positive"));
    }
    let counted = filters.ends.is_some() || filters.sides.is_some();
    let obstructions = match applies {
        Applies::Findings => match (obstacles, reach, ends, sides) {
            (None, None, None, None) => None,
            (Some(obstacles), Some(reach), Some(ends), Some(sides)) => Some(Obstructions {
                obstacles,
                reach,
                ends: Some(ends),
                sides: Some(sides),
                side_zone,
            }),
            _ => {
                return Err(invalid(
                    "`obstacles`, `obstruction_reach`, `end_obstructions` and \
                     `side_obstructions` are declared together",
                ));
            }
        },
        Applies::Filter => {
            if ends.is_some() || sides.is_some() {
                return Err(invalid(
                    "`end_obstructions` and `side_obstructions` are findings; with \
                     `applies_when` `filter` declare `end_states` and `side_states`",
                ));
            }
            match (obstacles, reach) {
                (None, None) if !counted => None,
                (Some(obstacles), Some(reach)) if counted => Some(Obstructions {
                    obstacles,
                    reach,
                    ends: None,
                    sides: None,
                    side_zone,
                }),
                _ => {
                    return Err(invalid(
                        "`end_states` and `side_states` need `obstacles` and \
                         `obstruction_reach`, and those need one of them",
                    ));
                }
            }
        }
    };
    if side_zone.is_some() && obstructions.is_none() {
        return Err(invalid("`side_zone_length` needs `obstacles`"));
    }
    Ok(obstructions)
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
        // The broad phase names a pair of two bays once; each is near the
        // other.
        let is_bay: BTreeSet<&ObjectId> = bays.iter().map(|bay| &bay.id).collect();
        for pair in pairs {
            let (subject, counterpart) = (pair.subject(), pair.counterpart());
            if subject == counterpart {
                continue;
            }
            let mut add = |from: &ObjectId, to: &ObjectId| {
                let near = found.near.entry(from.clone()).or_default();
                if !near.contains(to) {
                    near.push(to.clone());
                }
            };
            add(subject, counterpart);
            if is_bay.contains(counterpart) {
                add(counterpart, subject);
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

/// The obstructions counted at one pair of edges: surely and at most, with
/// the obstacles surely and possibly obstructing them.
#[derive(Default)]
struct Count {
    surely: usize,
    most: usize,
    related: Vec<ObjectId>,
    open: Vec<ObjectId>,
}

impl Count {
    fn possible(&self) -> BTreeSet<usize> {
        (self.surely..=self.most).collect()
    }
}

/// What the obstacles near a bay do: an obstacle within it, and the
/// obstructed ends and sides.
#[derive(Default)]
struct Counted {
    inside: Option<Check>,
    ends: Count,
    sides: Count,
    unknown: Vec<String>,
    evidence: Vec<Evidence>,
}

/// What a bay's orientation may be.
struct Oriented {
    possible: BTreeSet<State>,
    reasons: Vec<String>,
    evidence: Vec<Evidence>,
}

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

    fn checks(&self, references: Option<&Nearby>, obstacles: Option<&Nearby>) -> Vec<Check> {
        let rectangle = self
            .services
            .rectangles
            .map(|service| rectangle(service, self.id()));
        let mut sizes = Vec::new();
        if self.config.sized() {
            sizes.extend(self.size(rectangle.as_ref()));
        }
        if self.config.high() {
            sizes.push(self.height());
        }
        let counted = match (&self.config.obstructions, obstacles) {
            (Some(obstructions), Some(obstacles)) => {
                Some(self.count(obstructions, obstacles, rectangle.as_ref()))
            }
            _ => None,
        };
        let mut checks = Vec::new();
        if let Some(Ok(counted)) = &counted
            && let Some(inside) = &counted.inside
        {
            checks.push(inside.clone());
        }
        match self.config.applies {
            Applies::Findings => {
                checks.extend(sizes);
                if let (Some(orientation), Some(aisles), Some(wanted)) = (
                    &self.config.orientation,
                    references,
                    self.config.orientation.as_ref().and_then(|o| o.wanted),
                ) {
                    checks.push(self.orientation(orientation, wanted, aisles, rectangle.as_ref()));
                }
                if let (Some(obstructions), Some(counted)) = (&self.config.obstructions, counted) {
                    match counted {
                        Ok(counted) => checks.extend(judge_counts(obstructions, &counted)),
                        Err(error) => checks.push(Err(error)),
                    }
                }
            }
            Applies::Filter => {
                let oriented = match (&self.config.orientation, references) {
                    (Some(orientation), Some(references)) => {
                        Some(self.oriented(orientation, references, rectangle.as_ref()))
                    }
                    _ => None,
                };
                checks.extend(self.filtered(sizes, oriented.as_ref(), counted.as_ref()));
            }
        }
        checks
    }

    /// The size checks, applied to this bay only as far as its states are
    /// the filters'.
    fn filtered(
        &self,
        sizes: Vec<Check>,
        oriented: Option<&Oriented>,
        counted: Option<&Result<Counted, Unavailable>>,
    ) -> Vec<Check> {
        let filters = &self.config.filters;
        let mut applies = Tri::Yes;
        let mut why = Vec::new();
        let mut said = Vec::new();
        let mut evidence = Vec::new();
        if let (Some(allowed), Some(oriented)) = (&filters.orientations, oriented) {
            let names: Vec<&str> = oriented.possible.iter().map(|state| state.name()).collect();
            let answer = within(&oriented.possible, allowed);
            if answer == Tri::Maybe {
                let mut reasons = oriented.reasons.clone();
                reasons.sort();
                reasons.dedup();
                why.push(format!(
                    "its orientation may be {} ({})",
                    names.join(" or "),
                    reasons.join("; ")
                ));
            }
            said.push(format!("orientation {}", names.join(" or ")));
            evidence.extend(oriented.evidence.iter().cloned());
            applies = applies.and(answer);
        }
        for (label, allowed) in [("ends", &filters.ends), ("sides", &filters.sides)] {
            let Some(allowed) = allowed else { continue };
            let (answer, possible) = match counted {
                Some(Ok(counted)) => {
                    let count = if label == "ends" {
                        &counted.ends
                    } else {
                        &counted.sides
                    };
                    let possible = count.possible();
                    let answer = within(&possible, allowed);
                    if answer == Tri::Maybe {
                        let mut reasons = counted.unknown.clone();
                        if !count.open.is_empty() {
                            reasons.push(format!("{} may obstruct them", list(&count.open)));
                        }
                        why.push(format!(
                            "{} of its {label} may be obstructed ({})",
                            possible
                                .iter()
                                .map(|count| count_name(*count))
                                .collect::<Vec<_>>()
                                .join(" or "),
                            reasons.join("; ")
                        ));
                    }
                    evidence.extend(counted.evidence.iter().cloned());
                    (answer, possible)
                }
                Some(Err((_, message))) => {
                    why.push(format!("its obstructed {label} are unknown: {message}"));
                    (Tri::Maybe, (0..=2).collect())
                }
                None => (Tri::Maybe, (0..=2).collect()),
            };
            said.push(format!(
                "{label} obstructed {}",
                possible
                    .iter()
                    .map(|count| count_name(*count))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ));
            applies = applies.and(answer);
        }
        let states = said.join(", ");
        match applies {
            Tri::No => Vec::new(),
            Tri::Yes => sizes
                .into_iter()
                .map(|check| {
                    check.map(|found| {
                        found.map(|(message, mut cited, related)| {
                            cited.extend(evidence.iter().cloned());
                            (format!("{message} (a bay with {states})"), cited, related)
                        })
                    })
                })
                .collect(),
            Tri::Maybe => {
                let why = why.join("; ");
                sizes
                    .into_iter()
                    .map(|check| match check {
                        Ok(None) => Ok(None),
                        Ok(Some((message, _, _))) => Err((
                            NotEvaluatedReason::IncompleteEvidence,
                            format!("{message}, if the bound applies to it: {why}"),
                        )),
                        Err((reason, message)) => Err((reason, format!("{message}; {why}"))),
                    })
                    .collect()
            }
        }
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
        wanted: Alignment,
        aisles: &Nearby,
        rectangle: Option<&Result<PlanRectangle, Unavailable>>,
    ) -> Check {
        let Reference::Aisles { reach, .. } = orientation.reference else {
            return Ok(None);
        };
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
        let mut unknown: Vec<String> = aisles
            .blind
            .iter()
            .map(|aisle| format!("{aisle} has no readable extent and may be near"))
            .collect();
        let mut evidence = vec![own.evidence().clone()];
        let mut seen = Vec::new();
        for aisle in aisles.near(self.id()) {
            let within = match within_reach(proximity, self.id(), aisle, reach) {
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
                "no aisle lies within {reach} m of the bay, so it is not {} to one",
                wanted.name()
            )
        } else {
            format!(
                "the bay is not {} to any aisle within {reach} m, within {} degrees",
                wanted.name(),
                orientation.tolerance
            )
        };
        Ok(Some((message, evidence, seen)))
    }

    /// The orientation states the bay may be in: one decided by every
    /// aisle or neighbour surely there, `unclear` when none or ones that
    /// disagree are, and more where an uncertain one might change it.
    #[allow(clippy::too_many_lines)]
    fn oriented(
        &self,
        orientation: &Orientation<'_>,
        references: &Nearby,
        rectangle: Option<&Result<PlanRectangle, Unavailable>>,
    ) -> Oriented {
        let everything = |reason: String, evidence: Vec<Evidence>| Oriented {
            possible: State::ALL.into_iter().collect(),
            reasons: vec![reason],
            evidence,
        };
        if let Some((_, message)) = references.unbounded.get(self.id()) {
            return everything(message.clone(), Vec::new());
        }
        let (Some(rectangles), Some(proximity)) =
            (self.services.rectangles, self.services.proximity)
        else {
            return everything("the services are not registered".into(), Vec::new());
        };
        let own = match rectangle {
            Some(Ok(rectangle)) => rectangle,
            Some(Err((_, message))) => return everything(message.clone(), Vec::new()),
            None => return everything("its rectangle was not measured".into(), Vec::new()),
        };
        let (reach, neighbours) = match orientation.reference {
            Reference::Aisles { reach, .. } => (reach, false),
            Reference::Neighbours { reach } => (reach, true),
        };
        let what = if neighbours { "neighbour" } else { "aisle" };
        let mut evidence = vec![own.evidence().clone()];
        let mut reasons = Vec::new();
        let mut open = false;
        for blind in &references.blind {
            if blind != self.id() {
                open = true;
                reasons.push(format!("{blind} has no readable extent and may be near"));
            }
        }
        // States surely given, and states uncertain ones might give.
        let mut decided = BTreeSet::new();
        let mut maybe = BTreeSet::new();
        for other in references.near(self.id()) {
            let within = match within_reach(proximity, self.id(), other, reach) {
                Ok((within, cited)) => {
                    evidence.push(cited);
                    within
                }
                Err((_, message)) => {
                    open = true;
                    reasons.push(format!("whether {other} is near is unknown: {message}"));
                    continue;
                }
            };
            if within == Tri::No {
                continue;
            }
            let selected = Tri::of(references.matched.contains(other), false);
            let theirs = match rectangle_of(rectangles, other) {
                Ok(theirs) => theirs,
                Err(message) => {
                    open = true;
                    reasons.push(message);
                    continue;
                }
            };
            evidence.push(theirs.evidence().clone());
            let (present, relation) = if neighbours {
                let (parallel, why) =
                    aligned(own, &theirs, Alignment::Parallel, orientation.tolerance);
                if let Some(why) = why {
                    reasons.push(why);
                }
                (
                    within.and(selected).and(parallel),
                    beside(own, &theirs, orientation.tolerance),
                )
            } else {
                let mut relation = BTreeMap::new();
                for alignment in ALIGNMENTS {
                    let (holds, why) = aligned(own, &theirs, alignment, orientation.tolerance);
                    if let Some(why) = why {
                        reasons.push(why);
                    }
                    relation.insert(State::of(alignment), holds);
                }
                (within.and(selected), Ok(relation))
            };
            if present == Tri::No {
                continue;
            }
            let relation = match relation {
                Ok(relation) => relation,
                Err(why) => {
                    reasons.push(why);
                    State::ALL[..3]
                        .iter()
                        .map(|state| (*state, Tri::Maybe))
                        .collect()
                }
            };
            let sure: Vec<State> = relation
                .iter()
                .filter(|(_, holds)| **holds == Tri::Yes)
                .map(|(state, _)| *state)
                .collect();
            let possible = relation
                .iter()
                .filter(|(_, holds)| holds.possible())
                .map(|(state, _)| *state);
            if present == Tri::Yes && sure.len() == 1 {
                decided.insert(sure[0]);
            } else {
                if present == Tri::Maybe {
                    reasons.push(format!("{other} may be a {what} within reach"));
                }
                maybe.extend(possible);
            }
        }
        let possible: BTreeSet<State> = if open {
            State::ALL.into_iter().collect()
        } else if decided.len() > 1 {
            BTreeSet::from([State::Unclear])
        } else if let Some(state) = decided.first() {
            let mut possible = BTreeSet::from([*state]);
            if maybe.iter().any(|other| other != state) {
                possible.insert(State::Unclear);
            }
            possible
        } else {
            let mut possible = maybe;
            possible.insert(State::Unclear);
            possible
        };
        Oriented {
            possible,
            reasons,
            evidence,
        }
    }

    /// Counts the ends and sides the obstacles near the bay obstruct.
    #[allow(clippy::too_many_lines)]
    fn count(
        &self,
        obstructions: &Obstructions<'_>,
        obstacles: &Nearby,
        rectangle: Option<&Result<PlanRectangle, Unavailable>>,
    ) -> Result<Counted, Unavailable> {
        if let Some(unavailable) = obstacles.unbounded.get(self.id()) {
            return Err(unavailable.clone());
        }
        let (Some(proximity), Some(extents)) = (self.services.proximity, self.services.extents)
        else {
            return Ok(Counted::default());
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
            return Ok(Counted::default());
        }
        let own = match rectangle {
            Some(Ok(rectangle)) => rectangle,
            Some(Err(error)) => return Err(error.clone()),
            None => return Ok(Counted::default()),
        };
        let long = own.long_axis().map_err(|reason| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!("its ends and sides are unknown: {reason}"),
            )
        })?;
        evidence.push(own.evidence().clone());
        let axes = own.axes();
        let (along_axis, across_axis) = (axes[long], axes[1 - long]);
        let positions = |object: &ObjectId| -> Result<[[(f64, f64); 2]; 2], Unavailable> {
            Ok([
                along(extents, object, along_axis)?,
                along(extents, object, across_axis)?,
            ])
        };
        let bay = positions(self.id())?;
        // The stretch of the sides an obstruction must overlap: the central
        // `side_zone` of them, or all of them.
        let zone = obstructions
            .side_zone
            .map_or(bay[0], |length| central(bay[0], length));
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
                let stretch = if ends { bay[span] } else { zone };
                let past = beyond(bay[normal], theirs[normal], high, m);
                past_none = past_none.and(past.not());
                match near.and(past).and(overlaps(stretch, theirs[span], m)) {
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
        let inside = if !inside.is_empty() {
            Some(Ok(Some((
                format!(
                    "{} within the bay, past none of its edges",
                    names(&inside, "stands", "stand")
                ),
                evidence.clone(),
                inside,
            ))))
        } else if !maybe_inside.is_empty() || !unknown.is_empty() {
            let mut reasons = unknown.clone();
            if !maybe_inside.is_empty() {
                reasons.push(format!("{} may stand within the bay", list(&maybe_inside)));
            }
            Some(Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether an obstacle stands within the bay is unknown: {}",
                    reasons.join("; ")
                ),
            )))
        } else {
            None
        };
        let count = |first: usize| {
            let edges = first..first + 2;
            let gather = |sets: &[BTreeSet<ObjectId>]| -> Vec<ObjectId> {
                sets.iter()
                    .flatten()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            };
            Count {
                surely: edges.clone().filter(|&edge| !sure[edge].is_empty()).count(),
                // An obstacle that cannot be placed may obstruct either edge.
                most: if unknown.is_empty() {
                    edges
                        .clone()
                        .filter(|&edge| !sure[edge].is_empty() || !possible[edge].is_empty())
                        .count()
                } else {
                    2
                },
                related: gather(&sure[edges.clone()]),
                open: gather(&possible[edges]),
            }
        };
        Ok(Counted {
            inside,
            ends: count(0),
            sides: count(2),
            unknown,
            evidence,
        })
    }
}

/// The obstruction findings of findings mode.
fn judge_counts(obstructions: &Obstructions<'_>, counted: &Counted) -> Vec<Check> {
    let mut checks = Vec::new();
    for (label, count, allowed) in [
        ("ends", &counted.ends, obstructions.ends),
        ("sides", &counted.sides, obstructions.sides),
    ] {
        let Some(allowed) = allowed else { continue };
        if count.surely > allowed.0 {
            checks.push(Ok(Some((
                format!(
                    "{} of its {label} obstructed by {}, {} allowed",
                    count.surely,
                    list(&count.related),
                    allowed.name()
                ),
                counted.evidence.clone(),
                count.related.clone(),
            ))));
        } else if count.most > allowed.0 {
            let mut reasons = counted.unknown.clone();
            if !count.open.is_empty() {
                reasons.push(format!("{} may obstruct them", list(&count.open)));
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

/// Whether every possible state is allowed (`Yes`), none is (`No`), or
/// some are.
fn within<T: Ord>(possible: &BTreeSet<T>, allowed: &BTreeSet<T>) -> Tri {
    Tri::of(possible.is_subset(allowed), possible.is_disjoint(allowed))
}

/// How a neighbour stands to the bay: the angle between the bay's long axis
/// and the direction to the neighbour's centre, judged as the alignment it
/// would make with an aisle running along the row (beside it perpendicular,
/// before or behind it parallel). `Err` when the centres are too close to
/// tell a direction.
fn beside(
    own: &PlanRectangle,
    theirs: &PlanRectangle,
    tolerance: f64,
) -> Result<BTreeMap<State, Tri>, String> {
    let long = own.long_axis()?;
    let axes = own.axes();
    let (along_axis, across_axis) = (axes[long], axes[1 - long]);
    let (a, b) = (own.centre(), theirs.centre());
    let d = [b[0] - a[0], b[1] - a[1]];
    let distance = d[0].hypot(d[1]);
    let error = own.centre_radius_metres() + theirs.centre_radius_metres();
    if distance <= error {
        return Err(format!(
            "the centres of {} and {} are too close to tell where one lies from the other",
            own.object(),
            theirs.object()
        ));
    }
    let along = d[0].mul_add(along_axis[0], d[1] * along_axis[1]).abs();
    let across = d[0].mul_add(across_axis[0], d[1] * across_axis[1]).abs();
    let angle = across.atan2(along).to_degrees();
    // The centres' uncertainty turns the direction by at most this much,
    // the axis by its error; plus the rounding of the arctangent.
    let slack =
        (error / distance).asin().to_degrees() + own.axis_error_radians().to_degrees() + 1e-9;
    let interval = ((angle - slack).max(0.0), (angle + slack).min(90.0));
    Ok(ALIGNMENTS
        .into_iter()
        .map(|alignment| (State::of(alignment), alignment.holds(interval, tolerance)))
        .collect())
}

/// The central stretch `length` long of a side spanning `side` (its lowest
/// and highest positions, each an interval), within the side.
fn central(side: [(f64, f64); 2], length: f64) -> [(f64, f64); 2] {
    let centre = (
        f64::midpoint(side[0].0, side[1].0),
        f64::midpoint(side[0].1, side[1].1),
    );
    let half = length / 2.0;
    [
        (
            side[0].0.max(centre.0 - half),
            side[0].1.max(centre.1 - half),
        ),
        (
            side[1].0.min(centre.0 + half),
            side[1].1.min(centre.1 + half),
        ),
    ]
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
