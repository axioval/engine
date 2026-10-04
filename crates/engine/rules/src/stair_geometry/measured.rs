//! Stair and ramp measurements as values, measured through the
//! walking-surface service exactly as `stair-geometry` and `ramp-geometry`
//! measure them: a flight's steps and a ramp's runs as members, a flight's
//! rise and width, and the landing at either end.

use std::collections::BTreeMap;

use axioval_engine::{
    HandrailEvidence, HandrailRequest, LandingRequest, MeasuredInterval, MeasuredMember,
    MeasuredProvider, Measurement, MemberValue, PropertyResolutionError, RailMeasurement, RailSide,
    RiserClosure, RuleContext, TreadFlight, TreadFlightRequest, WalkingEnd, WalkingStretch,
    WalkingSurfaceServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{ObjectId, QuantityDimension};

use super::handrails::RiserOffsets;
use super::service_error;

/// Measures stair and ramp values.
pub(crate) struct StairMeasures;

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

fn value(
    interval: MeasuredInterval,
    dimension: Option<QuantityDimension>,
    locator: String,
) -> Measurement {
    Measurement::Value {
        lower: interval.lower(),
        upper: interval.upper(),
        dimension,
        locator,
    }
}

fn number(
    interval: Option<MeasuredInterval>,
    dimension: Option<QuantityDimension>,
    locator: String,
) -> MemberValue {
    MemberValue::Measured(match interval {
        Some(interval) => value(interval, dimension, locator),
        None => Measurement::Absent { locator },
    })
}

/// `2r + g`, an interval sure to hold it, as `stair-geometry` computes it.
fn step_length(riser: MeasuredInterval, going: MeasuredInterval) -> Option<MeasuredInterval> {
    let lower = 2.0f64.mul_add(riser.lower(), going.lower()).next_down();
    let upper = 2.0f64.mul_add(riser.upper(), going.upper()).next_up();
    MeasuredInterval::try_new(lower, upper).ok()
}

fn walking(
    context: &RuleContext<'_>,
) -> Result<WalkingSurfaceServiceHandle, PropertyResolutionError> {
    context
        .services
        .get::<WalkingSurfaceServiceHandle>()
        .cloned()
        .ok_or_else(|| {
            PropertyResolutionError::MissingService(
                "the walking-surface service is not registered".into(),
            )
        })
}

fn refused(
    name: &str,
    object: &ObjectId,
    error: &axioval_engine::WalkingSurfaceError,
) -> PropertyResolutionError {
    let (reason, why) = service_error(error);
    crate::measured_kinds::resolution_error((reason, format!("`{name}` of {object}: {why}")))
}

/// The flight `object`, walked where `call` places its line.
fn flight(
    call: &MeasuredCall,
    object: &ObjectId,
    stairs: &WalkingSurfaceServiceHandle,
) -> Result<TreadFlight, PropertyResolutionError> {
    let request = match call.argument("walking_line_offset") {
        Some(MeasuredArgument::Length(offset)) => {
            TreadFlightRequest::from_inner_side(object.clone(), *offset)
        }
        _ => Ok(TreadFlightRequest::new(object.clone())),
    };
    request
        .and_then(|request| stairs.measure_tread_flight(&request))
        .map_err(|error| refused(call.name(), object, &error))
}

/// One member per riser: step `j` climbs riser `j` onto tread `j`, whose
/// going, nosing and winder angle are measured from tread `j - 1`.
fn steps(flight: &TreadFlight, object: &ObjectId) -> Vec<MeasuredMember> {
    let risers = flight.risers();
    let goings = flight.goings();
    let nosings = flight.nosings();
    let winders = flight.winder_angles();
    let closures = flight.riser_closures();
    let total = risers.len();
    (0..total)
        .map(|index| {
            let at = |field: &str| format!("steps:{object}#{}/{total}:{field}", index + 1);
            let before = index.checked_sub(1);
            let going = before.and_then(|before| goings.get(before).copied());
            let mut fields = BTreeMap::new();
            fields.insert("riser", number(Some(risers[index]), LENGTH, at("riser")));
            fields.insert("going", number(going, LENGTH, at("going")));
            fields.insert(
                "step_length",
                number(
                    going.and_then(|going| step_length(risers[index], going)),
                    LENGTH,
                    at("step_length"),
                ),
            );
            fields.insert(
                "nosing",
                number(
                    before.and_then(|before| nosings.get(before).copied()),
                    LENGTH,
                    at("nosing"),
                ),
            );
            fields.insert(
                "winder_angle",
                match before.map(|before| winders.get(before).copied()) {
                    Some(Some(None)) => MemberValue::Undecided {
                        why: format!("winder angle {index} of {total} is not measured"),
                    },
                    Some(Some(Some(angle))) => number(
                        Some(angle),
                        Some(QuantityDimension::PlaneAngle),
                        at("winder_angle"),
                    ),
                    _ => number(None, None, at("winder_angle")),
                },
            );
            fields.insert(
                "open_riser",
                match closures.get(index) {
                    Some(RiserClosure::Closed) => MemberValue::Truth {
                        value: false,
                        locator: at("open_riser"),
                    },
                    Some(RiserClosure::Open) => MemberValue::Truth {
                        value: true,
                        locator: at("open_riser"),
                    },
                    _ => MemberValue::Undecided {
                        why: format!(
                            "whether riser {} of {total} is closed is not measured",
                            index + 1
                        ),
                    },
                },
            );
            MeasuredMember {
                certain: true,
                fields,
            }
        })
        .collect()
}

fn length(call: &MeasuredCall, key: &str) -> f64 {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => *value,
        _ => 0.0,
    }
}

