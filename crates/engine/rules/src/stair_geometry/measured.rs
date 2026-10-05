//! Stair and ramp measurements as values, measured through the
//! walking-surface service exactly as `stair-geometry` and `ramp-geometry`
//! measure them: a flight's steps and a ramp's runs as members, a flight's
//! rise and width, and the landing at either end.

use std::collections::BTreeMap;

use axioval_engine::{
    ElevationInterval, HandrailEvidence, HandrailRequest, LandingRequest, MeasuredInterval,
    MeasuredMember, MeasuredProvider, Measurement, MemberValue, PropertyResolutionError,
    RailMeasurement, RailSide, RiserClosure, RuleContext, TreadFlight, TreadFlightRequest,
    WalkingEnd, WalkingStretch, WalkingSurfaceServiceHandle,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

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

/// A value of a measurement whose evidence is `exact`: rounded when so.
fn measured(interval: MeasuredInterval, exact: bool, locator: String) -> Measurement {
    if exact {
        Measurement::Rounded {
            lower: interval.lower(),
            upper: interval.upper(),
            dimension: LENGTH,
            locator,
        }
    } else {
        Measurement::Cited {
            lower: interval.lower(),
            upper: interval.upper(),
            dimension: LENGTH,
            locator,
            exact: false,
        }
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
                exact: flight.evidence().exact,
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

/// Why the side of `rail` is unknown: it may reach over the middle, or lie
/// wholly in either half.
fn unplaced(rail: &ObjectId) -> String {
    format!("whether {rail} runs along a side or over the middle is undecided")
}

/// Where a rail lies among the pieces along its side, as its index and
/// their number (`None` surely over the middle), and the gap to the next
/// piece. A rail that cannot be placed is undecided, never on no side.
fn position(
    measured: &HandrailEvidence,
    (rail, measurement): (&ObjectId, &RailMeasurement),
    locator: &str,
) -> (Result<Option<(usize, usize)>, String>, MemberValue) {
    let Some(side) = measured.side(measurement) else {
        if measured.over_middle(measurement) {
            return (Ok(None), number(None, LENGTH, locator.to_owned()));
        }
        let why = unplaced(rail);
        return (Err(why.clone()), MemberValue::Undecided { why });
    };
    let Ok(pieces) = measured.side_rail(side) else {
        let why = format!(
            "the pieces along the side of {rail} lie beside one another, not one after another"
        );
        return (Err(why.clone()), MemberValue::Undecided { why });
    };
    let Some(index) = pieces.iter().position(|(piece, _)| piece == rail) else {
        let why = format!("{rail} is not among the pieces along its own side");
        return (Err(why.clone()), MemberValue::Undecided { why });
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
        let over_middle = side.is_none() && measured.over_middle(measurement);
        let truth = |value: bool, field: &str| MemberValue::Truth {
            value,
            locator: at(field),
        };
        // A rail on a side is on that side only; one surely over the middle
        // is on neither; any other is undecided, never on neither.
        let on = |wanted: RailSide, field: &str| match side {
            Some(side) => truth(side == wanted, field),
            None if over_middle => truth(false, field),
            None => MemberValue::Undecided {
                why: unplaced(rail),
            },
        };
        let (place, gap) = position(measured, (rail, measurement), &at("gap_after"));
        let placed = |test: &dyn Fn(usize, usize) -> bool, field: &str| match &place {
            Ok(Some((index, total))) => truth(test(*index, *total), field),
            // Surely over the middle: no piece of either side.
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
            ("left", on(RailSide::Left, "left")),
            ("right", on(RailSide::Right, "right")),
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
            exact: measured.evidence().exact,
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
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
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
    let mut cited = Vec::new();
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
        cited.push(measured.evidence().clone());
    }
    Ok((members, cited))
}

/// The landing at `end` among `candidates`: whether there is one, its depth
/// and width (none without one, undecided without a rectangle), and
/// whether its evidence is exact.
fn end_landing(
    stairs: &WalkingSurfaceServiceHandle,
    object: &ObjectId,
    end: WalkingEnd,
    candidates: impl Iterator<Item = ObjectId>,
) -> Result<([MemberValue; 3], bool), String> {
    let request = LandingRequest::new(object.clone(), end, candidates);
    let measured = stairs
        .measure_landing(&request)
        .map_err(|error| service_error(&error).1)?;
    let exact = measured.evidence().exact;
    let locator = measured.evidence().locator.clone();
    let Some(landing) = measured.landing() else {
        return Ok((
            [
                MemberValue::Truth {
                    value: false,
                    locator: locator.clone(),
                },
                number(None, LENGTH, locator.clone()),
                number(None, LENGTH, locator),
            ],
            exact,
        ));
    };
    let size = |size: Option<MeasuredInterval>| match size {
        Some(size) => number(Some(size), LENGTH, locator.clone()),
        None => MemberValue::Undecided {
            why: format!(
                "the landing {} fills no rectangle along the walking direction",
                landing.carrier()
            ),
        },
    };
    Ok((
        [
            MemberValue::Truth {
                value: true,
                locator: locator.clone(),
            },
            size(measured.depth()),
            size(measured.width()),
        ],
        exact,
    ))
}

/// The clear width of the landing at one end of a flight, as
/// `stair-geometry` measures it; none when no landing meets it.
fn landing_clear_width(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, PropertyResolutionError> {
    let name = call.name();
    let stairs = walking(context)?;
    let obstacles = crate::measured_kinds::objects_of_kinds(context, call, "obstacles", object)?;
    let carriers = crate::measured_kinds::objects_of_kinds(context, call, "landing", object)?;
    let selector = axioval_ir::contract::Selector::Objects {
        objects: obstacles.clone(),
    };
    let band = (length(call, "band_from"), length(call, "band_to"));
    let check = super::clear_width::ClearWidthCheck::measuring(&selector, band);
    let landings: super::Selected = Ok((carriers.into_iter().collect(), false));
    let top = call.choice("end") == Some("top");
    let candidates: Vec<ObjectId> = obstacles.into_iter().collect();
    let locator = format!("{name}:{object}:{}", if top { "top" } else { "bottom" });
    match super::clear_width::landing_width(&stairs, &check, &candidates, &landings, (object, top))
    {
        None => Ok(Measurement::Absent {
            locator: format!("{locator}: no selected object carries a landing there"),
        }),
        Some(width) => {
            let (width, exact) = width.interval().map_err(|why| {
                PropertyResolutionError::Incomplete(format!("`{name}` of {object}: {why}"))
            })?;
            Ok(measured(width, exact, locator))
        }
    }
}

/// A whole stair's rise, from its lowest flight's base to its highest
/// flight's top, as `stair-geometry` measures it.
fn stair_rise(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Measurement, PropertyResolutionError> {
    let name = call.name();
    let stairs = walking(context)?;
    let Some(MeasuredArgument::Path(steps)) = call.argument("stair") else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let everything: Vec<&axioval_ir::Object> = context.project.objects().collect();
    let (parts, _) = crate::support::Traversal::path(steps)
        .and_then(|path| path.related(context, object, &everything))
        .map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{name}` of {object}: {why}"),
            ))
        })?;
    let kinds = crate::measured_kinds::objects_of_kinds(context, call, "flights", object)?;
    let mut flights = Vec::new();
    for part in parts.iter().filter(|part| kinds.contains(*part)) {
        let request = TreadFlightRequest::new(part.clone());
        flights.push(
            stairs
                .measure_tread_flight(&request)
                .map_err(|error| refused(name, part, &error))?,
        );
    }
    let (Some(lowest), Some(highest)) = (
        flights.iter().map(TreadFlight::base).reduce(|a, b| {
            ElevationInterval::try_new(
                a.lower_metres().min(b.lower_metres()),
                a.upper_metres().min(b.upper_metres()),
            )
            .unwrap_or(a)
        }),
        flights.iter().map(TreadFlight::top).reduce(|a, b| {
            ElevationInterval::try_new(
                a.lower_metres().max(b.lower_metres()),
                a.upper_metres().max(b.upper_metres()),
            )
            .unwrap_or(a)
        }),
    ) else {
        return Err(PropertyResolutionError::Incomplete(format!(
            "`{name}` of {object}: the path reaches no flight"
        )));
    };
    let rise = MeasuredInterval::try_new(
        (highest.lower_metres() - lowest.upper_metres()).next_down(),
        (highest.upper_metres() - lowest.lower_metres()).next_up(),
    )
    .map_err(|_| PropertyResolutionError::InvalidValue)?;
    let exact = flights.iter().all(|flight| flight.evidence().exact);
    Ok(measured(rise, exact, format!("{name}:{object}")))
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
        let measured_landing = stairs
            .measure_landing(&request)
            .map_err(|error| refused(name, object, &error))?;
        let place = if top { "top" } else { "bottom" };
        let locator = format!("{name}:{object}:{place}");
        if name == "landing_count" {
            let found = f64::from(u8::from(measured_landing.landing().is_some()));
            return Ok(Measurement::Value {
                lower: found,
                upper: found,
                dimension: None,
                locator,
            });
        }
        let Some(landing) = measured_landing.landing() else {
            return Ok(Measurement::Absent {
                locator: format!("{locator}: no selected object carries a landing there"),
            });
        };
        let size = if name == "landing_depth" {
            measured_landing.depth()
        } else {
            measured_landing.width()
        };
        let size = size.ok_or_else(|| {
            PropertyResolutionError::Incomplete(format!(
                "`{name}` of {object}: the landing {} at its {place} fills no rectangle along \
                 the walking direction",
                landing.carrier()
            ))
        })?;
        Ok(measured(
            size,
            measured_landing.evidence().exact,
            format!("{locator}:{}", landing.carrier()),
        ))
    }
}

impl MeasuredProvider for StairMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[
            "end_width",
            "flight_rise",
            "flight_width",
            "handrail_breaks",
            "landing_clear_width",
            "landing_count",
            "landing_depth",
            "landing_door_conflicts",
            "landing_width",
            "missing_tactile_strips",
            "obstructed_end_spaces",
            "rails_over_surfaces",
            "stair_rise",
            "walking_line_turns",
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
        if name == "landing_clear_width" {
            return landing_clear_width(call, object, context);
        }
        if name == "stair_rise" {
            return stair_rise(call, object, context);
        }
        if super::defects::NAMES.contains(&name) {
            return super::defects::count(call, object, context);
        }
        if name.starts_with("landing_") || name == "landing_count" {
            return Self::landing(call, object, context);
        }
        let flight = flight(call, object, &walking(context)?)?;
        let locator = format!("{name}:{object}");
        if name == "end_width" {
            // A turning flight's winders have no width: a landing at its end
            // is compared with the tread meeting it.
            let width = if flight.walking_line().is_turning() {
                let tread = if call.choice("end") == Some("top") {
                    flight.treads().last()
                } else {
                    flight.treads().first()
                };
                tread.and_then(axioval_engine::Tread::width)
            } else {
                flight.width()
            };
            let width = width.ok_or_else(|| {
                PropertyResolutionError::Incomplete(format!(
                    "`{name}` of {object}: the width at that end is not measured"
                ))
            })?;
            return Ok(measured(width, flight.evidence().exact, locator));
        }
        if name == "walking_line_turns" {
            let turns = f64::from(u8::from(flight.walking_line().is_turning()));
            return Ok(Measurement::Value {
                lower: turns,
                upper: turns,
                dimension: None,
                locator,
            });
        }
        if name == "flight_rise" {
            return Ok(measured(flight.rise(), flight.evidence().exact, locator));
        }
        let width = flight.width().ok_or_else(|| {
            PropertyResolutionError::Incomplete(format!(
                "`{name}` of {object}: a tread fills no rectangle along the flight, so its \
                 width is not measured"
            ))
        })?;
        Ok(measured(width, flight.evidence().exact, locator))
    }

    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let stairs = walking(context)?;
        if call.name() == "handrails" {
            return handrails(call, object, context, &stairs);
        }
        if call.name() == "steps" {
            let flight = flight(call, object, &stairs)?;
            return Ok((steps(&flight, object), vec![flight.evidence().clone()]));
        }
        let measured = stairs
            .measure_sloped_runs(object)
            .map_err(|error| refused(call.name(), object, &error))?;
        let candidates = if call.argument("landing").is_some() {
            Some(crate::measured_kinds::objects_of_kinds(
                context, call, "landing", object,
            )?)
        } else {
            None
        };
        let total = measured.runs().len();
        let mut members = Vec::new();
        for (index, run) in measured.runs().iter().enumerate() {
            let at = |field: &str| format!("runs:{object}#{}/{total}:{field}", index + 1);
            #[allow(clippy::cast_precision_loss)]
            let numbered = (index + 1) as f64;
            let mut fields = BTreeMap::from([
                (
                    "run",
                    MemberValue::Measured(Measurement::Value {
                        lower: numbered,
                        upper: numbered,
                        dimension: None,
                        locator: at("run"),
                    }),
                ),
                ("slope", number(Some(run.slope()), None, at("slope"))),
                ("length", number(Some(run.length()), LENGTH, at("length"))),
                ("rise", number(Some(run.rise()), LENGTH, at("rise"))),
                ("width", number(run.width(), LENGTH, at("width"))),
            ]);
            let mut exact = measured.evidence().exact;
            for (end, place) in [
                (WalkingEnd::RunBottom(index), "bottom"),
                (WalkingEnd::RunTop(index), "top"),
            ] {
                let landing = candidates.as_ref().map(|candidates| {
                    end_landing(&stairs, object, end, candidates.iter().cloned())
                });
                let [present, depth, width] = match landing {
                    // Without landing kinds nothing is known of the landing:
                    // neither whether there is one nor its size, never none.
                    None => {
                        let undecided = || MemberValue::Undecided {
                            why: "the runs list states no `landing` kinds".into(),
                        };
                        [undecided(), undecided(), undecided()]
                    }
                    Some(Ok((fields, cited))) => {
                        exact &= cited;
                        fields
                    }
                    Some(Err(why)) => {
                        let undecided = || MemberValue::Undecided { why: why.clone() };
                        [undecided(), undecided(), undecided()]
                    }
                };
                fields.insert(
                    if place == "bottom" {
                        "bottom_landing"
                    } else {
                        "top_landing"
                    },
                    present,
                );
                fields.insert(
                    if place == "bottom" {
                        "bottom_landing_depth"
                    } else {
                        "top_landing_depth"
                    },
                    depth,
                );
                fields.insert(
                    if place == "bottom" {
                        "bottom_landing_width"
                    } else {
                        "top_landing_width"
                    },
                    width,
                );
            }
            members.push(MeasuredMember {
                certain: true,
                exact,
                fields,
            });
        }
        Ok((members, vec![measured.evidence().clone()]))
    }
}
