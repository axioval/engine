//! `space-distance`: how far each space lies from its nearest destination
//! space, in a straight line, between bodies or walking.
//!
//! The search stays here: each applicable row's destinations, which surely
//! qualify and which might, and the nearest distance bounded from above by
//! the sure ones and from below by every possible one (`Search::answer`).
//! The capability runs as a template (`space_distance/template.rs`) judging
//! that distance against the row's bounds through the measured list
//! `distance_rows` (`space_distance/measured.rs`).

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::RowMeasures;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex, PoisonError};

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CentrePlacement, ColumnKind, CompiledRule, ConnectorRouting, MetricPoint,
    MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome, NearestTargetRequest,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanSpan, PlanSpanServiceHandle,
    ProximityProjection, ProximityRequest, ProximityServiceHandle, RuleCapability, RuleContext,
    TableColumn, VerticalExtentServiceHandle,
};
use axioval_ir::contract::{ParameterValue, Selector};
use axioval_ir::{Evidence, Object, ObjectId};

use crate::climbing::{self, Climbing};
use crate::plan_area::shown;
use crate::selection::{Selection, selector_matches};
use crate::space_access::{AccessDeclaration, AccessIndex, AccessType, Link, Partners};
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Traversal, Unavailable, invalid};

pub(crate) const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("from", ColumnKind::Selector),
    TableColumn::required("to", ColumnKind::Selector),
    TableColumn::optional("measure", ColumnKind::String),
    TableColumn::optional("same_storey", ColumnKind::Boolean),
    TableColumn::optional("direct_access", ColumnKind::Boolean),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
];

/// The capability's parameter descriptor.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![
        ParameterDescriptor::required("distances", ParameterType::Table(COLUMNS)),
        ParameterDescriptor::optional("storey_path", ParameterType::StringList),
        ParameterDescriptor::optional("storey_selector", ParameterType::Selector),
        ParameterDescriptor::optional("access_path", ParameterType::StringList),
        ParameterDescriptor::optional("door_selector", ParameterType::Selector),
        ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
        ParameterDescriptor::optional("space_selector", ParameterType::Selector),
        ParameterDescriptor::optional("walking_radius", ParameterType::Number),
        ParameterDescriptor::optional("walking_height", ParameterType::Number),
        ParameterDescriptor::optional("walking_step", ParameterType::Number),
        ParameterDescriptor::optional("walking_slope", ParameterType::Number),
    ];
    parameters.extend(climbing::descriptors());
    parameters
}

/// Checks the distance from each selected space to its nearest destination.
///
/// Every row of `distances` whose `from` selector picks the space applies to
/// it. Its destinations are the other spaces `to` picks, only those on the
/// same storey with `same_storey`, and only those it has direct access to
/// through a shared door or opening with `direct_access`. The nearest
/// destination must lie at most `maximum` and at least `minimum` metres
/// away; with a maximum, a space without any reachable destination is found.
///
/// `measure` `straight` (the default) is the plan distance between the
/// footprints' centres (the plan-span service). `closest` is the shortest
/// distance between the spaces' bodies in space (the proximity service's
/// `minimum_3d` distance), so two long rooms side by side are as close as
/// the wall between them is thick. `walking` is the shortest
/// route (the metric-routing service) between representative points: the
/// centre of each footprint, which must lie inside it, on the space's floor
/// (the bottom of its vertical extent). A space whose centre lies outside
/// its footprint, as for an L-shaped room, has no representative point and
/// is not evaluated rather than walked from elsewhere. `walking_radius`,
/// `walking_height` and `walking_step` state the body walked with;
/// `walking_slope` defaults to level. With `stair_selector`,
/// `ramp_selector` or `lift_selector`, walks may climb the selected
/// connectors to other storeys, a climb counting by `stair_length` and
/// `vertical_factor`.
///
/// Storeys are the nearest `storey_selector` objects `storey_path` climbs to
/// (as `same-container` climbs); direct access reads `access_path` among
/// `door_selector` and `opening_selector` objects (as `space-connection`
/// does).
///
/// Every length is an interval and a route may be blocked. The nearest
/// distance is bounded from above by the destinations that surely qualify
/// and from below by every one that might: a verdict stands only when
/// destinations whose qualification or distance is unknown cannot change
/// it. Walking, each bound is one nearest-destination query (the
/// metric-routing service's `nearest_target`) over all its destinations at
/// once; a destination without a representative point leaves the lower
/// bound at zero.
pub struct SpaceDistance;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for SpaceDistance {
    fn id(&self) -> &'static str {
        TEMPLATE.id
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Measure {
    Straight,
    Closest,
    Walking,
}

impl Measure {
    pub(crate) fn parse(row: &str, stated: Option<&str>) -> Result<Self, Unavailable> {
        match stated.unwrap_or("straight") {
            "straight" => Ok(Self::Straight),
            "closest" => Ok(Self::Closest),
            "walking" => Ok(Self::Walking),
            other => Err(invalid(format!(
                "{row}: measure `{other}` is unsupported (straight, closest, walking)"
            ))),
        }
    }

    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Straight => "in a straight line between centres",
            Self::Closest => "between the closest points of the bodies",
            Self::Walking => "walking",
        }
    }
}