/// Where a rail lies among the pieces along its side, as its index and
/// their number (`None` over the middle), and the gap to the next piece.
fn position(
    measured: &HandrailEvidence,
    (rail, measurement): (&ObjectId, &RailMeasurement),
    locator: &str,
) -> (Result<Option<(usize, usize)>, String>, MemberValue) {
    let Some(side) = measured.side(measurement) else {
        return (Ok(None), number(None, LENGTH, locator.to_owned()));
    };
    let Ok(pieces) = measured.side_rail(side) else {
        let why = format!(
            "the pieces along the side of {rail} lie beside one another, not one after another"
        );
        return (Err(why.clone()), MemberValue::Undecided { why });
    };
    let Some(index) = pieces.iter().position(|(piece, _)| piece == rail) else {
        return (Ok(None), number(None, LENGTH, locator.to_owned()));
    };
    let gap = match pieces.get(index + 1) {
        None => number(None, LENGTH, locator.to_owned()),
        Some((_, next)) => match measured.gap(measurement, next) {
            Some(gap) => number(Some(gap), LENGTH, locator.to_owned()),
            None => MemberValue::Undecided {
                why: format!("the gap after {rail} is not measured"),
            },
        },
    };
    (Ok(Some((index, pieces.len()))), gap)
}

/// One member per rail along `stretch`, numbered `run`.
fn rails_along(
    measured: &HandrailEvidence,
    risers: Option<&RiserOffsets>,
    (object, run): (&ObjectId, usize),
) -> Vec<MeasuredMember> {
    let mut members = Vec::new();
    for (rail, measurement) in measured.rails() {
        let at = |field: &str| format!("handrails:{object}#{run}:{rail}:{field}");
        let side = measured.side(measurement);
        let truth = |value: bool, field: &str| MemberValue::Truth {
            value,
            locator: at(field),
        };
        let (place, gap) = position(measured, (rail, measurement), &at("gap_after"));
        let placed = |test: &dyn Fn(usize, usize) -> bool, field: &str| match &place {
            Ok(Some((index, total))) => truth(test(*index, *total), field),
            Ok(None) => truth(false, field),
            Err(why) => MemberValue::Undecided { why: why.clone() },
        };
        let extension =
            |reach: Option<MeasuredInterval>, bottom: bool, field: &str| match (reach, risers) {
                (None, _) => number(None, LENGTH, at(field)),
                (Some(reach), None) => number(Some(reach), LENGTH, at(field)),
                (Some(reach), Some(risers)) => match risers.shift(reach, bottom) {
                    Ok(reach) => number(Some(reach), LENGTH, at(field)),
                    Err(why) => MemberValue::Undecided { why },
                },
            };
        let rise = |rise: Option<MeasuredInterval>, field: &str| match rise {
            Some(rise) => number(Some(rise), LENGTH, at(field)),
            None => MemberValue::Undecided {
                why: format!("whether {rail} runs level is not measured"),
            },
        };
        #[allow(clippy::cast_precision_loss)]
        let numbered = run as f64;
        let fields = BTreeMap::from([
            (
                "run",
                MemberValue::Measured(Measurement::Value {
                    lower: numbered,
                    upper: numbered,
                    dimension: None,
                    locator: at("run"),
                }),
            ),
            ("left", truth(side == Some(RailSide::Left), "left")),
            ("right", truth(side == Some(RailSide::Right), "right")),
            (
                "height_lowest",
                number(Some(measurement.lowest()), LENGTH, at("height_lowest")),
            ),
            (
                "height_highest",
                number(Some(measurement.highest()), LENGTH, at("height_highest")),
            ),
            (
                "extension_bottom",
                extension(
                    measured.bottom_extension(measurement),
                    true,
                    "extension_bottom",
                ),
            ),
            (
                "extension_top",
                extension(measured.top_extension(measurement), false, "extension_top"),
            ),
            (
                "bottom_rise",
                rise(measurement.bottom_rise(), "bottom_rise"),
            ),
            ("top_rise", rise(measurement.top_rise(), "top_rise")),
            (
                "first_on_side",
                placed(&|index, _| index == 0, "first_on_side"),
            ),
            (
                "last_on_side",
                placed(&|index, total| index + 1 == total, "last_on_side"),
            ),
            ("gap_after", gap),
        ]);
        members.push(MeasuredMember {
            certain: true,
            fields,
        });
    }
    members
}

