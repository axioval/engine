//! A storey's parallel members as values, paired exactly as `wall-spacing`
//! pairs them: each parallel pair as a measured member stating its plan
//! distance, and the largest area of a footprint the bands between pairs
//! at most a maximum apart leave uncovered.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason,
    PlanAreaServiceHandle, PropertyResolutionError, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Bands, Config, Members, Pair, Services, Storey, uncovered};
use crate::measured_kinds::{objects_of_kinds, resolution_error};
use crate::orientation::Tri;
use crate::support::{Traversal, Unavailable, invalid};

/// Measures a storey's parallel pairs and the area their bands leave.
pub(crate) struct SpacingMeasures;

const PARALLEL_PAIRS: &str = "parallel_pairs";
const BAND_UNCOVERED_AREA: &str = "band_uncovered_area";

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

fn path(call: &MeasuredCall, key: &str) -> Result<Traversal, Unavailable> {
    match call.argument(key) {
        Some(MeasuredArgument::Path(steps)) => Traversal::path(steps),
        _ => Err(invalid(format!("`{key}` is required"))),
    }
}

/// The objects of the kinds `key` names, as a universe for a path.
fn kinds<'a>(
    context: &RuleContext<'a>,
    call: &MeasuredCall,
    key: &str,
    object: &ObjectId,
) -> Result<(BTreeSet<ObjectId>, Vec<&'a Object>), Unavailable> {
    let found = objects_of_kinds(context, call, key, object)
        .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
    let universe = context
        .project
        .objects()
        .filter(|candidate| found.contains(&candidate.id))
        .collect();
    Ok((found, universe))
}

/// The parallel pairs among the members `object` reaches within `reach`,
/// and why more members may pair, as `wall-spacing` finds them.
fn pairs(
    call: &MeasuredCall,
    object: &Object,
    context: &RuleContext<'_>,
    reach: f64,
) -> Result<(Vec<Pair>, Vec<String>), Unavailable> {
    let tolerance = length(call, "angle_tolerance");
    if tolerance >= 45.0 {
        return Err(invalid("angle_tolerance must lie in [0, 45) degrees"));
    }
    let every = Selector::All;
    let config = Config {
        members: &every,
        member_path: path(call, "member_path")?,
        tolerance,
        minimum: None,
        coverage: None,
    };
    let services = Services::of(context, &config)?;
    let (matched, universe) = kinds(context, call, "members", &object.id)?;
    let storey = Storey {
        context,
        config: &config,
        services: &services,
        object,
    };
    let (reached, _) = config.member_path.related(context, &object.id, &universe)?;
    storey.pairs(
        &reached,
        &Members {
            matched: &matched,
            universe: &universe,
        },
        reach,
    )
}

impl SpacingMeasures {
    /// One member per pair that may be parallel and facing, certain when it
    /// surely is; a member whose extent cannot be read may pair with any.
    fn pairs(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let (pairs, blind) = pairs(call, object, context, length(call, "reach"))?;
        let undecided = |why: String| MeasuredMember {
            certain: false,
            exact: false,
            fields: BTreeMap::from([("distance", MemberValue::Undecided { why })]),
            evidence: Vec::new(),
        };
        let mut members: Vec<MeasuredMember> = blind.into_iter().map(undecided).collect();
        for pair in pairs {
            if pair.paired == Tri::No {
                continue;
            }
            let (lower, upper) = pair.distance;
            let distance = if upper.is_finite() {
                MemberValue::Measured(Measurement::Value {
                    lower,
                    upper,
                    dimension: Some(QuantityDimension::Length),
                    locator: pair
                        .evidence
                        .last()
                        .map_or_else(String::new, |evidence| evidence.locator.clone()),
                })
            } else {
                MemberValue::Undecided {
                    why: pair.why.join("; "),
                }
            };
            members.push(MeasuredMember {
                certain: pair.paired == Tri::Yes,
                exact: pair.evidence.iter().all(|evidence| evidence.exact),
                fields: BTreeMap::from([("distance", distance)]),
                evidence: Vec::new(),
            });
        }
        Ok(members)
    }

    /// The largest area of a footprint `object` reaches that lies outside
    /// every band between parallel pairs at most `maximum` apart.
    fn uncovered(
        call: &MeasuredCall,
        object: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let maximum = length(call, "maximum");
        let areas = context
            .services
            .get::<PlanAreaServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "plan-area service is not registered".to_owned(),
                )
            })?;
        let (pairs, blind) = pairs(call, object, context, maximum)?;
        let Bands {
            least,
            most,
            mut unknown,
            ..
        } = Bands::of(&pairs, maximum);
        unknown.extend(blind);
        let (_, universe) = kinds(context, call, "footprints", &object.id)?;
        let (footprints, _) =
            path(call, "footprint_path")?.related(context, &object.id, &universe)?;
        let mut largest: Option<(f64, f64)> = None;
        let mut locators = Vec::new();
        // Cited as exactly as every pair and area it was measured from.
        let mut exact = unknown.is_empty()
            && pairs
                .iter()
                .flat_map(|pair| &pair.evidence)
                .all(|evidence| evidence.exact);
        for footprint in footprints {
            let ((lower, upper), cited) =
                uncovered(areas, &footprint, (&least, &most), unknown.is_empty())?;
            exact &= cited.iter().all(|evidence| evidence.exact);
            locators.extend(cited.into_iter().map(|evidence| evidence.locator));
            largest = Some(largest.map_or((lower, upper), |(low, high)| {
                (low.max(lower), high.max(upper))
            }));
        }
        let (lower, upper) = largest.ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "{} reaches no footprint object, so it has no gross footprint to cover",
                    object.id
                ),
            )
        })?;
        Ok(crate::measured_kinds::interval(
            (lower, upper),
            Some(QuantityDimension::Area),
            exact,
            locators.join("; "),
        ))
    }
}

fn storey<'a>(
    context: &RuleContext<'a>,
    object: &ObjectId,
) -> Result<&'a Object, PropertyResolutionError> {
    context.project.object(object).ok_or_else(|| {
        PropertyResolutionError::Unavailable(format!("{object} is not in the project"))
    })
}

fn refused(name: &str, object: &ObjectId) -> impl Fn(Unavailable) -> PropertyResolutionError {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of {object}: {why}")))
}

impl MeasuredProvider for SpacingMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[BAND_UNCOVERED_AREA]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[PARALLEL_PAIRS]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        if call.name() != BAND_UNCOVERED_AREA {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        Self::uncovered(call, storey(context, object)?, context)
            .map_err(refused(call.name(), object))
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        if call.name() != PARALLEL_PAIRS {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        Self::pairs(call, storey(context, object)?, context).map_err(refused(call.name(), object))
    }
}