/// One row of `distances`.
#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) name: String,
    from: Selector,
    to: Selector,
    measure: Measure,
    same_storey: bool,
    direct_access: bool,
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
}

/// What a rule declares, read as the capability read it: the rows, then
/// the storeys, the access, the walking profile and the connectors.
pub(crate) struct Declared<'a> {
    pub(crate) rows: Vec<Row>,
    pub(crate) storeys: Option<Traversal>,
    pub(crate) access: Option<AccessDeclaration<'a>>,
    pub(crate) profile: Option<MobilityProfile>,
    pub(crate) climbing: Option<Climbing<'a>>,
}

/// The rule's declaration, in the capability's order and words.
///
/// # Errors
///
/// An invalid declaration.
pub(crate) fn declaration(rule: &CompiledRule) -> Result<Declared<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let table = parameters
        .table("distances")?
        .ok_or_else(|| invalid("parameter `distances` is required"))?;
    let mut rows = Vec::new();
    for (index, row) in table.into_iter().enumerate() {
        let name = match row.text("label")? {
            Some(label) => format!("row {index} ({label})"),
            None => format!("row {index}"),
        };
        let measure = Measure::parse(&name, row.text("measure")?)?;
        let bound = |column: &str| -> Result<Option<f64>, Unavailable> {
            match row.number(column)? {
                Some(value) if value < 0.0 => {
                    Err(invalid(format!("{name}: `{column}` must not be negative")))
                }
                other => Ok(other),
            }
        };
        let (minimum, maximum) = (bound("minimum")?, bound("maximum")?);
        match (minimum, maximum) {
            (None, None) => {
                return Err(invalid(format!("{name} states no `minimum` or `maximum`")));
            }
            (Some(minimum), Some(maximum)) if minimum > maximum => {
                return Err(invalid(format!("{name}: `minimum` exceeds `maximum`")));
            }
            _ => {}
        }
        rows.push(Row {
            from: row
                .selector("from")?
                .ok_or_else(|| invalid(format!("{name} has no `from`")))?
                .clone(),
            to: row
                .selector("to")?
                .ok_or_else(|| invalid(format!("{name} has no `to`")))?
                .clone(),
            measure,
            same_storey: row.boolean("same_storey")?.unwrap_or(false),
            direct_access: row.boolean("direct_access")?.unwrap_or(false),
            minimum,
            maximum,
            name,
        });
    }
    let storeys = match (
        parameters.strings("storey_path")?,
        parameters.selector("storey_selector")?,
    ) {
        (Some(path), Some(_)) => Some(Traversal::path(path)?),
        (None, None) => None,
        _ => return Err(invalid("`storey_path` and `storey_selector` go together")),
    };
    if storeys.is_none() && rows.iter().any(|row| row.same_storey) {
        return Err(invalid(
            "`same_storey` needs `storey_path` and `storey_selector`",
        ));
    }
    let access = AccessDeclaration::parse(&parameters)?;
    if access.is_none() && rows.iter().any(|row| row.direct_access) {
        return Err(invalid("`direct_access` needs `access_path`"));
    }
    let walking = [
        "walking_radius",
        "walking_height",
        "walking_step",
        "walking_slope",
    ]
    .map(|name| parameters.number(name));
    let [radius, height, step, slope] = walking;
    let profile = match (radius?, height?, step?) {
        (Some(radius), Some(height), Some(step)) => Some(
            MobilityProfile::try_new(radius, height, step, slope?.unwrap_or(0.0))
                .map_err(|error| invalid(error.to_string()))?,
        ),
        (None, None, None) if slope?.is_none() => None,
        _ => {
            return Err(invalid(
                "`walking_radius`, `walking_height` and `walking_step` go together",
            ));
        }
    };
    if profile.is_none() && rows.iter().any(|row| row.measure == Measure::Walking) {
        return Err(invalid(
            "a walking row needs `walking_radius`, `walking_height` and `walking_step`",
        ));
    }
    Ok(Declared {
        rows,
        storeys,
        access,
        profile,
        climbing: Climbing::parse(&parameters)?,
    })
}