/// The handrails `call` lists along `object`.
fn handrails(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    stairs: &WalkingSurfaceServiceHandle,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let name = call.name();
    let rails = crate::measured_kinds::objects_of_kinds(context, call, "rails", object)?;
    let reach = (length(call, "reach_across"), length(call, "reach_above"));
    let level_over = length(call, "level_over");
    let (stretches, risers) = if call.choice("of") == Some("ramp") {
        let runs = stairs
            .measure_sloped_runs(object)
            .map_err(|error| refused(name, object, &error))?;
        let stretches: Vec<WalkingStretch> =
            (0..runs.runs().len()).map(WalkingStretch::Run).collect();
        (stretches, None)
    } else {
        let flight = flight(call, object, stairs)?;
        let risers = (call.choice("from") == Some("riser")).then(|| RiserOffsets::of(&flight));
        (vec![WalkingStretch::Flight], risers)
    };
    let mut members = Vec::new();
    for (index, stretch) in stretches.into_iter().enumerate() {
        let request = HandrailRequest::try_new(
            object.clone(),
            stretch,
            rails.iter().cloned(),
            reach,
            level_over,
        )
        .map_err(|error| refused(name, object, &error))?;
        let measured = stairs
            .measure_handrails(&request)
            .map_err(|error| refused(name, object, &error))?;
        members.extend(rails_along(&measured, risers.as_ref(), (object, index + 1)));
    }
    Ok(members)
}

impl StairMeasures {
    fn landing(
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let name = call.name();
        let stairs = walking(context)?;
        let candidates = crate::measured_kinds::objects_of_kinds(context, call, "landing", object)?;
        let top = call.choice("end") == Some("top");
        let end = if call.choice("of") == Some("ramp") {
            let runs = stairs
                .measure_sloped_runs(object)
                .map_err(|error| refused(name, object, &error))?;
            let last = runs.runs().len().saturating_sub(1);
            if top {
                WalkingEnd::RunTop(last)
            } else {
                WalkingEnd::RunBottom(0)
            }
        } else {
            // A landing is placed at the end of a flight measured first, as
            // `stair-geometry` measures it.
            flight(call, object, &stairs)?;
            if top {
                WalkingEnd::FlightTop
            } else {
                WalkingEnd::FlightBottom
            }
        };
        let request = LandingRequest::new(object.clone(), end, candidates);
        let measured = stairs
            .measure_landing(&request)
            .map_err(|error| refused(name, object, &error))?;
        let place = if top { "top" } else { "bottom" };
        let locator = format!("{name}:{object}:{place}");
        let Some(landing) = measured.landing() else {
            return Ok(Measurement::Absent {
                locator: format!("{locator}: no selected object carries a landing there"),
            });
        };
        let size = if name == "landing_depth" {
            measured.depth()
        } else {
            measured.width()
        };
        let size = size.ok_or_else(|| {
            PropertyResolutionError::Incomplete(format!(
                "`{name}` of {object}: the landing {} at its {place} fills no rectangle along \
                 the walking direction",
                landing.carrier()
            ))
        })?;
        Ok(value(
            size,
            LENGTH,
            format!("{locator}:{}", landing.carrier()),
        ))
    }
}

impl MeasuredProvider for StairMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            "flight_rise",
            "flight_width",
            "landing_depth",
            "landing_width",
        ]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &["handrails", "runs", "steps"]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let name = call.name();
        if name.starts_with("landing_") {
            return Self::landing(call, object, context);
        }
        let flight = flight(call, object, &walking(context)?)?;
        let locator = format!("{name}:{object}");
        if name == "flight_rise" {
            return Ok(value(flight.rise(), LENGTH, locator));
        }
        let width = flight.width().ok_or_else(|| {
            PropertyResolutionError::Incomplete(format!(
                "`{name}` of {object}: a tread fills no rectangle along the flight, so its \
                 width is not measured"
            ))
        })?;
        Ok(value(width, LENGTH, locator))
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let stairs = walking(context)?;
        if call.name() == "handrails" {
            return handrails(call, object, context, &stairs);
        }
        if call.name() == "steps" {
            return Ok(steps(&flight(call, object, &stairs)?, object));
        }
        let measured = stairs
            .measure_sloped_runs(object)
            .map_err(|error| refused(call.name(), object, &error))?;
        let total = measured.runs().len();
        Ok(measured
            .runs()
            .iter()
            .enumerate()
            .map(|(index, run)| {
                let at = |field: &str| format!("runs:{object}#{}/{total}:{field}", index + 1);
                let fields = BTreeMap::from([
                    ("slope", number(Some(run.slope()), None, at("slope"))),
                    ("length", number(Some(run.length()), LENGTH, at("length"))),
                    ("rise", number(Some(run.rise()), LENGTH, at("rise"))),
                    ("width", number(run.width(), LENGTH, at("width"))),
                ]);
                MeasuredMember {
                    certain: true,
                    fields,
                }
            })
            .collect())
    }
}
