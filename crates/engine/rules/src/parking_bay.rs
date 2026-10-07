//! Parking bays: their size along their own axes, their orientation to the
//! aisle and the obstructions allowed at their ends and sides.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;

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
use crate::pairs::reason as proximity_reason;
#[cfg(feature = "parity-reference")]
use crate::plan_area::{Verdict, judge, shown};
#[cfg(feature = "parity-reference")]
use crate::selection::select_objects;
use crate::support::{Parameters, Unavailable, invalid};

mod items;
mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use items::BayItems;
pub(crate) use measured::BayMeasures;

pub(crate) const NAME: &str = "parking-bay";

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
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `parking_bay` list of each bay, its sizes against their bounds,
/// its counts of obstacles against what is allowed and its searches'
/// answers, judged by the template.
pub struct ParkingBay;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for ParkingBay {
    fn id(&self) -> &'static str {
        template::ID
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

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
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

/// Checks the rule parameters the measured `parking_bay` is handed, as the
/// rule states them: the capability's declaration, in its order and words.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, axioval_ir::contract::ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    parse(&Parameters(&rule)).map(|_| ())
}

/// How many of a bay's two ends, or two sides, may be obstructed.
#[derive(Clone, Copy)]
pub(crate) struct Allowed(usize);

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
pub(crate) enum Applies {
    Findings,
    Filter,
}

/// The objects a bay's orientation is read against.
pub(crate) enum Reference<'a> {
    /// Aisles within `reach`.
    Aisles {
        #[cfg(feature = "parity-reference")]
        aisles: &'a Selector,
        reach: f64,
        /// The selector's lifetime, which only the parity reference reads.
        marker: std::marker::PhantomData<&'a Selector>,
    },
    /// Neighbouring bays within `reach`.
    Neighbours { reach: f64 },
}

pub(crate) struct Orientation<'a> {
    pub(crate) reference: Reference<'a>,
    pub(crate) tolerance: f64,
    /// The alignment a finding is judged against, in findings mode.
    pub(crate) wanted: Option<Alignment>,
}

pub(crate) struct Obstructions<'a> {
    #[cfg(feature = "parity-reference")]
    pub(crate) obstacles: &'a Selector,
    /// The selector's lifetime, which only the parity reference reads.
    pub(crate) marker: std::marker::PhantomData<&'a Selector>,
    pub(crate) reach: f64,
    /// How many may be obstructed, in findings mode.
    pub(crate) ends: Option<Allowed>,
    pub(crate) sides: Option<Allowed>,
    /// The central stretch of a side an obstruction must overlap.
    pub(crate) side_zone: Option<f64>,
}

/// The states a bay must be in for its size bounds to apply.
#[derive(Default)]
pub(crate) struct Filters {
    orientations: Option<BTreeSet<State>>,
    ends: Option<BTreeSet<usize>>,
    sides: Option<BTreeSet<usize>>,
}

pub(crate) struct Config<'a> {
    pub(crate) width: (Option<f64>, Option<f64>),
    pub(crate) length: (Option<f64>, Option<f64>),
    pub(crate) height: (Option<f64>, Option<f64>),
    pub(crate) applies: Applies,
    pub(crate) orientation: Option<Orientation<'a>>,
    pub(crate) obstructions: Option<Obstructions<'a>>,
    pub(crate) filters: Filters,
}

impl Config<'_> {
    /// No check at all: what a measured value reusing the bay's own
    /// measurements needs.
    fn bare() -> Self {
        Config {
            width: (None, None),
            length: (None, None),
            height: (None, None),
            applies: Applies::Findings,
            orientation: None,
            obstructions: None,
            filters: Filters::default(),
        }
    }

    fn sized(&self) -> bool {
        [self.width, self.length]
            .iter()
            .any(|(low, high)| low.is_some() || high.is_some())
    }

    fn high(&self) -> bool {
        self.height.0.is_some() || self.height.1.is_some()
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

pub(crate) fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
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
        (Some(aisles), None) => {
            #[cfg(not(feature = "parity-reference"))]
            let _ = aisles;
            Some(Reference::Aisles {
                #[cfg(feature = "parity-reference")]
                aisles,
                reach: aisle_reach.unwrap_or(0.0),
                marker: std::marker::PhantomData,
            })
        }
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
            (Some(obstacles), Some(reach), Some(ends), Some(sides)) => {
                #[cfg(not(feature = "parity-reference"))]
                let _ = obstacles;
                Some(Obstructions {
                    #[cfg(feature = "parity-reference")]
                    obstacles,
                    marker: std::marker::PhantomData,
                    reach,
                    ends: Some(ends),
                    sides: Some(sides),
                    side_zone,
                })
            }
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
                (Some(obstacles), Some(reach)) if counted => {
                    #[cfg(not(feature = "parity-reference"))]
                    let _ = obstacles;
                    Some(Obstructions {
                        #[cfg(feature = "parity-reference")]
                        obstacles,
                        marker: std::marker::PhantomData,
                        reach,
                        ends: None,
                        sides: None,
                        side_zone,
                    })
                }
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

pub(crate) struct Services<'a> {
    pub(crate) rectangles: Option<&'a PlanSpanServiceHandle>,
    pub(crate) extents: Option<&'a VerticalExtentServiceHandle>,
    pub(crate) proximity: Option<&'a ProximityServiceHandle>,
}

