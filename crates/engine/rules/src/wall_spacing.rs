//! Spacing of parallel walls or beams on a storey, and how much of the
//! storey the bands between them cover.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, NotEvaluatedReason, ObjectBounds, ParameterDescriptor,
    ParameterType, PlanAreaServiceHandle, PlanBand, PlanRectangle, PlanSpanServiceHandle,
    ProximityProjection, ProximityRequest, ProximityServiceHandle, RuleCapability, RuleContext,
    VerticalExtentServiceHandle, projected_candidate_pairs,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::orientation::{
    Alignment, Tri, aligned, along, angle_tolerance, rectangle, rectangle_service,
};
use crate::pairs::refuse_all;
use crate::plan_area::{shown, unavailable};
use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

const NAME: &str = "wall-spacing";

/// Requires the parallel walls or beams on each selected storey to stand at
/// least a minimum apart in plan and, with a maximum, the bands between
/// parallel pairs at most that far apart to cover the storey's gross
/// footprint.
///
/// The members are the `members` objects the storey reaches along
/// `member_path`. Two members are a **parallel pair** when their long axes
/// (from the least-area rectangle of each footprint) lie within
/// `angle_tolerance` of parallel and they face each other: along the first
/// one's long axis, their extents share a stretch of positive length. Two
/// collinear walls meeting end to end are not a pair.
///
/// - **Minimum**: each parallel pair's plan distance (closest points,
///   through the proximity service's `horizontal` projection) is at least
///   `minimum`.
/// - **Maximum**: the band between each parallel pair at most `maximum`
///   apart (the convex hull of both footprints, cut to the stretch they
///   share) must cover the storey's gross footprint: the `footprints`
///   objects the storey reaches along `footprint_path`, such as its slabs.
///   The area of each footprint outside every band above `uncovered_above`
///   is a finding.
///
/// Every measure is an interval, and a member whose axes are not its own
/// (tied or unproven orientation, a square) is parallel to nothing surely.
/// A pair is judged too close only when surely parallel, facing, selected
/// and closer than the minimum; the uncovered area is bounded from above by
/// the sure bands and from below by every possible one, so an undecided
/// member leaves a finding or pass standing only when it cannot change it.
pub struct WallSpacing;

struct Coverage<'a> {
    maximum: f64,
    footprints: &'a Selector,
    footprint_path: Traversal,
    threshold: f64,
}

struct Config<'a> {
    members: &'a Selector,
    member_path: Traversal,
    tolerance: f64,
    minimum: Option<f64>,
    coverage: Option<Coverage<'a>>,
}