/// Checks the rule parameters `distance_rows` names, as the rule states
/// them: the declaration the capability refused, in its order and words.
///
/// # Errors
///
/// An invalid declaration.
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(stated.clone());
    declaration(&rule).map(|_| ())
}

/// A measured distance between two spaces.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Distance {
    /// Between `lower` and `upper` metres.
    Between(f64, f64, Evidence),
}

impl Distance {
    pub(crate) fn lower(&self) -> f64 {
        let Self::Between(lower, _, _) = self;
        *lower
    }

    pub(crate) fn upper(&self) -> f64 {
        let Self::Between(_, upper, _) = self;
        *upper
    }

    pub(crate) fn evidence(&self) -> &Evidence {
        let Self::Between(_, _, evidence) = self;
        evidence
    }
}

pub(crate) fn missing(service: &str) -> Unavailable {
    (
        NotEvaluatedReason::MissingService,
        format!("{service} service is not registered"),
    )
}

pub(crate) fn incomplete(message: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message)
}

/// Measures one pair of spaces between centres or between the closest
/// points of their bodies.
pub(crate) fn measure_pair(
    context: &RuleContext<'_>,
    measure: Measure,
    first: &ObjectId,
    second: &ObjectId,
) -> Result<Distance, Unavailable> {
    let services = context.services;
    match measure {
        Measure::Straight => {
            let spans = services
                .get::<PlanSpanServiceHandle>()
                .ok_or_else(|| missing("plan-span"))?;
            let length = spans
                .measure_span(first, second, PlanSpan::Centres)
                .map_err(|error| incomplete(format!("{first} to {second}: {error}")))?;
            Ok(Distance::Between(
                length.lower_metres(),
                length.upper_metres(),
                length.evidence().clone(),
            ))
        }
        Measure::Closest => {
            let proximity = services
                .get::<ProximityServiceHandle>()
                .ok_or_else(|| missing("proximity"))?;
            let distance = ProximityRequest::projected(
                first.clone(),
                second.clone(),
                ProximityProjection::Minimum3d,
            )
            .and_then(|request| proximity.measure_distance(&request))
            .map_err(|error| incomplete(format!("{first} to {second}: {error}")))?;
            let (lower, upper) = distance.interval_metres();
            if !(lower.is_finite() && upper.is_finite()) {
                return Err(incomplete(format!(
                    "{first} to {second}: the bodies have no closest distance"
                )));
            }
            Ok(Distance::Between(lower, upper, distance.evidence().clone()))
        }
        Measure::Walking => unreachable!("walking is measured by nearest-target queries"),
    }
}

