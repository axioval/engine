//! `space-distance`: how far each space lies from its nearest destination
//! space, in a straight line or walking.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CentrePlacement, ColumnKind, CompiledRule, Deviation, MetricPoint,
    MetricRoutingServiceHandle, MobilityProfile, NearestTargetOutcome, NearestTargetRequest,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanSpan, PlanSpanServiceHandle,
    ProximityProjection, ProximityRequest, ProximityServiceHandle, RuleCapability, RuleContext,
    TableColumn, VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId};

use crate::plan_area::shown;
use crate::selection::{Selection, select_objects, selector_matches};
use crate::space_access::{AccessDeclaration, AccessIndex, AccessType, Link, Partners};
use crate::support::table::{Matched, RowSelection, RowTest, match_rows};
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

const COLUMNS: &[TableColumn] = &[
    TableColumn::optional("label", ColumnKind::String),
    TableColumn::required("from", ColumnKind::Selector),
    TableColumn::required("to", ColumnKind::Selector),
    TableColumn::optional("measure", ColumnKind::String),
    TableColumn::optional("same_storey", ColumnKind::Boolean),
    TableColumn::optional("direct_access", ColumnKind::Boolean),
    TableColumn::optional("minimum", ColumnKind::Number),
    TableColumn::optional("maximum", ColumnKind::Number),
];

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
/// `walking_slope` defaults to level.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Measure {
    Straight,
    Closest,
    Walking,
}

impl Measure {
    fn parse(row: &str, stated: Option<&str>) -> Result<Self, Unavailable> {
        match stated.unwrap_or("straight") {
            "straight" => Ok(Self::Straight),
            "closest" => Ok(Self::Closest),
            "walking" => Ok(Self::Walking),
            other => Err(invalid(format!(
                "{row}: measure `{other}` is unsupported (straight, closest, walking)"
            ))),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Straight => "in a straight line between centres",
            Self::Closest => "between the closest points of the bodies",
            Self::Walking => "walking",
        }
    }
}

struct Row<'a> {
    name: String,
    from: &'a Selector,
    to: &'a Selector,
    measure: Measure,
    same_storey: bool,
    direct_access: bool,
    minimum: Option<f64>,
    maximum: Option<f64>,
}

struct Declaration<'a> {
    rows: Vec<Row<'a>>,
    storeys: Option<(Traversal<'a>, &'a Selector)>,
    access: Option<AccessDeclaration<'a>>,
    profile: Option<MobilityProfile>,
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
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
                .ok_or_else(|| invalid(format!("{name} has no `from`")))?,
            to: row
                .selector("to")?
                .ok_or_else(|| invalid(format!("{name} has no `to`")))?,
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
        (Some(path), Some(selector)) => Some((Traversal::path(path)?, selector)),
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
    Ok(Declaration {
        rows,
        storeys,
        access,
        profile,
    })
}

impl RuleCapability for SpaceDistance {
    fn id(&self) -> &'static str {
        "axioval:capability.space-distance"
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
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
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("space-distance: {message}"),
                );
            }
        };
        let storeys = declared.storeys.as_ref().map(|(climb, selector)| {
            let (found, outcomes) = select_objects(context, selector);
            let decided = outcomes.not_evaluated_outcomes().is_empty();
            (
                climb,
                found.into_iter().map(|object| object.id.clone()).collect(),
                decided,
            )
        });
        let index = declared.access.as_ref().map(|access| access.index(context));
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        let mut judge = Judge {
            context,
            rule,
            profile: declared.profile,
            storeys,
            index: index.as_ref(),
            targets: BTreeMap::new(),
            climbed: BTreeMap::new(),
            partners: BTreeMap::new(),
            points: BTreeMap::new(),
            lengths: BTreeMap::new(),
        };
        for space in spaces {
            let matched = match_rows(
                &declared.rows,
                RowSelection::All,
                |row| match selector_matches(context, row.from, space, &mut Vec::new()) {
                    Selection::Match => RowTest::Match(0),
                    Selection::NoMatch => RowTest::NoMatch,
                    Selection::NotEvaluated(..) => RowTest::Undecided,
                },
            );
            let Matched::Rows(applicable) = matched else {
                evaluation.push_object_not_evaluated(
                    space.id.clone(),
                    NotEvaluatedReason::IncompleteEvidence,
                    "space-distance: whether a row's `from` picks this space is undecided",
                );
                continue;
            };
            for (index, row) in applicable {
                match judge.row(space, index, row) {
                    Ok(Some((found, deviation))) => {
                        evaluation.push_graded_finding(found, deviation);
                    }
                    Ok(None) => {}
                    Err((reason, message)) => evaluation.push_object_not_evaluated(
                        space.id.clone(),
                        reason,
                        format!("space-distance {}: {message}", row.name),
                    ),
                }
            }
        }
        evaluation
    }
}

/// A measured distance between two spaces.
#[derive(Clone, Debug, PartialEq)]
enum Distance {
    /// Between `lower` and `upper` metres.
    Between(f64, f64, Evidence),
}

impl Distance {
    fn lower(&self) -> f64 {
        let Self::Between(lower, _, _) = self;
        *lower
    }

    fn upper(&self) -> f64 {
        let Self::Between(_, upper, _) = self;
        *upper
    }