impl RuleCapability for WallSpacing {
    fn id(&self) -> &'static str {
        "axioval:capability.wall-spacing"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("members", ParameterType::Selector),
            ParameterDescriptor::required("member_path", ParameterType::StringList),
            ParameterDescriptor::required("angle_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("minimum", ParameterType::Quantity),
            ParameterDescriptor::optional("maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("footprints", ParameterType::Selector),
            ParameterDescriptor::optional("footprint_path", ParameterType::StringList),
            ParameterDescriptor::optional("uncovered_above", ParameterType::Quantity),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match parse(&Parameters(rule)) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(reason, format!("{NAME}: {message}"));
            }
        };
        let (storeys, mut evaluation) = select_objects(context, &rule.selector);
        let services = match Services::of(context, &config) {
            Ok(services) => services,
            Err((reason, message)) => return refuse_all(&storeys, evaluation, &reason, &message),
        };
        let (matched, selection) = select_objects(context, config.members);
        let undecided: BTreeSet<ObjectId> = selection
            .not_evaluated_outcomes()
            .iter()
            .filter_map(|outcome| outcome.object_id().cloned())
            .collect();
        let matched: BTreeSet<ObjectId> = matched.iter().map(|object| object.id.clone()).collect();
        let universe: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| matched.contains(&object.id) || undecided.contains(&object.id))
            .collect();
        let members = Members {
            matched: &matched,
            universe: &universe,
        };
        for storey in storeys {
            let judged = Storey {
                context,
                config: &config,
                services: &services,
                object: storey,
            };
            for check in judged.checks(&members) {
                match check {
                    Ok((message, evidence, related)) => {
                        evaluation
                            .push_finding(finding(rule, &storey.id, message, evidence, related));
                    }
                    Err((reason, message)) => {
                        evaluation.push_object_not_evaluated(storey.id.clone(), reason, message);
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
    let path = |name: &str| -> Result<Option<Traversal>, Unavailable> {
        parameters.strings(name)?.map(Traversal::path).transpose()
    };
    let minimum = length("minimum")?;
    let maximum = length("maximum")?;
    if let (Some(minimum), Some(maximum)) = (minimum, maximum)
        && minimum > maximum
    {
        return Err(invalid("minimum exceeds maximum"));
    }
    let footprints = parameters.selector("footprints")?;
    let footprint_path = path("footprint_path")?;
    let threshold = match parameters.quantity("uncovered_above")? {
        None => None,
        Some((value, QuantityDimension::Area)) if value >= 0.0 => Some(value),
        Some(_) => return Err(invalid("uncovered_above must be a non-negative area")),
    };
    let coverage = match (maximum, footprints, footprint_path, threshold) {
        (None, None, None, None) => None,
        (Some(maximum), Some(footprints), Some(footprint_path), Some(threshold)) => {
            Some(Coverage {
                maximum,
                footprints,
                footprint_path,
                threshold,
            })
        }
        _ => {
            return Err(invalid(
                "`maximum`, `footprints`, `footprint_path` and `uncovered_above` are declared \
                 together",
            ));
        }
    };
    if minimum.is_none() && coverage.is_none() {
        return Err(invalid("declare `minimum`, `maximum` or both"));
    }
    Ok(Config {
        members: parameters.required_selector("members")?,
        member_path: Traversal::path(
            parameters
                .strings("member_path")?
                .ok_or_else(|| invalid("parameter `member_path` is required"))?,
        )?,
        tolerance: angle_tolerance(parameters, "angle_tolerance")?
            .ok_or_else(|| invalid("parameter `angle_tolerance` is required"))?,
        minimum,
        coverage,
    })
}

struct Services<'a> {
    rectangles: &'a PlanSpanServiceHandle,
    proximity: &'a ProximityServiceHandle,
    extents: &'a VerticalExtentServiceHandle,
    areas: Option<&'a PlanAreaServiceHandle>,
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
            rectangles: rectangle_service(context)?,
            proximity: context
                .services
                .get::<ProximityServiceHandle>()
                .ok_or_else(|| missing("proximity"))?,
            extents: context
                .services
                .get::<VerticalExtentServiceHandle>()
                .ok_or_else(|| missing("vertical-extent"))?,
            areas: match config.coverage {
                None => None,
                Some(_) => Some(
                    context
                        .services
                        .get::<PlanAreaServiceHandle>()
                        .ok_or_else(|| missing("plan-area"))?,
                ),
            },
        })
    }
}

/// The candidate members: selected or undecided.
struct Members<'m> {
    matched: &'m BTreeSet<ObjectId>,
    universe: &'m [&'m Object],
}

/// A finding (message, evidence, related objects).
type Found = (String, Vec<Evidence>, Vec<ObjectId>);

/// One pair of members as far as it was judged.
struct Pair {
    first: ObjectId,
    second: ObjectId,
    /// Parallel, facing and both selected.
    paired: Tri,
    /// Why `paired` or the distance is undecided.
    why: Vec<String>,
    distance: (f64, f64),
    /// The first member's long axis, or the second's.
    axis: Option<[f64; 2]>,
    evidence: Vec<Evidence>,
}

struct Storey<'s, 'a> {
    context: &'s RuleContext<'a>,
    config: &'s Config<'s>,
    services: &'s Services<'a>,
    object: &'s Object,
}