impl<'a> Services<'a> {
    pub(crate) fn of(context: &RuleContext<'a>, config: &Config<'_>) -> Result<Self, Unavailable> {
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
pub(crate) struct Nearby {
    /// Objects the selector picks.
    pub(crate) matched: BTreeSet<ObjectId>,
    /// Objects near each bay, matched or undecided.
    pub(crate) near: BTreeMap<ObjectId, Vec<ObjectId>>,
    /// Matched or undecided objects whose extent cannot be read: they may
    /// stand near any bay.
    pub(crate) blind: BTreeSet<ObjectId>,
    /// Bays whose extent cannot be read.
    pub(crate) unbounded: BTreeMap<ObjectId, Unavailable>,
    /// Enclosing boxes, for every object with a readable extent.
    pub(crate) bounds: BTreeMap<ObjectId, ObjectBounds>,
}

impl Nearby {
    #[cfg(feature = "parity-reference")]
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
        Self::of(proximity, matched, &undecided, bays, reach)
    }

    /// The `matched` and `undecided` objects near each bay.
    pub(crate) fn of(
        proximity: &ProximityServiceHandle,
        matched: BTreeSet<ObjectId>,
        undecided: &BTreeSet<ObjectId>,
        bays: &[&Object],
        reach: f64,
    ) -> Result<Self, Unavailable> {
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
        for object in found.matched.iter().chain(undecided) {
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

    /// The same, keeping the extents only of the bays and the objects near
    /// one: all a bay's steps read of them.
    pub(crate) fn kept(mut self) -> Self {
        let near: BTreeSet<&ObjectId> = self.near.values().flatten().collect();
        let unbounded = &self.unbounded;
        let bays: BTreeSet<&ObjectId> = self.near.keys().collect();
        let kept: BTreeMap<ObjectId, ObjectBounds> = std::mem::take(&mut self.bounds)
            .into_iter()
            .filter(|(object, _)| {
                near.contains(object) || bays.contains(object) || unbounded.contains_key(object)
            })
            .collect();
        self.bounds = kept;
        self
    }

    fn near(&self, bay: &ObjectId) -> &[ObjectId] {
        self.near.get(bay).map_or(&[], Vec::as_slice)
    }
}

/// A finding (message, evidence, related objects), or `None` for a pass.
pub(crate) type Check = Result<Option<(String, Vec<Evidence>, Vec<ObjectId>)>, Unavailable>;

/// What was measured, its interval in metres and its evidence.
pub(crate) type Size = (String, (f64, f64), Vec<Evidence>);

/// A bay's size against inclusive bounds: what was measured (`width along
/// the bay's own axes`), its interval and evidence, or why it could not be.
pub(crate) struct Sized {
    pub(crate) measured: Result<Size, Unavailable>,
    pub(crate) bounds: (Option<f64>, Option<f64>),
}

/// Obstacles counted at a bay, from surely to at most, against how many
/// are allowed, with the words of a finding and of a doubt.
pub(crate) struct Counting {
    pub(crate) counted: (usize, usize),
    pub(crate) allowed: usize,
    pub(crate) found: String,
    pub(crate) open: String,
    pub(crate) related: Vec<ObjectId>,
    pub(crate) evidence: Vec<Evidence>,
}

/// One thing judged of a bay, in the capability's order: a search's own
/// answer, a size against its bounds, or a count against what is allowed.
pub(crate) enum Matter {
    Judged(Check),
    Size(Sized),
    Count(Counting),
}

/// How the filters stand for a bay in filter mode: whether the size bounds
/// apply to it, the states it may be in, why that is open, and the
/// evidence of the states.
pub(crate) struct Filtering {
    pub(crate) applies: Tri,
    pub(crate) states: String,
    pub(crate) why: String,
    pub(crate) evidence: Vec<Evidence>,
}

/// A count judged: a finding above what is allowed, open where it may be.
#[cfg(feature = "parity-reference")]
pub(crate) fn judge_count(counting: &Counting) -> Check {
    let (surely, most) = counting.counted;
    if surely > counting.allowed {
        Ok(Some((
            counting.found.clone(),
            counting.evidence.clone(),
            counting.related.clone(),
        )))
    } else if most > counting.allowed {
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            counting.open.clone(),
        ))
    } else {
        Ok(None)
    }
}

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
    inside: Option<Counting>,
    /// How many obstacles stand within the bay: surely, and at most (every
    /// obstacle that cannot be placed counted).
    within: (usize, usize),
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

pub(crate) struct Bay<'s, 'a> {
    pub(crate) config: &'s Config<'s>,
    pub(crate) services: &'s Services<'a>,
    pub(crate) object: &'s Object,
}

/// The four edges of a bay: along its long axis (ends) or across it
/// (sides), on the high or low side.
const EDGES: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

impl Bay<'_, '_> {
    fn id(&self) -> &ObjectId {
        &self.object.id
    }

