//! How a source's coordinate system departs from the reference source's,
//! as measured values of the source: compared exactly as
//! [`compare_coordinate_systems`] compares them, the one comparison
//! `coordinate-consistency` and hosts share.
//!
//! Every value here is a source's (or the project's), never an object's:
//! read on an object, it is the value of the object's source, and a
//! template judging the sources reads it for a source holding no object.
//!
//! - `coordinate_shift`, `coordinate_turn`, `map_conversion`,
//!   `map_scale_change` and `map_target_change`: one statement compared, a
//!   number. A statement one side makes and the other does not cannot be
//!   compared, and neither can map conversions one source leaves
//!   unrecorded; a statement neither makes, where that is no unknown, has
//!   no value.
//! - `coordinate_reference` (of the project): how many sources are
//!   compared with the reference, which it cites.
//! - `coordinate_differences` (a member list): each statement differing
//!   beyond the tolerances, or not comparable, in words, then the
//!   georeference.

use axioval_engine::{
    Citation, CoordinateSystemServiceHandle, MeasuredMember, MeasuredProvider, Measurement,
    MemberValue, NotEvaluatedReason, PropertyResolutionError, RuleContext, SourceCoordinateSystem,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension, SourceId};

use super::{
    Compared, CoordinateConsistency, CoordinateTolerance, compare_coordinate_systems,
    map_conversions, reference_source, sites, true_norths, world_frames,
};
use crate::measured_kinds::resolution_error;
use crate::support::{Unavailable, invalid};

/// Measures the coordinate values of a source and of the project.
pub(crate) struct CoordinateMeasures;

const COORDINATE_SHIFT: &str = "coordinate_shift";
const COORDINATE_TURN: &str = "coordinate_turn";
const MAP_CONVERSION: &str = "map_conversion";
const MAP_SCALE_CHANGE: &str = "map_scale_change";
const MAP_TARGET_CHANGE: &str = "map_target_change";
const COORDINATE_REFERENCE: &str = "coordinate_reference";
const COORDINATE_DIFFERENCES: &str = "coordinate_differences";

/// A refusal of `name` of `subject` (a source, or the project), as a
/// property resolution states it.
fn refused(
    name: &str,
    subject: impl std::fmt::Display,
) -> impl Fn(Unavailable) -> PropertyResolutionError {
    move |(reason, why)| resolution_error((reason, format!("`{name}` of {subject}: {why}")))
}

/// The discipline `reference` names, if any.
fn discipline(call: &MeasuredCall) -> Option<&str> {
    match call.argument("reference") {
        Some(MeasuredArgument::Text(discipline)) => Some(discipline.as_str()),
        _ => None,
    }
}

/// The run's coordinate-system service.
fn service<'a>(
    context: &RuleContext<'a>,
) -> Result<&'a CoordinateSystemServiceHandle, Unavailable> {
    context
        .services
        .get::<CoordinateSystemServiceHandle>()
        .ok_or_else(|| {
            (
                NotEvaluatedReason::MissingService,
                "no coordinate-system service is registered".to_owned(),
            )
        })
}

/// The reference source: the one of the discipline `reference` names, or
/// the first source in identity order.
fn reference(call: &MeasuredCall, context: &RuleContext<'_>) -> Result<SourceId, Unavailable> {
    let Some(discipline) = discipline(call) else {
        return crate::support::sources(context)
            .into_iter()
            .next()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::IncompleteEvidence,
                    "the run checks no source".to_owned(),
                )
            });
    };
    reference_source(context, Some(discipline)).map(|(reference, _)| reference)
}