impl Storey<'_, '_> {
    fn checks(&self, members: &Members<'_>) -> Vec<Result<Found, Unavailable>> {
        let (reached, mut evidence) =
            match self
                .config
                .member_path
                .related(self.context, &self.object.id, members.universe)
            {
                Ok(reached) => reached,
                Err(unavailable) => return vec![Err(unavailable)],
            };
        let (pairs, blind) = match self.pairs(&reached, members) {
            Ok(pairs) => pairs,
            Err(unavailable) => return vec![Err(unavailable)],
        };
        for pair in &pairs {
            evidence.extend(pair.evidence.iter().cloned());
        }
        let mut checks = Vec::new();
        if let Some(minimum) = self.config.minimum {
            checks.extend(Self::minimum(minimum, &pairs, &blind));
        }
        if let Some(coverage) = &self.config.coverage {
            checks.extend(self.coverage(coverage, &pairs, &blind, &evidence));
        }
        checks
    }

    /// Every pair near enough to matter, and the members whose extent cannot
    /// be read.
    fn pairs(
        &self,
        reached: &[ObjectId],
        members: &Members<'_>,
    ) -> Result<(Vec<Pair>, Vec<String>), Unavailable> {
        let reach = self
            .config
            .minimum
            .unwrap_or(0.0)
            .max(self.config.coverage.as_ref().map_or(0.0, |c| c.maximum));
        let mut bounds: BTreeMap<ObjectId, ObjectBounds> = BTreeMap::new();
        let mut blind = Vec::new();
        for member in reached {
            match self.services.proximity.bounds(member) {
                Ok(extent) if extent.object() == member => {
                    bounds.insert(member.clone(), extent);
                }
                Ok(_) => blind.push(format!("the bounds of {member} name another object")),
                Err(error) => blind.push(format!("{member} has no readable extent: {error}")),
            }
        }
        let listed: Vec<ObjectBounds> = bounds.values().cloned().collect();
        let candidates =
            projected_candidate_pairs(&listed, &listed, ProximityProjection::Horizontal, reach)
                .map_err(|error| (NotEvaluatedReason::InvalidEvidence, error.to_string()))?;
        let unordered: BTreeSet<(ObjectId, ObjectId)> = candidates
            .iter()
            .filter(|pair| pair.subject() < pair.counterpart())
            .map(|pair| (pair.subject().clone(), pair.counterpart().clone()))
            .collect();
        let mut rectangles: BTreeMap<ObjectId, Result<PlanRectangle, String>> = BTreeMap::new();
        let mut rectangle_of = |object: &ObjectId| {
            rectangles
                .entry(object.clone())
                .or_insert_with(|| {
                    rectangle(self.services.rectangles, object).map_err(|(_, message)| {
                        format!("the axes of {object} are unknown: {message}")
                    })
                })
                .clone()
        };
        let mut pairs = Vec::new();
        for (first, second) in unordered {
            let one = rectangle_of(&first);
            let other = rectangle_of(&second);
            pairs.push(self.pair(first, second, &one, &other, &bounds, members));
        }
        Ok((pairs, blind))
    }

    fn pair(
        &self,
        first: ObjectId,
        second: ObjectId,
        one: &Result<PlanRectangle, String>,
        other: &Result<PlanRectangle, String>,
        bounds: &BTreeMap<ObjectId, ObjectBounds>,
        members: &Members<'_>,
    ) -> Pair {
        let mut pair = Pair {
            paired: Tri::of(
                members.matched.contains(&first) && members.matched.contains(&second),
                false,
            ),
            why: Vec::new(),
            distance: (0.0, f64::INFINITY),
            axis: None,
            evidence: Vec::new(),
            first,
            second,
        };
        let parallel = match (one, other) {
            (Ok(one), Ok(other)) => {
                pair.evidence
                    .extend([one.evidence().clone(), other.evidence().clone()]);
                let (parallel, why) =
                    aligned(one, other, Alignment::Parallel, self.config.tolerance);
                pair.why.extend(why);
                parallel
            }
            (Err(why), _) | (_, Err(why)) => {
                pair.why.push(why.clone());
                Tri::Maybe
            }
        };
        let oriented = [one, other]
            .into_iter()
            .filter_map(|rectangle| rectangle.as_ref().ok())
            .find_map(|rectangle| {
                rectangle
                    .long_axis()
                    .ok()
                    .map(|long| (rectangle, rectangle.axes()[long]))
            });
        pair.axis = oriented.map(|(_, axis)| axis);
        let facing = match oriented {
            Some((rectangle, axis)) => self.facing(&pair, rectangle, axis, bounds),
            None => Ok(Tri::Maybe),
        };
        let facing = facing.unwrap_or_else(|message| {
            pair.why.push(message);
            Tri::Maybe
        });
        if parallel == Tri::No || facing == Tri::No {
            pair.paired = Tri::No;
            return pair;
        }
        pair.paired = pair.paired.and(parallel).and(facing);
        let request = ProximityRequest::projected(
            pair.first.clone(),
            pair.second.clone(),
            ProximityProjection::Horizontal,
        );
        match request.and_then(|request| self.services.proximity.measure_distance(&request)) {
            Ok(measured) => {
                pair.distance = measured.interval_metres();
                pair.evidence.push(measured.evidence().clone());
            }
            Err(error) => pair.why.push(format!(
                "the plan distance between {} and {} is unknown: {error}",
                pair.first, pair.second
            )),
        }
        pair
    }