/// An object's representative point for walking: the centre of its
/// footprint, which must lie inside it, on its floor (the bottom of its
/// vertical extent). Shared with `escape-route`.
pub(crate) fn representative_point(
    context: &RuleContext<'_>,
    object: &ObjectId,
) -> Result<(MetricPoint, Vec<Evidence>), Unavailable> {
    let services = context.services;
    let spans = services
        .get::<PlanSpanServiceHandle>()
        .ok_or_else(|| missing("plan-span"))?;
    let extents = services
        .get::<VerticalExtentServiceHandle>()
        .ok_or_else(|| missing("vertical-extent"))?;
    let centre = spans
        .measure_centre(object)
        .map_err(|error| incomplete(format!("the centre of {object}: {error}")))?;
    if !centre.is_exact() {
        return Err(incomplete(format!(
            "the centre of {object} is known only within {} m, so no route starts or ends \
             there",
            shown(centre.radius_metres(), centre.radius_metres())
        )));
    }
    match centre.placement() {
        CentrePlacement::Inside => {}
        CentrePlacement::Outside => {
            return Err(incomplete(format!(
                "the centre of {object} lies outside its footprint, so it has no \
                 representative point to walk from"
            )));
        }
        CentrePlacement::Undecided => {
            return Err(incomplete(format!(
                "the centre of {object} lies on its footprint's boundary"
            )));
        }
    }
    let extent = extents
        .measure_vertical_extent(object)
        .map_err(|error| incomplete(format!("the floor of {object}: {error}")))?;
    let floor = extent.bottom();
    if !floor.is_exact() {
        return Err(incomplete(format!(
            "the floor of {object} lies {} m high, not at one elevation",
            shown(floor.lower_metres(), floor.upper_metres())
        )));
    }
    let [x, y] = centre.point();
    let point = MetricPoint::try_new(object.clone(), [x, y, floor.lower_metres()])
        .map_err(|error| incomplete(format!("the centre of {object}: {error}")))?;
    Ok((
        point,
        vec![centre.evidence().clone(), extent.evidence().clone()],
    ))
}

/// A destination that surely qualifies, or might.
struct Candidate {
    id: ObjectId,
    /// `None` when it surely qualifies, else why it might not.
    doubt: Option<String>,
    evidence: Vec<Evidence>,
}

/// The nearest distance, bounded by the destinations that surely qualify
/// from above and by every one that might from below.
struct Nearest {
    most: f64,
    least: f64,
    /// The sure destination bounding `most` from above.
    sure: Option<(ObjectId, Distance, Vec<Evidence>)>,
    /// A possible destination at the least lower bound.
    closest: Option<(ObjectId, Distance, Vec<Evidence>)>,
    /// Evidence that destinations cannot be reached.
    blocked: Vec<Evidence>,
    doubts: Vec<String>,
    reason: NotEvaluatedReason,
}

impl Nearest {
    fn new() -> Self {
        Self {
            most: f64::INFINITY,
            least: f64::INFINITY,
            sure: None,
            closest: None,
            blocked: Vec::new(),
            doubts: Vec::new(),
            reason: NotEvaluatedReason::IncompleteEvidence,
        }
    }

    fn doubt(&mut self, (why, message): Unavailable) {
        if why == NotEvaluatedReason::MissingService {
            self.reason = why;
        }
        self.doubts.push(message);
    }
}

/// Nearest storeys, and the climb's evidence.
type Climb = Result<(BTreeSet<ObjectId>, Vec<Evidence>), Unavailable>;

/// What a nearest-destination query found.
enum Walked {
    Reached(usize, Distance),
    Unreachable(Evidence),
}

/// What a run of a rule reads once for all its spaces: its rows, storeys,
/// access, walking profile and connectors, and what it measured so far.
pub(crate) struct Search {
    rows: Vec<Row>,
    /// The climb, the storeys, and whether the storey selection decided
    /// every object.
    storeys: Option<(Traversal, BTreeSet<ObjectId>, bool)>,
    index: Option<AccessIndex>,
    profile: Option<MobilityProfile>,
    /// The connectors walks may climb, when the rule selects any.
    connectors: Option<Result<ConnectorRouting, Unavailable>>,
    caches: Mutex<Caches>,
}

/// What a search measured, kept across its spaces.
#[derive(Default)]
struct Caches {
    targets: BTreeMap<(usize, ObjectId), Selection>,
    climbed: BTreeMap<ObjectId, Climb>,
    partners: BTreeMap<ObjectId, Partners>,
    points: BTreeMap<ObjectId, Result<(MetricPoint, Vec<Evidence>), Unavailable>>,
    lengths: BTreeMap<(Measure, ObjectId, ObjectId), Result<Distance, Unavailable>>,
}

/// One finding a row's bounds may lead to: its words, the objects it
/// relates and the evidence it cites.
pub(crate) struct Finding {
    pub(crate) words: String,
    pub(crate) related: Vec<ObjectId>,
    pub(crate) evidence: Vec<Evidence>,
}