    fn evidence(&self) -> &Evidence {
        let Self::Between(_, _, evidence) = self;
        evidence
    }
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

struct Judge<'r, 'c> {
    context: &'r RuleContext<'c>,
    rule: &'r CompiledRule,
    profile: Option<MobilityProfile>,
    /// The climb, the storeys, and whether the storey selection decided
    /// every object.
    storeys: Option<(&'r Traversal<'r>, BTreeSet<ObjectId>, bool)>,
    index: Option<&'r AccessIndex>,
    targets: BTreeMap<(usize, ObjectId), Selection>,
    climbed: BTreeMap<ObjectId, Climb>,
    partners: BTreeMap<ObjectId, Partners>,
    points: BTreeMap<ObjectId, Result<(MetricPoint, Vec<Evidence>), Unavailable>>,
    lengths: BTreeMap<(Measure, ObjectId, ObjectId), Result<Distance, Unavailable>>,
}

fn missing(service: &str) -> Unavailable {
    (
        NotEvaluatedReason::MissingService,
        format!("{service} service is not registered"),
    )
}

fn incomplete(message: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message)
}

/// Measures one pair of spaces between centres or between the closest
/// points of their bodies.
fn measure_pair(
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

impl Judge<'_, '_> {
    fn storey(&mut self, space: &ObjectId) -> Climb {
        let Some((climb, storeys, decided)) = &self.storeys else {
            return Err(invalid("`same_storey` needs `storey_path`"));
        };
        if !decided {
            return Err(incomplete(
                "the storey selector was not evaluated conclusively".into(),
            ));
        }
        let context = self.context;
        self.climbed
            .entry(space.clone())
            .or_insert_with(|| climb.nearest_containers(context, space, storeys))
            .clone()
    }

    fn target(&mut self, index: usize, to: &Selector, object: &Object) -> Selection {
        let context = self.context;
        self.targets
            .entry((index, object.id.clone()))
            .or_insert_with(|| selector_matches(context, to, object, &mut Vec::new()))
            .clone()
    }

    /// The candidates `row` names for `space`.
    fn candidates(
        &mut self,
        space: &Object,
        index: usize,
        row: &Row<'_>,
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
        if row.direct_access && !self.partners.contains_key(&space.id) {
            let index = self
                .index
                .ok_or_else(|| invalid("`direct_access` needs `access_path`"))?;
            self.partners
                .insert(space.id.clone(), index.partners(&space.id, AccessType::Any));
        }
        let mut candidates = Vec::new();
        for object in context.project.objects() {
            if object.id == space.id {
                continue;
            }
            let mut doubts = Vec::new();
            let mut evidence = Vec::new();
            match self.target(index, row.to, object) {
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
                match self.partners[&space.id].with(&object.id) {
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
        if let Some(known) = self.lengths.get(&key) {
            return known.clone();
        }
        let measured = measure_pair(self.context, measure, &key.1, &key.2);
        self.lengths.insert(key, measured.clone());
        measured
    }

    /// A space's representative point, cached.
    fn point(&mut self, space: &ObjectId) -> Result<(MetricPoint, Vec<Evidence>), Unavailable> {
        if let Some(known) = self.points.get(space) {
            return known.clone();
        }
        let located = representative_point(self.context, space);
        self.points.insert(space.clone(), located.clone());
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
        let Some(profile) = self.profile else {
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
        let walk = |targets: &[(&Candidate, MetricPoint)]| -> Result<Walked, Unavailable> {
            let request = NearestTargetRequest::try_new(
                origin.clone(),
                targets.iter().map(|(_, point)| point.clone()).collect(),
                profile,
            )
            .map_err(|error| incomplete(error.to_string()))?;
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

    /// Judges one row for one space.
    fn row(
        &mut self,
        space: &Object,
        index: usize,
        row: &Row<'_>,
    ) -> Result<Option<(Finding, Deviation)>, Unavailable> {
        let candidates = self.candidates(space, index, row)?;
        let nearest = match row.measure {
            Measure::Straight | Measure::Closest => {
                self.nearest_pairwise(row.measure, &space.id, &candidates)
            }
            Measure::Walking => self.nearest_walking(&space.id, &candidates),
        };
        let (most, least) = (nearest.most, nearest.least);
        let how = row.measure.describe();
        if let Some(maximum) = row.maximum
            && least > maximum
        {
            // No reachable destination is infinitely far.
            return Ok(Some((
                self.too_far(space, row, &candidates, &nearest, maximum),
                Deviation::above(maximum, least, most),
            )));
        }
        if let Some(minimum) = row.minimum
            && most < minimum
        {
            let (id, distance, cited) = nearest
                .sure
                .as_ref()
                .expect("a finite upper bound comes from a sure destination");
            let mut evidence = cited.clone();
            evidence.push(distance.evidence().clone());
            return Ok(Some((
                finding(
                    self.rule,
                    &space.id,
                    format!(
                        "{id} is {} m away {how}; {} requires at least {minimum} m",
                        shown(distance.lower(), distance.upper()),
                        row.name
                    ),
                    evidence,
                    vec![id.clone()],
                ),
                Deviation::below(minimum, least, most),
            )));
        }
        let within = row.maximum.is_none_or(|maximum| most <= maximum);
        let beyond = row.minimum.is_none_or(|minimum| least >= minimum);
        if within && beyond {
            return Ok(None);
        }
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
        let mut doubts = nearest.doubts;
        if doubts.is_empty() {
            doubts.push(format!(
                "it is known only to lie {} m away",
                shown(least, most)
            ));
        }
        doubts.sort();
        doubts.dedup();
        Err((
            nearest.reason,
            format!(
                "the nearest destination is {shown_most} {how}, {bounds} required, and {}",
                doubts.join("; ")
            ),
        ))
    }

    /// The finding for a space whose every possible destination lies
    /// beyond `maximum`, or which has none.
    fn too_far(
        &self,
        space: &Object,
        row: &Row<'_>,
        candidates: &[Candidate],
        nearest: &Nearest,
        maximum: f64,
    ) -> Finding {
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
        let (message, related) = if candidates.is_empty() {
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
        finding(self.rule, &space.id, message, evidence, related)
    }
}