    /// Whether the pair's extents along `axis` share a stretch of positive
    /// length.
    fn facing(
        &self,
        pair: &Pair,
        rectangle: &PlanRectangle,
        axis: [f64; 2],
        bounds: &BTreeMap<ObjectId, ObjectBounds>,
    ) -> Result<Tri, String> {
        let extents = self.services.extents;
        let one = along(extents, &pair.first, axis).map_err(|(_, message)| message)?;
        let other = along(extents, &pair.second, axis).map_err(|(_, message)| message)?;
        // Positions along an axis turned by up to the axis error move by at
        // most that angle times the distance from the rectangle's centre.
        let centre = rectangle.centre();
        let reach = [&pair.first, &pair.second]
            .into_iter()
            .filter_map(|id| bounds.get(id))
            .flat_map(|bounds| {
                let (min, max) = (bounds.enclosing().min(), bounds.enclosing().max());
                [min[0], max[0]].into_iter().flat_map(move |x| {
                    [min[1], max[1]]
                        .into_iter()
                        .map(move |y| (x - centre[0]).hypot(y - centre[1]))
                })
            })
            .fold(0.0, f64::max);
        let margin = 2.0 * reach * rectangle.axis_error_radians().sin();
        let shared_low = one[0].1.max(other[0].1);
        let shared_high = one[1].0.min(other[1].0);
        let possible_low = one[0].0.max(other[0].0);
        let possible_high = one[1].1.min(other[1].1);
        Ok(Tri::of(
            shared_high - shared_low > margin,
            possible_high - possible_low <= -margin,
        ))
    }