/// The coordinate systems of the reference and of `source`.
fn systems(
    call: &MeasuredCall,
    source: &SourceId,
    context: &RuleContext<'_>,
) -> Result<(SourceCoordinateSystem, SourceCoordinateSystem), Unavailable> {
    let service = service(context)?;
    let reference = reference(call, context)?;
    let base = service.coordinate_system(&reference).map_err(|error| {
        (
            NotEvaluatedReason::IncompleteEvidence,
            format!("the reference `{reference}`'s coordinate system cannot be read: {error}"),
        )
    })?;
    let own = service
        .coordinate_system(source)
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
    /// One statement of `source`'s coordinate system compared with the
    /// reference's.
    fn measure_of(
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, Unavailable> {
        let (base, own) = systems(call, source, context)?;
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

    /// How many sources are compared with the reference, which it cites.
    fn reference_of(
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), Unavailable> {
        let (reference, others) = reference_source(context, discipline(call))?;
        #[allow(clippy::cast_precision_loss)]
        let compared = others.len() as f64;
        Ok((
            Measurement::Rounded {
                lower: compared,
                upper: compared,
                dimension: None,
                locator: format!("{COORDINATE_REFERENCE}: `{reference}`"),
            },
            Citation {
                sources: vec![reference],
                ..Citation::default()
            },
        ))
    }

    /// Each statement of `source`'s coordinate system departing from the
    /// reference's beyond the tolerances, or not comparable, in
    /// [`compare_coordinate_systems`]'s words and order, then the
    /// georeference; of the reference itself, only a missing map
    /// conversion the call requires. The evidence of both systems.
    fn differences_of(
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
        let number = |key: &str| match call.argument(key) {
            Some(MeasuredArgument::Length(value) | MeasuredArgument::Number(value)) => Ok(*value),
            _ => Err(invalid(format!("`{key}` states no number"))),
        };
        let tolerance = CoordinateTolerance::try_new(
            number("length")?,
            number("angle")?.to_radians(),
            number("scale")?,
        )
        .map_err(invalid)?;
        let require_map = matches!(
            call.argument("require_map"),
            Some(MeasuredArgument::Truth(true))
        );
        let service = service(context)?;
        let reference = reference(call, context)?;
        if *source == reference {
            // The reference is never compared with itself; a coordinate
            // system it cannot read leaves every other source open instead.
            let Ok(base) = service.coordinate_system(&reference) else {
                return Ok((Vec::new(), Vec::new()));
            };
            let members = if require_map && base.map().is_none() {
                vec![item(
                    Some(true),
                    "states no map conversion; the federation requires one",
                    true,
                    base.evidence().exact,
                )]
            } else {
                Vec::new()
            };
            return Ok((members, vec![base.evidence().clone()]));
        }
        let (base, own) = systems(call, source, context)?;
        let exact = base.evidence().exact && own.evidence().exact;
        let consistency = compare_coordinate_systems(&base, &own, tolerance);
        let mut members: Vec<MeasuredMember> = consistency
            .differences
            .iter()
            .map(|(_, words)| item(Some(true), words, true, exact))
            .chain(
                consistency
                    .unknown
                    .iter()
                    .map(|(_, why)| item(None, why, true, exact)),
            )
            .collect();
        if let Some((found, words, recorded)) = georeference(&consistency, require_map) {
            members.push(item(found, &words, recorded, exact));
        }
        Ok((
            members,
            vec![base.evidence().clone(), own.evidence().clone()],
        ))
    }
}

/// Whether two sources' georeferences agree, from whether each states a
/// map conversion: `None` where they are compared (both state one) or
/// nothing is to say; otherwise whether that is a difference (`Some(true)`,
/// a source stating none where one is required) or unknown (`None`), its
/// words and whether it is recorded.
fn georeference(
    consistency: &CoordinateConsistency,
    require_map: bool,
) -> Option<(Option<bool>, String, bool)> {
    match consistency.georeferenced {
        (_, false) if require_map => Some((Some(true), "states no map conversion".into(), true)),
        (true, true) => None,
        // The reference's own missing conversion is its own finding.
        (false, true) if require_map => None,
        (reference_map, source_map) => Some((
            None,
            format!(
                "{} no map conversion, so whether the georeferences agree is unknown",
                match (reference_map, source_map) {
                    (true, false) => "this source states",
                    (false, true) => "the reference states",
                    _ => "neither source states",
                }
            ),
            false,
        )),
    }
}

/// One statement compared: found (`Some(true)`), or undecided (`None`)
/// with why, in `words`.
fn item(found: Option<bool>, words: &str, recorded: bool, exact: bool) -> MeasuredMember {
    let found = match found {
        Some(value) => MemberValue::Truth {
            value,
            locator: format!("{COORDINATE_DIFFERENCES}: {words}"),
        },
        None => MemberValue::Undecided {
            why: words.to_owned(),
        },
    };
    MeasuredMember {
        certain: true,
        exact,
        fields: [
            ("found", found),
            (
                "finding",
                MemberValue::Text {
                    text: words.to_owned(),
                },
            ),
            (
                "recorded",
                MemberValue::Truth {
                    value: recorded,
                    locator: format!("{COORDINATE_DIFFERENCES}: {words}"),
                },
            ),
        ]
        .into(),
    }
}

impl MeasuredProvider for CoordinateMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            COORDINATE_REFERENCE,
            COORDINATE_SHIFT,
            COORDINATE_TURN,
            MAP_CONVERSION,
            MAP_SCALE_CHANGE,
            MAP_TARGET_CHANGE,
        ]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[COORDINATE_DIFFERENCES]
    }

    /// A source's value, read on an object: its source's.
    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        self.measure_source(call, &object.source, context)
            .map(|(measurement, _)| measurement)
    }

    fn measure_source(
        &self,
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        Self::measure_of(call, source, context)
            .map(|measurement| (measurement, Citation::default()))
            .map_err(refused(call.name(), source))
    }

    fn measure_project(
        &self,
        call: &MeasuredCall,
        context: &RuleContext<'_>,
    ) -> Result<(Measurement, Citation), PropertyResolutionError> {
        Self::reference_of(call, context).map_err(refused(call.name(), "the project"))
    }

    fn members_of_source(
        &self,
        call: &MeasuredCall,
        source: &SourceId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        Self::differences_of(call, source, context).map_err(refused(call.name(), source))
    }
}
