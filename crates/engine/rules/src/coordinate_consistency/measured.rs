//! How an object's source's coordinate system departs from the reference
//! source's, statement by statement, as values: compared exactly as
//! `coordinate-consistency` compares them, so a tolerance on each is an
//! expression and the verdict the capability's.
//!
//! A statement one side makes and the other does not cannot be compared,
//! and neither can map conversions one source leaves unrecorded; a
//! statement neither makes, where that is no unknown, has no value.

use axioval_engine::{
    CoordinateSystemServiceHandle, MeasuredProvider, Measurement, NotEvaluatedReason,
    PropertyResolutionError, RuleContext, SourceCoordinateSystem,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension, SourceId};

use super::{
    Compared, CoordinateTolerance, Declaration, map_conversions, sites, true_norths, world_frames,
};
use crate::measured_kinds::refused;
use crate::support::{Unavailable, sources};

/// Measures `coordinate_shift`, `coordinate_turn`, `map_conversion`,
/// `map_scale_change` and `map_target_change`.
pub(crate) struct CoordinateMeasures;

const COORDINATE_SHIFT: &str = "coordinate_shift";
const COORDINATE_TURN: &str = "coordinate_turn";
const MAP_CONVERSION: &str = "map_conversion";
const MAP_SCALE_CHANGE: &str = "map_scale_change";
const MAP_TARGET_CHANGE: &str = "map_target_change";

/// The reference source: the one of the discipline `reference` names, or
/// the first source in identity order.
fn reference(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<SourceId, Unavailable> {
    let discipline = match call.argument("reference") {
        Some(MeasuredArgument::Text(discipline)) => Some(discipline.as_str()),
        _ => None,
    };
    if discipline.is_none() {
        return sources(context).into_iter().next().ok_or_else(|| {
            (
                NotEvaluatedReason::IncompleteEvidence,
                "the run checks no source".to_owned(),
            )
        });
    }
    let declaration = Declaration {
        reference: discipline,
        tolerance: CoordinateTolerance::default(),
        require_map: false,
    };
    declaration.sources(context).map(|(reference, _)| reference)
}

/// The coordinate systems of the reference and of `object`'s source.
fn systems(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(SourceCoordinateSystem, SourceCoordinateSystem), Unavailable> {
    let service = context
        .services
        .get::<CoordinateSystemServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "no coordinate-system service is registered".to_owned(),
            )
        })?;
    let reference = reference(call, context)?;
    let base = service.coordinate_system(&reference).map_err(|error| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("the reference `{reference}`'s coordinate system cannot be read: {error}"),
        )
    })?;
    let own = service
        .coordinate_system(&object.source)
        .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?;
    Ok((base, own))
}

fn unknown(reason: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, reason)
}

/// How far a value computed from stated coordinates may lie from the exact
/// one, in units in the last place: relative to the value, and absolute (an
/// angle between two plan directions cancels in its cross product).
#[derive(Clone, Copy)]
struct Rounding {
    relative: f64,
    absolute: f64,
}

/// A flag or a count: no arithmetic, no rounding.
const COUNTED: Rounding = Rounding {
    relative: 0.0,
    absolute: 0.0,
};
/// A difference of two scales.
const DIFFERENCE: Rounding = Rounding {
    relative: 1.0,
    absolute: 0.0,
};
/// A distance between two points: differences, squares, a sum and a root.
const DISTANCE: Rounding = Rounding {
    relative: 4.0,
    absolute: 0.0,
};
/// The rotation between two axis triples, from the norm of their
/// difference.
const ROTATION: Rounding = Rounding {
    relative: 8.0,
    absolute: 0.0,
};
/// The angle between two unit plan directions, from a cross and a dot
/// product.
const PLAN_ANGLE: Rounding = Rounding {
    relative: 4.0,
    absolute: 4.0,
};