    fn minimum(minimum: f64, pairs: &[Pair], blind: &[String]) -> Vec<Result<Found, Unavailable>> {
        let mut checks = Vec::new();
        let mut unknown: Vec<String> = blind.to_vec();
        for pair in pairs {
            let (low, high) = pair.distance;
            let close = pair.paired.and(Tri::of(high < minimum, low >= minimum));
            match close {
                Tri::Yes => checks.push(Ok((
                    format!(
                        "{} and {} are parallel and {} m apart in plan; at least {minimum} m \
                         required",
                        pair.first,
                        pair.second,
                        shown(low, high)
                    ),
                    pair.evidence.clone(),
                    vec![pair.first.clone(), pair.second.clone()],
                ))),
                Tri::Maybe => {
                    let mut why = pair.why.clone();
                    if why.is_empty() {
                        why.push(format!(
                            "{} and {} may be closer than {minimum} m ({} m)",
                            pair.first,
                            pair.second,
                            shown(low, high)
                        ));
                    }
                    unknown.extend(why);
                }
                Tri::No => {}
            }
        }
        if !unknown.is_empty() {
            checks.push(Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "minimum spacing: whether every parallel pair stands {minimum} m apart is \
                     unknown: {}",
                    unknown.join("; ")
                ),
            )));
        }
        checks
    }

    fn coverage(
        &self,
        coverage: &Coverage<'_>,
        pairs: &[Pair],
        blind: &[String],
        evidence: &[Evidence],
    ) -> Vec<Result<Found, Unavailable>> {
        let Some(areas) = self.services.areas else {
            return Vec::new();
        };
        let maximum = coverage.maximum;
        let Bands {
            least,
            most,
            mut unknown,
            related,
        } = Bands::of(pairs, maximum);
        unknown.splice(0..0, blind.iter().cloned());
        let (matched, selection) = select_objects(self.context, coverage.footprints);
        if let Some(outcome) = selection.not_evaluated_outcomes().first() {
            return vec![Err((
                outcome.reason().clone(),
                format!(
                    "the storey's footprint objects are undecided: {}",
                    outcome.message()
                ),
            ))];
        }
        let (footprints, mut cited) =
            match coverage
                .footprint_path
                .related(self.context, &self.object.id, &matched)
            {
                Ok(found) => found,
                Err(unavailable) => return vec![Err(unavailable)],
            };
        if footprints.is_empty() {
            return vec![Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} reaches no footprint object, so it has no gross footprint to cover",
                    self.object.id
                ),
            ))];
        }
        cited.extend(evidence.iter().cloned());
        let threshold = coverage.threshold;
        let mut checks = Vec::new();
        for footprint in footprints {
            let measure = |bands: &[PlanBand]| {
                areas
                    .measure_outside_bands(&footprint, bands)
                    .map_err(unavailable)
            };
            let upper = match measure(&least) {
                Ok(area) => {
                    cited.push(area.evidence().clone());
                    area.upper_square_metres()
                }
                Err(error) => {
                    checks.push(Err(error));
                    continue;
                }
            };
            // Anything unknown may bound another band: nothing is surely left.
            let lower = if unknown.is_empty() {
                match measure(&most) {
                    Ok(area) => {
                        cited.push(area.evidence().clone());
                        area.lower_square_metres()
                    }
                    Err(error) => {
                        checks.push(Err(error));
                        continue;
                    }
                }
            } else {
                0.0
            };
            let what = format!(
                "{} m² of {footprint} lies outside every band between parallel members at most \
                 {maximum} m apart",
                shown(lower, upper)
            );
            if lower > threshold {
                let mut objects: Vec<ObjectId> = related.iter().cloned().collect();
                objects.push(footprint.clone());
                checks.push(Ok((
                    format!("{what}; at most {threshold} m² allowed"),
                    cited.clone(),
                    objects,
                )));
            } else if upper > threshold {
                let mut message = format!("{what}, which straddles {threshold} m²");
                for reason in &unknown {
                    message.push_str("; ");
                    message.push_str(reason);
                }
                checks.push(Err((NotEvaluatedReason::IncompleteEvidence, message)));
            }
        }
        checks
    }
}

/// The bands between parallel pairs at most a maximum apart.
struct Bands {
    /// Surely qualifying: the least the bands can cover.
    least: Vec<PlanBand>,
    /// Surely or possibly qualifying: the most.
    most: Vec<PlanBand>,
    /// Why the bands may cover more than `most`.
    unknown: Vec<String>,
    /// The members bounding a sure band.
    related: BTreeSet<ObjectId>,
}

impl Bands {
    fn of(pairs: &[Pair], maximum: f64) -> Self {
        let mut least = Vec::new();
        let mut most = Vec::new();
        let mut unknown = Vec::new();
        let mut related = BTreeSet::new();
        for pair in pairs {
            let (low, high) = pair.distance;
            let band = pair.paired.and(Tri::of(high <= maximum, low > maximum));
            if band == Tri::No {
                continue;
            }
            let Some(axis) = pair.axis else {
                unknown.push(format!(
                    "{} and {} may bound a band, but neither has a long axis",
                    pair.first, pair.second
                ));
                continue;
            };
            let Ok(built) = PlanBand::try_new(pair.first.clone(), pair.second.clone(), axis) else {
                unknown.push(format!(
                    "no band can be drawn between {} and {}",
                    pair.first, pair.second
                ));
                continue;
            };
            if band == Tri::Yes {
                least.push(built.clone());
                related.extend([pair.first.clone(), pair.second.clone()]);
            }
            most.push(built);
        }
        Self {
            least,
            most,
            unknown,
            related,
        }
    }
}