    /// What is judged of the bay, in the capability's order, and how the
    /// filters stand for it in filter mode (they decide whether its sizes
    /// are judged).
    pub(crate) fn steps(
        &self,
        references: Option<&Nearby>,
        obstacles: Option<&Nearby>,
    ) -> (Vec<Matter>, Option<Filtering>) {
        let rectangle = self
            .services
            .rectangles
            .map(|service| rectangle(service, self.id()));
        let mut sizes = Vec::new();
        if self.config.sized() {
            sizes.extend(self.size(rectangle.as_ref()));
        }
        if self.config.high()
            && let Some(height) = self.height()
        {
            sizes.push(height);
        }
        let counted = match (&self.config.obstructions, obstacles) {
            (Some(obstructions), Some(obstacles)) => {
                Some(self.count(obstructions, obstacles, rectangle.as_ref()))
            }
            _ => None,
        };
        let mut steps = Vec::new();
        if let Some(Ok(counted)) = &counted
            && let Some(inside) = &counted.inside
        {
            steps.push(Matter::Count(Counting {
                counted: inside.counted,
                allowed: inside.allowed,
                found: inside.found.clone(),
                open: inside.open.clone(),
                related: inside.related.clone(),
                evidence: inside.evidence.clone(),
            }));
        }
        match self.config.applies {
            Applies::Findings => {
                steps.extend(sizes.into_iter().map(Matter::Size));
                if let (Some(orientation), Some(aisles), Some(wanted)) = (
                    &self.config.orientation,
                    references,
                    self.config.orientation.as_ref().and_then(|o| o.wanted),
                ) {
                    steps.push(Matter::Judged(self.orientation(
                        orientation,
                        wanted,
                        aisles,
                        rectangle.as_ref(),
                    )));
                }
                if let (Some(obstructions), Some(counted)) = (&self.config.obstructions, counted) {
                    match counted {
                        Ok(counted) => steps.extend(
                            counts(obstructions, &counted)
                                .into_iter()
                                .map(Matter::Count),
                        ),
                        Err(error) => steps.push(Matter::Judged(Err(error))),
                    }
                }
                (steps, None)
            }
            Applies::Filter => {
                let oriented = match (&self.config.orientation, references) {
                    (Some(orientation), Some(references)) => {
                        Some(self.oriented(orientation, references, rectangle.as_ref()))
                    }
                    _ => None,
                };
                let filtering = self.filtered(oriented.as_ref(), counted.as_ref());
                steps.extend(sizes.into_iter().map(Matter::Size));
                (steps, Some(filtering))
            }
        }
    }

    /// The size checks, applied to this bay only as far as its states are
    /// the filters'.
    fn filtered(
        &self,
        oriented: Option<&Oriented>,
        counted: Option<&Result<Counted, Unavailable>>,
    ) -> Filtering {
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
        Filtering {
            applies,
            states: said.join(", "),
            why: why.join("; "),
            evidence,
        }
    }