/// What the search found for one row of one space: the nearest distance
/// between `least` and `most` metres (either infinite where no destination
/// bounds it), what lying beyond the maximum or within the minimum is
/// worded as, and why the distance is not known better.
pub(crate) struct Measured {
    pub(crate) least: f64,
    pub(crate) most: f64,
    /// Lying beyond the row's maximum, where it states one.
    pub(crate) above: Option<Finding>,
    /// Lying within the row's minimum, where a sure destination bounds the
    /// distance.
    pub(crate) below: Option<Finding>,
    /// The words and reason of a distance the bounds leave undecided.
    pub(crate) open: Unavailable,
}

/// One applicable row of one space, and what the search found for it.
pub(crate) struct Answer {
    pub(crate) row: Row,
    pub(crate) measured: Result<Measured, Unavailable>,
}

impl Search {
    /// The search a rule declares, its selections those measured values'
    /// arguments bound.
    pub(crate) fn new(
        context: &RuleContext<'_>,
        declared: Declared<'_>,
        storeys: Option<&axioval_ir::measured::MeasuredSelection>,
    ) -> Self {
        let storeys = declared.storeys.map(|climb| {
            let (found, decided) = match storeys {
                Some(selection) => (selection.matched.clone(), selection.undecided.is_empty()),
                None => (BTreeSet::new(), false),
            };
            (climb, found, decided)
        });
        Self {
            index: declared.access.as_ref().map(|access| access.index(context)),
            connectors: declared
                .climbing
                .as_ref()
                .map(|climbing| climbing.routing(context)),
            rows: declared.rows,
            storeys,
            profile: declared.profile,
            caches: Mutex::new(Caches::default()),
        }
    }

    /// Every row that applies to `space`, each with what the search found.
    ///
    /// # Errors
    ///
    /// Whether a row's `from` picks the space is undecided.
    pub(crate) fn answer(
        &self,
        context: &RuleContext<'_>,
        space: &Object,
    ) -> Result<Vec<Answer>, Unavailable> {
        let matched = match_rows(&self.rows, RowSelection::All, |row| match selector_matches(
            context,
            &row.from,
            space,
            &mut Vec::new(),
        ) {
            Selection::Match => RowTest::Match(0),
            Selection::NoMatch => RowTest::NoMatch,
            Selection::NotEvaluated(..) => RowTest::Undecided,
        });
        let Matched::Rows(applicable) = matched else {
            return Err(incomplete(
                "space-distance: whether a row's `from` picks this space is undecided".into(),
            ));
        };
        let mut caches = self.caches.lock().unwrap_or_else(PoisonError::into_inner);
        let mut judge = Judge {
            context,
            search: self,
            caches: &mut caches,
        };
        Ok(applicable
            .into_iter()
            .map(|(index, row)| Answer {
                row: row.clone(),
                measured: judge.row(space, index, row),
            })
            .collect())
    }
}

struct Judge<'s, 'c> {
    context: &'s RuleContext<'c>,
    search: &'s Search,
    caches: &'s mut Caches,
}

impl Judge<'_, '_> {
    fn storey(&mut self, space: &ObjectId) -> Climb {
        let Some((climb, storeys, decided)) = &self.search.storeys else {
            return Err(invalid("`same_storey` needs `storey_path`"));
        };
        if !decided {
            return Err(incomplete(
                "the storey selector was not evaluated conclusively".into(),
            ));
        }
        let context = self.context;
        self.caches
            .climbed
            .entry(space.clone())
            .or_insert_with(|| climb.nearest_containers(context, space, storeys))
            .clone()
    }

    fn target(&mut self, index: usize, to: &Selector, object: &Object) -> Selection {
        let context = self.context;
        self.caches
            .targets
            .entry((index, object.id.clone()))
            .or_insert_with(|| selector_matches(context, to, object, &mut Vec::new()))
            .clone()
    }

