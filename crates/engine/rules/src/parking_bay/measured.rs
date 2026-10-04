//! A bay's own measurements as values, measured exactly as `parking-bay`
//! measures them: the sides of its least-area rectangle, how many of its
//! ends and sides obstacles obstruct (and how many stand within it), and
//! the angle between its long axis and the long axes of nearby objects,
//! one member per object, so its orientation to an aisle is an expression
//! with a tolerance.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::FRAC_PI_2;

use axioval_engine::{
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, NotEvaluatedReason, PlanRectangle,
    PlanSpanServiceHandle, PropertyResolutionError, ProximityServiceHandle, RuleContext,
    VerticalExtentServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId, QuantityDimension};

use super::{Bay, Config, Nearby, Obstructions, Services, rectangle_of, within_reach};
use crate::measured_kinds::{objects_of_kinds, resolution_error};
use crate::orientation::{Tri, rectangle, rectangle_service};
use crate::support::{Unavailable, invalid};

/// Measures a bay's sides, obstructions and the axes near it.
pub(crate) struct BayMeasures;

const RECTANGLE_SIDE: &str = "rectangle_side";
const OBSTRUCTION_COUNT: &str = "obstruction_count";
const AXES_WITHIN: &str = "axes_within";

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

fn service<'a, T: Send + Sync + 'static>(
    context: &RuleContext<'a>,
    what: &str,
) -> Result<&'a T, Unavailable> {
    context.services.get::<T>().ok_or_else(|| {
        (
            NotEvaluatedReason::MissingService,
            format!("{what} service is not registered"),
        )
    })
}

/// Degrees `(lower, upper)` within `[0, 90]` as radians, rounded outward.
fn radians((lower, upper): (f64, f64)) -> (f64, f64) {
    (
        lower.to_radians().next_down().max(0.0),
        upper.to_radians().next_up().min(FRAC_PI_2.next_up()),
    )
}

/// The objects near `bay` among `objects`, as `parking-bay` finds them.
fn nearby(
    proximity: &ProximityServiceHandle,
    objects: BTreeSet<ObjectId>,
    bay: &Object,
    reach: f64,
) -> Result<Nearby, Unavailable> {
    let nearby = Nearby::of(proximity, objects, &BTreeSet::new(), &[bay], reach)?;
    if let Some(unavailable) = nearby.unbounded.get(&bay.id) {
        return Err(unavailable.clone());
    }
    Ok(nearby)
}