    fn size(&self, rectangle: Option<&Result<PlanRectangle, Unavailable>>) -> Vec<Sized> {
        let rectangle = match rectangle {
            Some(Ok(rectangle)) => rectangle,
            Some(Err(error)) => {
                return vec![Sized {
                    measured: Err(error.clone()),
                    bounds: (None, None),
                }];
            }
            None => return Vec::new(),
        };
        let sides = match rectangle.width_and_length() {
            Ok(sides) => sides,
            Err(reason) => {
                return vec![Sized {
                    measured: Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("its own axes are unknown: {reason}"),
                    )),
                    bounds: (None, None),
                }];
            }
        };
        [
            ("width", sides[0], self.config.width),
            ("length", sides[1], self.config.length),
        ]
        .into_iter()
        .filter(|(_, _, (low, high))| low.is_some() || high.is_some())
        .map(|(name, measured, bounds)| Sized {
            measured: Ok((
                format!("{name} along the bay's own axes"),
                measured,
                vec![rectangle.evidence().clone()],
            )),
            bounds,
        })
        .collect()
    }

    fn height(&self) -> Option<Sized> {
        let extents = self.services.extents?;
        let bounds = self.config.height;
        Some(Sized {
            measured: extents
                .measure_vertical_extent(self.id())
                .map_err(|error| extent_unavailable(&error))
                .map(|extent| {
                    (
                        "height".to_owned(),
                        extent.height_metres(),
                        vec![extent.evidence().clone()],
                    )
                }),
            bounds,
        })
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
        // Obstacles that cannot be placed: they may stand anywhere.
        let mut unplaced = blind;
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
                    unplaced += 1;
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
                    unplaced += 1;
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
        let within = (inside.len(), inside.len() + maybe_inside.len() + unplaced);
        let inside = if inside.is_empty() && maybe_inside.is_empty() && unknown.is_empty() {
            None
        } else {
            let mut reasons = unknown.clone();
            if !maybe_inside.is_empty() {
                reasons.push(format!("{} may stand within the bay", list(&maybe_inside)));
            }
            Some(Counting {
                counted: within,
                allowed: 0,
                found: format!(
                    "{} within the bay, past none of its edges",
                    names(&inside, "stands", "stand")
                ),
                open: format!(
                    "whether an obstacle stands within the bay is unknown: {}",
                    reasons.join("; ")
                ),
                related: inside,
                evidence: evidence.clone(),
            })
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
            within,
            ends: count(0),
            sides: count(2),
            unknown,
            evidence,
        })
    }
}

/// The obstruction counts of findings mode, against what is allowed.
fn counts(obstructions: &Obstructions<'_>, counted: &Counted) -> Vec<Counting> {
    let mut counts = Vec::new();
    for (label, count, allowed) in [
        ("ends", &counted.ends, obstructions.ends),
        ("sides", &counted.sides, obstructions.sides),
    ] {
        let Some(allowed) = allowed else { continue };
        let mut reasons = counted.unknown.clone();
        if !count.open.is_empty() {
            reasons.push(format!("{} may obstruct them", list(&count.open)));
        }
        counts.push(Counting {
            counted: (count.surely, count.most),
            allowed: allowed.0,
            found: format!(
                "{} of its {label} obstructed by {}, {} allowed",
                count.surely,
                list(&count.related),
                allowed.name()
            ),
            open: format!(
                "how many of its {label} are obstructed is unknown, {} allowed: {}",
                allowed.name(),
                reasons.join("; ")
            ),
            related: count.related.clone(),
            evidence: counted.evidence.clone(),
        });
    }
    counts
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
    let interval = centre_angle(own, theirs)?;
    Ok(ALIGNMENTS
        .into_iter()
        .map(|alignment| (State::of(alignment), alignment.holds(interval, tolerance)))
        .collect())
}

/// The acute angle, in degrees, between `own`'s long axis and the
/// direction from its centre to `theirs`' centre: about 90 for a bay
/// beside it, about 0 for one end to end with it. The interval holds the
/// exact angle: widened by how far either centre and `own`'s axis may be
/// off, and by the rounding of the arctangent.
///
/// # Errors
///
/// Why it cannot be told: `own` has no long axis, or the centres are too
/// close for a direction between them.
pub(crate) fn centre_angle(
    own: &PlanRectangle,
    theirs: &PlanRectangle,
) -> Result<(f64, f64), String> {
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
    Ok(((angle - slack).max(0.0), (angle + slack).min(90.0)))
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
#[cfg(feature = "parity-reference")]
pub(crate) fn bounded(
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