    /// The candidates `row` names for `space`.
    fn candidates(
        &mut self,
        space: &Object,
        index: usize,
        row: &Row,
    ) -> Result<Vec<Candidate>, Unavailable> {
        let context = self.context;
        let mine = if row.same_storey {
            let (mine, evidence) = self.storey(&space.id)?;
            if mine.is_empty() {
                return Err(incomplete(format!(
                    "{} is on no storey, so no destination shares it",
                    space.id
                )));
            }
            Some((mine, evidence))
        } else {
            None
        };
        if row.direct_access && !self.caches.partners.contains_key(&space.id) {
            let index = self
                .search
                .index
                .as_ref()
                .ok_or_else(|| invalid("`direct_access` needs `access_path`"))?;
            self.caches
                .partners
                .insert(space.id.clone(), index.partners(&space.id, AccessType::Any));
        }
        let mut candidates = Vec::new();
        for object in context.project.objects() {
            if object.id == space.id {
                continue;
            }
            let mut doubts = Vec::new();
            let mut evidence = Vec::new();
            match self.target(index, &row.to, object) {
                Selection::NoMatch => continue,
                Selection::Match => {}
                Selection::NotEvaluated(_, why) => {
                    doubts.push(format!(
                        "whether `to` picks {} is undecided: {why}",
                        object.id
                    ));
                }
            }
            if let Some((mine, cited)) = &mine {
                match self.storey(&object.id) {
                    Ok((theirs, _)) if &theirs != mine => continue,
                    Ok((_, theirs)) => {
                        evidence.extend(cited.iter().cloned());
                        evidence.extend(theirs);
                    }
                    Err((_, why)) => {
                        doubts.push(format!("the storey of {} is unknown: {why}", object.id));
                    }
                }
            }
            if row.direct_access {
                match self.caches.partners[&space.id].with(&object.id) {
                    Ok(None) => continue,
                    Ok(Some(Link::Sure {
                        evidence: cited, ..
                    })) => evidence.extend(cited),
                    Ok(Some(Link::Maybe(why))) | Err(why) => {
                        doubts.push(format!(
                            "whether {} has direct access to {} is unknown: {why}",
                            space.id, object.id
                        ));
                    }
                }
            }
            candidates.push(Candidate {
                id: object.id.clone(),
                doubt: (!doubts.is_empty()).then(|| doubts.join("; ")),
                evidence,
            });
        }
        Ok(candidates)
    }

