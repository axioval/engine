//! Spacing of parallel walls or beams on a storey, and how much of the
//! storey the bands between them cover.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use axioval_engine::template::Template;
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
use crate::plan_area::unavailable;
use crate::support::{Parameters, Traversal, Unavailable, invalid};

mod items;
mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use items::SpacingItems;
pub(crate) use measured::SpacingMeasures;

pub(crate) const NAME: &str = "wall-spacing";

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
///
/// It runs as a template ([`axioval_engine::template`]): the items of the
/// measured `wall_spacing` list of each storey, its pairs surely parallel
/// and facing with their distances and the area of each footprint outside
/// the bands, judged against the minimum and the area allowed.
pub struct WallSpacing;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for WallSpacing {
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

/// Checks the rule parameters the measured `wall_spacing` is handed, as the
/// rule states them: the capability's declaration, in its order and words.
pub(crate) fn check_arguments(
    arguments: &BTreeMap<String, axioval_ir::contract::ParameterValue>,
) -> Result<(), Unavailable> {
    let rule = crate::light_area::synthesised(arguments.clone());
    parse(&Parameters(&rule)).map(|_| ())
}

pub(crate) struct Coverage<'a> {
    pub(crate) maximum: f64,
    pub(crate) footprints: &'a Selector,
    pub(crate) footprint_path: Traversal,
    pub(crate) threshold: f64,
}

pub(crate) struct Config<'a> {
    pub(crate) members: &'a Selector,
    pub(crate) member_path: Traversal,
    pub(crate) tolerance: f64,
    pub(crate) minimum: Option<f64>,
    pub(crate) coverage: Option<Coverage<'a>>,
}

pub(crate) fn parse<'a>(parameters: &Parameters<'a>) -> Result<Config<'a>, Unavailable> {
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

pub(crate) struct Services<'a> {
    pub(crate) rectangles: &'a PlanSpanServiceHandle,
    pub(crate) proximity: &'a ProximityServiceHandle,
    pub(crate) extents: &'a VerticalExtentServiceHandle,
    pub(crate) areas: Option<&'a PlanAreaServiceHandle>,
}

impl<'a> Services<'a> {
    pub(crate) fn of(context: &RuleContext<'a>, config: &Config<'_>) -> Result<Self, Unavailable> {
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
pub(crate) struct Members<'m> {
    pub(crate) matched: &'m BTreeSet<ObjectId>,
    pub(crate) universe: &'m [&'m Object],
}

/// A finding (message, evidence, related objects).
pub(crate) type Found = (String, Vec<Evidence>, Vec<ObjectId>);

/// One pair of members as far as it was judged.
pub(crate) struct Pair {
    pub(crate) first: ObjectId,
    pub(crate) second: ObjectId,
    /// Parallel, facing and both selected.
    pub(crate) paired: Tri,
    /// Why `paired` or the distance is undecided.
    pub(crate) why: Vec<String>,
    pub(crate) distance: (f64, f64),
    /// The first member's long axis, or the second's.
    pub(crate) axis: Option<[f64; 2]>,
    pub(crate) evidence: Vec<Evidence>,
}

pub(crate) struct Storey<'s, 'a> {
    pub(crate) context: &'s RuleContext<'a>,
    pub(crate) config: &'s Config<'s>,
    pub(crate) services: &'s Services<'a>,
    pub(crate) object: &'s Object,
}

impl Storey<'_, '_> {
    /// Every pair within `reach` of each other in plan, and the members
    /// whose extent cannot be read.
    pub(crate) fn pairs(
        &self,
        reached: &[ObjectId],
        members: &Members<'_>,
        reach: f64,
    ) -> Result<(Vec<Pair>, Vec<String>), Unavailable> {
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
}

/// The area of `footprint` outside the bands, `(lower, upper)` square
/// metres, with the evidence measuring it: at most what the sure bands
/// (`least`) leave, at least what every possible band (`most`) leaves when
/// nothing is unknown (`known`), otherwise nothing, since anything unknown
/// may bound another band.
pub(crate) fn uncovered(
    areas: &PlanAreaServiceHandle,
    footprint: &ObjectId,
    (least, most): (&[PlanBand], &[PlanBand]),
    known: bool,
) -> Result<((f64, f64), Vec<Evidence>), Unavailable> {
    let measure = |bands: &[PlanBand]| {
        areas
            .measure_outside_bands(footprint, bands)
            .map_err(unavailable)
    };
    let mut cited = Vec::new();
    let area = measure(least)?;
    cited.push(area.evidence().clone());
    let upper = area.upper_square_metres();
    let lower = if known {
        let area = measure(most)?;
        cited.push(area.evidence().clone());
        area.lower_square_metres()
    } else {
        0.0
    };
    Ok(((lower, upper), cited))
}

/// The bands between parallel pairs at most a maximum apart.
pub(crate) struct Bands {
    /// Surely qualifying: the least the bands can cover.
    pub(crate) least: Vec<PlanBand>,
    /// Surely or possibly qualifying: the most.
    pub(crate) most: Vec<PlanBand>,
    /// Why the bands may cover more than `most`.
    pub(crate) unknown: Vec<String>,
    /// The members bounding a sure band.
    pub(crate) related: BTreeSet<ObjectId>,
}

impl Bands {
    pub(crate) fn of(pairs: &[Pair], maximum: f64) -> Self {
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
