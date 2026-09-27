//! `space-distance`: how far each space lies from its nearest destination
//! space, in a straight line or walking.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CentrePlacement, ColumnKind, CompiledRule, MetricPoint,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingServiceHandle, MobilityProfile,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanSpan, PlanSpanServiceHandle,
    RuleCapability, RuleContext, TableColumn, VerticalExtentServiceHandle,
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
/// footprints' centres (the plan-span service). `walking` is the shortest
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
/// it. Each pair is routed on its own until a many-target search exists.
pub struct SpaceDistance;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Measure {
    Straight,
    Walking,
}

impl Measure {
    fn describe(self) -> &'static str {
        match self {
            Self::Straight => "in a straight line between centres",
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
        let measure = match row.text("measure")?.unwrap_or("straight") {
            "straight" => Measure::Straight,
            "walking" => Measure::Walking,
            other => {
                return Err(invalid(format!(
                    "{name}: measure `{other}` is unsupported (straight, walking)"
                )));
            }
        };
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
                    Ok(Some(found)) => evaluation.push_finding(found),
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
#[derive(Clone)]
enum Distance {
    /// Between `lower` and `upper` metres.
    Between(f64, f64, Evidence),
    /// No route exists, with complete evidence.
    Unreachable(Evidence),
}

impl Distance {
    fn lower(&self) -> f64 {
        match self {
            Self::Between(lower, _, _) => *lower,
            Self::Unreachable(_) => f64::INFINITY,
        }
    }

    fn upper(&self) -> f64 {
        match self {
            Self::Between(_, upper, _) => *upper,
            Self::Unreachable(_) => f64::INFINITY,
        }
    }

    fn evidence(&self) -> &Evidence {
        match self {
            Self::Between(_, _, evidence) | Self::Unreachable(evidence) => evidence,
        }
    }
}

/// A destination that surely qualifies, or might.
struct Candidate {
    id: ObjectId,
    /// `None` when it surely qualifies, else why it might not.
    doubt: Option<String>,
    evidence: Vec<Evidence>,
    distance: Result<Distance, Unavailable>,
}

/// Nearest storeys, and the climb's evidence.
type Climb = Result<(BTreeSet<ObjectId>, Vec<Evidence>), Unavailable>;

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

    /// The candidates `row` names for `space`, each measured.
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
            let distance = self.distance(row.measure, &space.id, &object.id);
            candidates.push(Candidate {
                id: object.id.clone(),
                doubt: (!doubts.is_empty()).then(|| doubts.join("; ")),
                evidence,
                distance,
            });
        }
        Ok(candidates)
    }

    fn distance(
        &mut self,
        measure: Measure,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Result<Distance, Unavailable> {
        // A straight line has no direction; a route may.
        let key = match measure {
            Measure::Straight if to < from => (measure, to.clone(), from.clone()),
            _ => (measure, from.clone(), to.clone()),
        };
        if let Some(known) = self.lengths.get(&key) {
            return known.clone();
        }
        let measured = match measure {
            Measure::Straight => self.straight(&key.1, &key.2),
            Measure::Walking => self.walking(from, to),
        };
        self.lengths.insert(key, measured.clone());
        measured
    }

    fn straight(&self, from: &ObjectId, to: &ObjectId) -> Result<Distance, Unavailable> {
        let spans = self
            .context
            .services
            .get::<PlanSpanServiceHandle>()
            .ok_or_else(|| missing("plan-span"))?;
        let length = spans
            .measure_span(from, to, PlanSpan::Centres)
            .map_err(|error| incomplete(format!("{from} to {to}: {error}")))?;
        Ok(Distance::Between(
            length.lower_metres(),
            length.upper_metres(),
            length.evidence().clone(),
        ))
    }

    /// A space's representative point: the centre of its footprint, which
    /// must lie inside it, on its floor.
    fn point(&mut self, space: &ObjectId) -> Result<(MetricPoint, Vec<Evidence>), Unavailable> {
        if let Some(known) = self.points.get(space) {
            return known.clone();
        }
        let located = self.locate(space);
        self.points.insert(space.clone(), located.clone());
        located
    }

    fn locate(&self, space: &ObjectId) -> Result<(MetricPoint, Vec<Evidence>), Unavailable> {
        let services = self.context.services;
        let spans = services
            .get::<PlanSpanServiceHandle>()
            .ok_or_else(|| missing("plan-span"))?;
        let extents = services
            .get::<VerticalExtentServiceHandle>()
            .ok_or_else(|| missing("vertical-extent"))?;
        let centre = spans
            .measure_centre(space)
            .map_err(|error| incomplete(format!("the centre of {space}: {error}")))?;
        if !centre.is_exact() {
            return Err(incomplete(format!(
                "the centre of {space} is known only within {} m, so no route starts there",
                shown(centre.radius_metres(), centre.radius_metres())
            )));
        }
        match centre.placement() {
            CentrePlacement::Inside => {}
            CentrePlacement::Outside => {
                return Err(incomplete(format!(
                    "the centre of {space} lies outside its footprint, so it has no \
                     representative point to walk from"
                )));
            }
            CentrePlacement::Undecided => {
                return Err(incomplete(format!(
                    "the centre of {space} lies on its footprint's boundary"
                )));
            }
        }
        let extent = extents
            .measure_vertical_extent(space)
            .map_err(|error| incomplete(format!("the floor of {space}: {error}")))?;
        let floor = extent.bottom();
        if !floor.is_exact() {
            return Err(incomplete(format!(
                "the floor of {space} lies {} m high, not at one elevation",
                shown(floor.lower_metres(), floor.upper_metres())
            )));
        }
        let [x, y] = centre.point();
        let point = MetricPoint::try_new(space.clone(), [x, y, floor.lower_metres()])
            .map_err(|error| incomplete(format!("the centre of {space}: {error}")))?;
        Ok((
            point,
            vec![centre.evidence().clone(), extent.evidence().clone()],
        ))
    }

    fn walking(&mut self, from: &ObjectId, to: &ObjectId) -> Result<Distance, Unavailable> {
        let routes = self
            .context
            .services
            .get::<MetricRoutingServiceHandle>()
            .ok_or_else(|| missing("metric-routing"))?;
        let profile = self
            .profile
            .ok_or_else(|| invalid("a walking row needs a walking profile"))?;
        let (origin, _) = self.point(from)?;
        let (destination, _) = self.point(to)?;
        let request = MetricRouteRequest::new(origin, destination, profile);
        match routes.route(&request) {
            Ok(MetricRouteOutcome::Reachable(route)) => {
                let length = route.shortest_distance();
                Ok(Distance::Between(
                    length.lower_metres(),
                    length.upper_metres(),
                    route.evidence().clone(),
                ))
            }
            Ok(MetricRouteOutcome::Blocked(blocked)) => Ok(Distance::Unreachable(
                blocked.completeness().evidence().clone(),
            )),
            Err(error) => Err(incomplete(format!("walking from {from} to {to}: {error}"))),
        }
    }

    /// Judges one row for one space.
    fn row(
        &mut self,
        space: &Object,
        index: usize,
        row: &Row<'_>,
    ) -> Result<Option<Finding>, Unavailable> {
        let candidates = self.candidates(space, index, row)?;
        // The nearest distance is at most the nearest sure destination's
        // upper bound, and at least the least lower bound of every one that
        // might count, an unmeasured one counting as zero.
        let most = candidates
            .iter()
            .filter(|candidate| candidate.doubt.is_none())
            .filter_map(|candidate| candidate.distance.as_ref().ok())
            .map(Distance::upper)
            .fold(f64::INFINITY, f64::min);
        let least = candidates
            .iter()
            .map(|candidate| candidate.distance.as_ref().map_or(0.0, Distance::lower))
            .fold(f64::INFINITY, f64::min);
        let mut doubts: Vec<String> = Vec::new();
        let mut reason = NotEvaluatedReason::IncompleteEvidence;
        for candidate in &candidates {
            if let Some(doubt) = &candidate.doubt {
                doubts.push(doubt.clone());
            }
            if let Err((why, message)) = &candidate.distance {
                if *why == NotEvaluatedReason::MissingService {
                    reason = why.clone();
                }
                doubts.push(message.clone());
            }
        }
        let how = row.measure.describe();
        if let Some(maximum) = row.maximum
            && least > maximum
        {
            return Ok(Some(self.too_far(space, row, &candidates, maximum)));
        }
        if let Some(minimum) = row.minimum
            && most < minimum
        {
            let nearest = candidates
                .iter()
                .filter(|candidate| candidate.doubt.is_none())
                .filter_map(|candidate| {
                    candidate
                        .distance
                        .as_ref()
                        .ok()
                        .map(|distance| (candidate, distance))
                })
                .min_by(|a, b| a.1.upper().total_cmp(&b.1.upper()))
                .expect("a finite upper bound comes from a sure destination");
            let mut evidence = nearest.0.evidence.clone();
            evidence.push(nearest.1.evidence().clone());
            return Ok(Some(finding(
                self.rule,
                &space.id,
                format!(
                    "{} is {} m away {how}; {} requires at least {minimum} m",
                    nearest.0.id,
                    shown(nearest.1.lower(), nearest.1.upper()),
                    row.name
                ),
                evidence,
                vec![nearest.0.id.clone()],
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
        let nearest = if most.is_finite() {
            format!("at most {} m", shown(most, most))
        } else {
            "unknown".to_owned()
        };
        doubts.sort();
        doubts.dedup();
        Err((
            reason,
            format!(
                "the nearest destination is {nearest} {how}, {bounds} required, and {}",
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
        let reachable: Vec<(&Candidate, &Distance)> = candidates
            .iter()
            .filter_map(|candidate| {
                candidate
                    .distance
                    .as_ref()
                    .ok()
                    .map(|distance| (candidate, distance))
            })
            .filter(|(_, distance)| matches!(distance, Distance::Between(..)))
            .collect();
        let mut evidence = Vec::new();
        let (message, related) = if candidates.is_empty() {
            (
                format!(
                    "has no destination{filters}; {} requires one within {maximum} m {how}",
                    row.name
                ),
                Vec::new(),
            )
        } else if let Some((nearest, distance)) = reachable
            .iter()
            .min_by(|a, b| a.1.lower().total_cmp(&b.1.lower()))
        {
            evidence.extend(nearest.evidence.iter().cloned());
            evidence.push(distance.evidence().clone());
            (
                format!(
                    "the nearest destination{filters}, {}, is {} m away {how}; {} allows at \
                     most {maximum} m",
                    nearest.id,
                    shown(distance.lower(), distance.upper()),
                    row.name
                ),
                vec![nearest.id.clone()],
            )
        } else {
            for candidate in candidates {
                if let Ok(distance) = &candidate.distance {
                    evidence.push(distance.evidence().clone());
                }
            }
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