impl BayMeasures {
    fn side(
        call: &MeasuredCall,
        bay: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let own = rectangle(rectangle_service(context)?, &bay.id)?;
        let sides = own.width_and_length().map_err(|reason| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                format!("its own axes are unknown: {reason}"),
            )
        })?;
        let (lower, upper) = sides[usize::from(call.choice("side") == Some("length"))];
        Ok(Measurement::Value {
            lower,
            upper,
            dimension: Some(QuantityDimension::Length),
            locator: own.evidence().locator.clone(),
        })
    }

    /// How many ends or sides the obstacles obstruct, or how many stand
    /// within the bay: surely, and at most.
    fn obstructions(
        call: &MeasuredCall,
        bay: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let side_zone = length(call, "side_zone");
        if side_zone == Some(0.0) {
            return Err(invalid("side_zone must be positive"));
        }
        let reach = length(call, "reach").unwrap_or(0.0);
        let obstacles = objects_of_kinds(context, call, "obstacles", &bay.id)
            .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
        let proximity = service::<ProximityServiceHandle>(context, "proximity")?;
        let rectangles = service::<PlanSpanServiceHandle>(context, "plan-span")?;
        let services = Services {
            rectangles: Some(rectangles),
            extents: Some(service::<VerticalExtentServiceHandle>(
                context,
                "vertical-extent",
            )?),
            proximity: Some(proximity),
        };
        let near = nearby(proximity, obstacles, bay, reach)?;
        let config = Config::bare();
        let judged = Bay {
            config: &config,
            services: &services,
            object: bay,
        };
        let every = Selector::All;
        let counted = judged.count(
            &Obstructions {
                obstacles: &every,
                reach,
                ends: None,
                sides: None,
                side_zone,
            },
            &near,
            Some(&rectangle(rectangles, &bay.id)),
        )?;
        let (surely, most) = match call.choice("at") {
            Some("ends") => (counted.ends.surely, counted.ends.most),
            Some("sides") => (counted.sides.surely, counted.sides.most),
            _ => counted.within,
        };
        let mut locators: Vec<String> = counted
            .evidence
            .iter()
            .map(|evidence| evidence.locator.clone())
            .collect();
        if locators.is_empty() {
            locators.push(format!("{OBSTRUCTION_COUNT}:{}", bay.id));
        }
        #[allow(clippy::cast_precision_loss)]
        Ok(Measurement::Value {
            lower: surely as f64,
            upper: most as f64,
            dimension: None,
            locator: locators.join("; "),
        })
    }

    /// One member per object of the kinds `of` names that may lie within
    /// `reach` of the bay in plan, stating the angle between the long axes:
    /// certain when it surely lies within reach.
    fn axes(
        call: &MeasuredCall,
        bay: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let reach = length(call, "reach").unwrap_or(0.0);
        let others = objects_of_kinds(context, call, "of", &bay.id)
            .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
        let proximity = service::<ProximityServiceHandle>(context, "proximity")?;
        let rectangles = service::<PlanSpanServiceHandle>(context, "plan-span")?;
        let near = nearby(proximity, others, bay, reach)?;
        let own = rectangle(rectangles, &bay.id)?;
        let undecided = |why: String| MeasuredMember {
            certain: false,
            exact: false,
            fields: BTreeMap::from([("angle", MemberValue::Undecided { why })]),
        };
        let mut members: Vec<MeasuredMember> = near
            .blind
            .iter()
            .map(|other| undecided(format!("{other} has no readable extent and may be near")))
            .collect();
        for other in near.near(&bay.id) {
            let within = match within_reach(proximity, &bay.id, other, reach) {
                Ok((within, _)) => within,
                Err((_, message)) => {
                    members.push(undecided(format!(
                        "whether {other} is near is unknown: {message}"
                    )));
                    continue;
                }
            };
            if within == Tri::No {
                continue;
            }
            let (angle, exact) = angle(&own, rectangles, other);
            members.push(MeasuredMember {
                certain: within == Tri::Yes,
                exact,
                fields: BTreeMap::from([("angle", angle)]),
            });
        }
        Ok(members)
    }
}

/// The acute angle between the long axes of `own` and `other`.
/// The angle between both long axes, and whether both rectangles are exact.
fn angle(
    own: &PlanRectangle,
    rectangles: &PlanSpanServiceHandle,
    other: &ObjectId,
) -> (MemberValue, bool) {
    let theirs = match rectangle_of(rectangles, other) {
        Ok(theirs) => theirs,
        Err(why) => return (MemberValue::Undecided { why }, false),
    };
    let exact = own.is_exact() && theirs.is_exact();
    let value = match own.long_axis_angle(&theirs) {
        Ok(degrees) => {
            let (lower, upper) = radians(degrees);
            MemberValue::Measured(Measurement::Value {
                lower,
                upper,
                dimension: Some(QuantityDimension::PlaneAngle),
                locator: format!("{}; {}", own.evidence().locator, theirs.evidence().locator),
            })
        }
        Err(why) => MemberValue::Undecided { why },
    };
    (value, exact)
}

fn bay<'a>(
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

impl MeasuredProvider for BayMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[OBSTRUCTION_COUNT, RECTANGLE_SIDE]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[AXES_WITHIN]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let bay = bay(context, object)?;
        match call.name() {
            RECTANGLE_SIDE => Self::side(call, bay, context),
            OBSTRUCTION_COUNT => Self::obstructions(call, bay, context),
            _ => return Err(PropertyResolutionError::InvalidRequest),
        }
        .map_err(refused(call.name(), object))
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        if call.name() != AXES_WITHIN {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        Self::axes(call, bay(context, object)?, context).map_err(refused(call.name(), object))
    }
}