/// A value computed from what both sources state, widened by its
/// computation's rounding, so it holds the exact one: cited exact when both
/// coordinate systems are (an identical statement stays an exact zero).
fn stated(
    value: f64,
    rounding: Rounding,
    dimension: Option<QuantityDimension>,
    (locator, exact): (String, bool),
) -> Measurement {
    let margin = if value == 0.0 && rounding.absolute == 0.0 {
        0.0
    } else {
        f64::EPSILON * rounding.relative.mul_add(value.abs(), rounding.absolute)
    };
    let (lower, upper) = if margin == 0.0 {
        (value, value)
    } else {
        (
            (value - margin).next_down().max(0.0),
            (value + margin).next_up(),
        )
    };
    crate::measured_kinds::interval((lower, upper), dimension, exact, locator)
}

impl CoordinateMeasures {
    fn measure_object(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let (base, own) = systems(call, object, context)?;
        let locator = format!(
            "{}: {} against {}",
            call.name(),
            own.evidence().locator,
            base.evidence().locator
        );
        let exact = own.evidence().exact && base.evidence().exact;
        let absent = |what: &str| Measurement::Absent {
            locator: format!("{locator}: {what} is stated by neither source"),
        };
        let length = Some(QuantityDimension::Length);
        let angle = Some(QuantityDimension::PlaneAngle);
        let of = call.choice("of");
        if call.name() == MAP_CONVERSION {
            let system = if of == Some("reference") { &base } else { &own };
            return Ok(stated(
                if system.map().is_some() { 1.0 } else { 0.0 },
                COUNTED,
                None,
                (locator, exact),
            ));
        }
        let frame = |compared: Compared<(f64, f64)>, what: &str| match compared {
            Compared::Both((shift, turn)) => Ok(if call.name() == COORDINATE_SHIFT {
                stated(shift, DISTANCE, length, (locator.clone(), exact))
            } else {
                stated(turn, ROTATION, angle, (locator.clone(), exact))
            }),
            Compared::Neither => Ok(absent(what)),
            Compared::Unknown(reason) => Err(unknown(reason)),
        };
        match (call.name(), of) {
            (_, Some("world")) => return frame(world_frames(&base, &own), "a world frame"),
            (_, Some("site")) => return frame(sites(&base, &own), "a site placement"),
            (_, Some("north")) => {
                return match true_norths(&base, &own) {
                    Compared::Both(turn) => Ok(stated(turn, PLAN_ANGLE, angle, (locator, exact))),
                    Compared::Neither => Ok(absent("true north")),
                    Compared::Unknown(reason) => Err(unknown(reason)),
                };
            }
            _ => {}
        }
        let (Some(a), Some(b)) = (base.map(), own.map()) else {
            return Err((
                NotEvaluatedReason::NotRecorded,
                format!(
                    "{} no map conversion, so whether the georeferences agree is unknown",
                    match (base.map().is_some(), own.map().is_some()) {
                        (true, false) => "this source states",
                        (false, true) => "the reference states",
                        _ => "neither source states",
                    }
                ),
            ));
        };
        let compared = map_conversions(a, b);
        Ok(match call.name() {
            MAP_TARGET_CHANGE => stated(
                if compared.target_differs { 1.0 } else { 0.0 },
                COUNTED,
                None,
                (locator, exact),
            ),
            MAP_SCALE_CHANGE => stated(compared.scale, DIFFERENCE, None, (locator, exact)),
            COORDINATE_TURN => stated(compared.turn, PLAN_ANGLE, angle, (locator, exact)),
            _ => match compared.shift {
                Compared::Both(shift) => stated(shift, DISTANCE, length, (locator, exact)),
                Compared::Neither => absent("a map offset"),
                Compared::Unknown(reason) => return Err(unknown(reason)),
            },
        })
    }
}

impl MeasuredProvider for CoordinateMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            COORDINATE_SHIFT,
            COORDINATE_TURN,
            MAP_CONVERSION,
            MAP_SCALE_CHANGE,
            MAP_TARGET_CHANGE,
        ]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Self::measure_object(call, object, context).map_err(refused(call.name(), object))
    }
}