    /// The distance between two spaces measured pair by pair: between
    /// centres or between the closest points of the bodies.
    fn pairwise(
        &mut self,
        measure: Measure,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Result<Distance, Unavailable> {
        // Neither distance has a direction.
        let key = if to < from {
            (measure, to.clone(), from.clone())
        } else {
            (measure, from.clone(), to.clone())
        };
        if let Some(known) = self.caches.lengths.get(&key) {
            return known.clone();
        }
        let measured = measure_pair(self.context, measure, &key.1, &key.2);
        self.caches.lengths.insert(key, measured.clone());
        measured
    }

    /// A space's representative point, cached.
    fn point(&mut self, space: &ObjectId) -> Result<(MetricPoint, Vec<Evidence>), Unavailable> {
        if let Some(known) = self.caches.points.get(space) {
            return known.clone();
        }
        let located = representative_point(self.context, space);
        self.caches.points.insert(space.clone(), located.clone());
        located
    }

    /// Bounds the nearest distance in a straight line or between bodies,
    /// pair by pair.
    fn nearest_pairwise(
        &mut self,
        measure: Measure,
        space: &ObjectId,
        candidates: &[Candidate],
    ) -> Nearest {
        let mut nearest = Nearest::new();
        for candidate in candidates {
            if let Some(doubt) = &candidate.doubt {
                nearest.doubts.push(doubt.clone());
            }
            match self.pairwise(measure, space, &candidate.id) {
                Ok(distance) => {
                    if candidate.doubt.is_none() && distance.upper() < nearest.most {
                        nearest.most = distance.upper();
                        nearest.sure = Some((
                            candidate.id.clone(),
                            distance.clone(),
                            candidate.evidence.clone(),
                        ));
                    }
                    if distance.lower() < nearest.least {
                        nearest.least = distance.lower();
                        nearest.closest = Some((
                            candidate.id.clone(),
                            distance.clone(),
                            candidate.evidence.clone(),
                        ));
                    }
                    nearest.blocked.push(distance.evidence().clone());
                }
                // An unmeasured destination might be at any distance.
                Err(unavailable) => {
                    nearest.least = 0.0;
                    nearest.doubt(unavailable);
                }
            }
        }
        nearest
    }

    /// Bounds the nearest distance walking with two nearest-destination
    /// queries: the destinations that surely qualify bound it from above,
    /// every one that might from below.
    #[allow(clippy::too_many_lines)]
    fn nearest_walking(&mut self, space: &ObjectId, candidates: &[Candidate]) -> Nearest {
        let mut nearest = Nearest::new();
        for candidate in candidates {
            if let Some(doubt) = &candidate.doubt {
                nearest.doubts.push(doubt.clone());
            }
        }
        if candidates.is_empty() {
            return nearest;
        }
        let Some(routes) = self.context.services.get::<MetricRoutingServiceHandle>() else {
            nearest.least = 0.0;
            nearest.doubt(missing("metric-routing"));
            return nearest;
        };
        let Some(profile) = self.search.profile else {
            nearest.least = 0.0;
            nearest.doubt(invalid("a walking row needs a walking profile"));
            return nearest;
        };
        let origin = match self.point(space) {
            Ok((origin, _)) => origin,
            Err(unavailable) => {
                nearest.least = 0.0;
                nearest.doubt(unavailable);
                return nearest;
            }
        };
        let mut sure: Vec<(&Candidate, MetricPoint)> = Vec::new();
        let mut all: Vec<(&Candidate, MetricPoint)> = Vec::new();
        let mut placed = true;
        for candidate in candidates {
            match self.point(&candidate.id) {
                Ok((point, _)) => {
                    if candidate.doubt.is_none() {
                        sure.push((candidate, point.clone()));
                    }
                    all.push((candidate, point));
                }
                Err(unavailable) => {
                    placed = false;
                    nearest.doubt(unavailable);
                }
            }
        }
        let routing = match &self.search.connectors {
            None => None,
            Some(Ok(routing)) => Some(routing.clone()),
            Some(Err(why)) => {
                nearest.least = 0.0;
                nearest.doubt(why.clone());
                return nearest;
            }
        };
        let walk = |targets: &[(&Candidate, MetricPoint)]| -> Result<Walked, Unavailable> {
            let request = NearestTargetRequest::try_new(
                origin.clone(),
                targets.iter().map(|(_, point)| point.clone()).collect(),
                profile,
            )
            .map_err(|error| incomplete(error.to_string()))?;
            let request = match &routing {
                Some(routing) => request.with_connectors(routing.clone()),
                None => request,
            };
            match routes.nearest_target(&request) {
                Ok(NearestTargetOutcome::Reached(reached)) => {
                    let length = reached.shortest_distance();
                    Ok(Walked::Reached(
                        reached.target(),
                        Distance::Between(
                            length.lower_metres(),
                            length.upper_metres(),
                            reached.evidence().clone(),
                        ),
                    ))
                }
                Ok(NearestTargetOutcome::Unreachable(unreachable)) => Ok(Walked::Unreachable(
                    unreachable.completeness().evidence().clone(),
                )),
                Err(error) => Err(incomplete(format!("walking from {space}: {error}"))),
            }
        };
        let upper = if sure.is_empty() {
            None
        } else {
            Some(walk(&sure))
        };
        match &upper {
            Some(Ok(Walked::Reached(target, distance))) => {
                let candidate = sure[*target].0;
                nearest.most = distance.upper();
                nearest.sure = Some((
                    candidate.id.clone(),
                    distance.clone(),
                    candidate.evidence.clone(),
                ));
            }
            Some(Ok(Walked::Unreachable(evidence))) => nearest.blocked.push(evidence.clone()),
            Some(Err(unavailable)) => nearest.doubt(unavailable.clone()),
            None => {}
        }
        if !placed {
            // An unplaced destination might be at any distance.
            nearest.least = 0.0;
            return nearest;
        }
        let lower = if sure.len() == all.len() {
            upper.expect("every destination is sure and placed, and there is one")
        } else {
            walk(&all)
        };
        match lower {
            Ok(Walked::Reached(target, distance)) => {
                let candidate = all[target].0;
                nearest.least = distance.lower();
                nearest.closest =
                    Some((candidate.id.clone(), distance, candidate.evidence.clone()));
            }
            Ok(Walked::Unreachable(evidence)) => {
                if !nearest.blocked.contains(&evidence) {
                    nearest.blocked.push(evidence);
                }
            }
            Err(unavailable) => {
                nearest.least = 0.0;
                if !nearest.doubts.contains(&unavailable.1) {
                    nearest.doubt(unavailable);
                }
            }
        }
        nearest
    }

    /// What the search finds for one row of one space.
    fn row(&mut self, space: &Object, index: usize, row: &Row) -> Result<Measured, Unavailable> {
        let candidates = self
            .candidates(space, index, row)
            .map_err(|(reason, message)| {
                (reason, format!("space-distance {}: {message}", row.name))
            })?;
        let nearest = match row.measure {
            Measure::Straight | Measure::Closest => {
                self.nearest_pairwise(row.measure, &space.id, &candidates)
            }
            Measure::Walking => self.nearest_walking(&space.id, &candidates),
        };
        let above = row
            .maximum
            .map(|maximum| too_far(row, &candidates, &nearest, maximum));
        let below = row.minimum.and_then(|minimum| {
            let (id, distance, cited) = nearest.sure.as_ref()?;
            let mut evidence = cited.clone();
            evidence.push(distance.evidence().clone());
            Some(Finding {
                words: format!(
                    "{id} is {} m away {}; {} requires at least {minimum} m",
                    shown(distance.lower(), distance.upper()),
                    row.measure.describe(),
                    row.name
                ),
                related: vec![id.clone()],
                evidence,
            })
        });
        Ok(Measured {
            least: nearest.least,
            most: nearest.most,
            above,
            below,
            open: undecided(row, &nearest),
        })
    }
}

/// The words of a distance the row's bounds leave undecided, and its
/// reason.
fn undecided(row: &Row, nearest: &Nearest) -> Unavailable {
    let (most, least) = (nearest.most, nearest.least);
    let bounds = match (row.minimum, row.maximum) {
        (Some(minimum), Some(maximum)) => format!("between {minimum} and {maximum} m"),
        (Some(minimum), None) => format!("at least {minimum} m"),
        (None, Some(maximum)) => format!("at most {maximum} m"),
        (None, None) => unreachable!("a row states a bound"),
    };
    let shown_most = if most.is_finite() {
        format!("at most {} m", shown(most, most))
    } else {
        "unknown".to_owned()
    };
    let mut doubts = nearest.doubts.clone();
    if doubts.is_empty() {
        doubts.push(format!(
            "it is known only to lie {} m away",
            shown(least, most)
        ));
    }
    doubts.sort();
    doubts.dedup();
    (
        nearest.reason.clone(),
        format!(
            "space-distance {}: the nearest destination is {shown_most} {}, {bounds} required, \
             and {}",
            row.name,
            row.measure.describe(),
            doubts.join("; ")
        ),
    )
}

/// The finding for a space whose every possible destination lies beyond
/// `maximum`, or which has none.
fn too_far(row: &Row, candidates: &[Candidate], nearest: &Nearest, maximum: f64) -> Finding {
    let how = row.measure.describe();
    let mut filters = Vec::new();
    if row.same_storey {
        filters.push("on its storey");
    }
    if row.direct_access {
        filters.push("with direct access");
    }
    let filters = if filters.is_empty() {
        String::new()
    } else {
        format!(" {}", filters.join(" "))
    };
    let mut evidence = Vec::new();
    let (words, related) = if candidates.is_empty() {
        (
            format!(
                "has no destination{filters}; {} requires one within {maximum} m {how}",
                row.name
            ),
            Vec::new(),
        )
    } else if let Some((id, distance, cited)) = &nearest.closest {
        evidence.extend(cited.iter().cloned());
        evidence.push(distance.evidence().clone());
        (
            format!(
                "the nearest destination{filters}, {id}, is {} m away {how}; {} allows at \
                 most {maximum} m",
                shown(distance.lower(), distance.upper()),
                row.name
            ),
            vec![id.clone()],
        )
    } else {
        evidence.extend(nearest.blocked.iter().cloned());
        (
            format!(
                "reaches none of its {} destination(s){filters} {how}; {} requires one \
                 within {maximum} m",
                candidates.len(),
                row.name
            ),
            candidates
                .iter()
                .map(|candidate| candidate.id.clone())
                .collect(),
        )
    };
    Finding {
        words,
        related,
        evidence,
    }
}
