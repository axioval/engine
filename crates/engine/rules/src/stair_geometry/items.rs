//! Stair and ramp items as measured member lists, which the
//! `stair-geometry` and `ramp-geometry` templates judge one by one: the
//! clearance above or below, the landing at each end, the doors on it and
//! swinging over it, the free space at each end, the clear widths, the
//! handrails (each stretch, rail, end extension and gap between pieces),
//! the handrail across a ramp's landings, the rails over accessible
//! surfaces and the tactile strips.
//!
//! The numbers are measured here and judged by the template against the
//! rule's parameters. A search (whether something obstructs an end space,
//! stands on a landing, swings over it, breaks a handrail, reaches over a
//! surface, or covers a strip) answers one three-valued result per searched
//! item: `found` true with the words of what it found (`finding`) and the
//! objects involved (`objects`), false, or undecided with why. A search
//! over a selection that leaves objects undecided treats them as possible
//! finds, as the capabilities always did.

use std::collections::BTreeMap;

use axioval_engine::{
    ClearanceBelowRequest, FreeSpaceServiceHandle, HandrailEvidence, HeadroomRequest,
    LandingEvidence, LandingRequest, MeasuredInterval, MeasuredMember, MeasuredMemo,
    MeasuredProvider, Measurement, MemberValue, PropertyResolutionError, ProximityServiceHandle,
    RailSide, RuleContext, SlopedSurface, TreadFlight, TreadFlightRequest, WalkingEnd,
    WalkingStretch, WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, ObjectId, QuantityDimension};

use super::handrails::{self, Along, HandrailCheck, RiserOffsets};
use super::{
    Check, Checks, Selected, clear_width, continuity, landing_level, landing_scale, obstruction,
    ramp_ends, service_error,
};

/// Measures the stair and ramp items: the number of a ramp's runs, and the
/// item lists.
pub(crate) struct StairItems;

impl MeasuredProvider for StairItems {
    fn names(&self) -> &'static [&'static str] {
        &["run_count"]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        LISTS
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        let name = call.name();
        let runs = runs(context, object).map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((
                reason,
                format!("`{name}` of {object}: {why}"),
            ))
        })?;
        #[allow(clippy::cast_precision_loss)]
        let count = runs.runs().len() as f64;
        // Cited as the service measured the runs.
        Ok(crate::measured_kinds::interval(
            (count, count),
            None,
            runs.evidence().exact,
            runs.evidence().locator.clone(),
        ))
    }

    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        members(call, object, context)
    }
}

/// The member lists measured here.
pub(super) const LISTS: &[&str] = &[
    "clear_widths",
    "clearances",
    "end_spaces",
    "flights",
    "handrail_stretches",
    "landing_doors",
    "landing_swings",
    "landings",
    "rail_continuity",
    "rail_extensions",
    "rail_gaps",
    "rail_heights",
    "rail_obstructions",
    "stair_clear_widths",
    "stair_continuity",
    "stairs",
    "tactile_strips",
];

const LENGTH: Option<QuantityDimension> = Some(QuantityDimension::Length);

/// A number field, `null` where there is none.
fn number(value: Option<MeasuredInterval>, locator: String) -> MemberValue {
    MemberValue::Measured(match value {
        Some(value) => Measurement::Value {
            lower: value.lower(),
            upper: value.upper(),
            dimension: LENGTH,
            locator,
        },
        None => Measurement::Absent { locator },
    })
}

/// A plain number field.
fn plain(value: f64, locator: String) -> MemberValue {
    MemberValue::Measured(Measurement::Value {
        lower: value,
        upper: value,
        dimension: None,
        locator,
    })
}

fn text(words: impl Into<String>) -> MemberValue {
    MemberValue::Text { text: words.into() }
}

fn truth(value: bool, locator: String) -> MemberValue {
    MemberValue::Truth { value, locator }
}

fn undecided(why: impl Into<String>) -> MemberValue {
    MemberValue::Undecided { why: why.into() }
}

fn objects(objects: &[ObjectId]) -> MemberValue {
    let mut objects = objects.to_vec();
    objects.dedup();
    MemberValue::Objects { objects }
}

fn member(exact: bool, fields: BTreeMap<&'static str, MemberValue>) -> MeasuredMember {
    MeasuredMember {
        certain: true,
        exact,
        fields,
    }
}

fn refused(call: &MeasuredCall, object: &ObjectId, why: &str) -> PropertyResolutionError {
    crate::measured_kinds::resolution_error((
        axioval_ir::NotEvaluatedReason::IncompleteEvidence,
        format!("`{}` of {object}: {why}", call.name()),
    ))
}

pub(super) fn walking(
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

fn length(call: &MeasuredCall, key: &str) -> Option<f64> {
    match call.argument(key) {
        Some(MeasuredArgument::Length(value)) => Some(*value),
        _ => None,
    }
}

/// The objects a selection argument picks surely, and whether it leaves
/// any undecided, as a capability sends them with its requests; `None`
/// where the call names none.
fn selection(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<Option<(Vec<ObjectId>, bool)>, PropertyResolutionError> {
    Ok(
        crate::measured_kinds::selection(context, call, key, None)?.map(|selection| {
            let undecided = !selection.undecided.is_empty();
            (selection.matched.into_iter().collect(), undecided)
        }),
    )
}

/// A selection the call must name.
fn selected(
    context: &RuleContext<'_>,
    call: &MeasuredCall,
    key: &str,
) -> Result<(Vec<ObjectId>, bool), PropertyResolutionError> {
    selection(context, call, key)?.ok_or(PropertyResolutionError::InvalidRequest)
}

/// The selector naming exactly `objects`, for the shared checks' requests.
fn naming(objects: &[ObjectId]) -> Selector {
    Selector::Objects {
        objects: objects.iter().cloned().collect(),
    }
}

/// A key memoizing the flight measured with a walking line.
#[derive(Clone, PartialEq, Eq, Hash)]
struct FlightKey(ObjectId, Option<u64>);

/// The flight `object`, walked where `offset` places its line, measured
/// once per run.
pub(super) fn flight(
    context: &RuleContext<'_>,
    object: &ObjectId,
    offset: Option<f64>,
) -> Result<TreadFlight, (axioval_ir::NotEvaluatedReason, String)> {
    let stairs = context
        .services
        .get::<WalkingSurfaceServiceHandle>()
        .cloned()
        .ok_or_else(|| {
            (
                axioval_ir::NotEvaluatedReason::MissingService,
                "the walking-surface service is not registered".to_owned(),
            )
        })?;
    MeasuredMemo::of(
        context.services,
        FlightKey(object.clone(), offset.map(f64::to_bits)),
        || {
            let request = match offset {
                None => Ok(TreadFlightRequest::new(object.clone())),
                Some(offset) => TreadFlightRequest::from_inner_side(object.clone(), offset),
            };
            request
                .and_then(|request| stairs.measure_tread_flight(&request))
                .map_err(|error| service_error(&error))
        },
    )
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RunsKey(ObjectId);

/// The ramp `object`'s runs, measured once per run.
pub(super) fn runs(
    context: &RuleContext<'_>,
    object: &ObjectId,
) -> Result<SlopedSurface, (axioval_ir::NotEvaluatedReason, String)> {
    let stairs = context
        .services
        .get::<WalkingSurfaceServiceHandle>()
        .cloned()
        .ok_or_else(|| {
            (
                axioval_ir::NotEvaluatedReason::MissingService,
                "the walking-surface service is not registered".to_owned(),
            )
        })?;
    MeasuredMemo::of(context.services, RunsKey(object.clone()), || {
        stairs
            .measure_sloped_runs(object)
            .map_err(|error| service_error(&error))
    })
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct LandingKey(ObjectId, u8, usize, Vec<ObjectId>);

/// The landing at `end` among `candidates`, measured once per run.
fn landing(
    context: &RuleContext<'_>,
    stairs: &WalkingSurfaceServiceHandle,
    object: &ObjectId,
    end: WalkingEnd,
    candidates: &[ObjectId],
) -> Result<LandingEvidence, String> {
    let (which, index) = match end {
        WalkingEnd::FlightBottom => (0, 0),
        WalkingEnd::FlightTop => (1, 0),
        WalkingEnd::RunBottom(index) => (2, index),
        WalkingEnd::RunTop(index) => (3, index),
    };
    MeasuredMemo::of(
        context.services,
        LandingKey(object.clone(), which, index, candidates.to_vec()),
        || {
            let request = LandingRequest::new(object.clone(), end, candidates.iter().cloned());
            stairs
                .measure_landing(&request)
                .map_err(|error| service_error(&error).1)
        },
    )
}

/// Whether the call measures a ramp's runs rather than a flight.
fn of_ramp(call: &MeasuredCall) -> bool {
    call.choice("of") == Some("ramp")
}

/// The flight or ramp a call measures.
pub(super) enum Walked {
    Flight(TreadFlight),
    Ramp(SlopedSurface),
}

pub(super) fn walked(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Walked, PropertyResolutionError> {
    let refusal = |(reason, why): (axioval_ir::NotEvaluatedReason, String)| {
        crate::measured_kinds::resolution_error((
            reason,
            format!("`{}` of {object}: {why}", call.name()),
        ))
    };
    if of_ramp(call) {
        runs(context, object).map(Walked::Ramp).map_err(refusal)
    } else {
        flight(context, object, length(call, "walking_line_offset"))
            .map(Walked::Flight)
            .map_err(refusal)
    }
}

/// One end whose landing is measured: where, how messages name it, the
/// width of what meets it, whether it is one of a ramp's two outermost
/// ends, and the level it lies at.
struct End {
    which: WalkingEnd,
    label: String,
    noun: &'static str,
    walking: Option<MeasuredInterval>,
    outermost: bool,
    level: axioval_engine::ElevationInterval,
}

fn ends(walked: &Walked) -> Vec<End> {
    match walked {
        Walked::Flight(flight) => [
            (WalkingEnd::FlightBottom, "the bottom of the flight"),
            (WalkingEnd::FlightTop, "the top of the flight"),
        ]
        .into_iter()
        .map(|(which, label)| {
            // A turning flight's winders have no width: its landing is
            // compared with the tread that meets it.
            let walking = if flight.walking_line().is_turning() {
                let tread = match which {
                    WalkingEnd::FlightBottom => flight.treads().first(),
                    _ => flight.treads().last(),
                };
                tread.and_then(axioval_engine::Tread::width)
            } else {
                flight.width()
            };
            End {
                which,
                label: label.to_owned(),
                noun: "flight",
                walking,
                outermost: false,
                level: landing_level(flight, which),
            }
        })
        .collect(),
        Walked::Ramp(surface) => {
            let runs = surface.runs();
            let total = runs.len();
            let mut ends = Vec::new();
            for (index, run) in runs.iter().enumerate() {
                for (which, place, level) in [
                    (WalkingEnd::RunBottom(index), "bottom", run.bottom()),
                    (WalkingEnd::RunTop(index), "top", run.top()),
                ] {
                    ends.push(End {
                        which,
                        label: format!("the {place} of run {} of {total}", index + 1),
                        noun: "run",
                        walking: run.width(),
                        outermost: (index == 0 && place == "bottom")
                            || (index + 1 == total && place == "top"),
                        level,
                    });
                }
            }
            ends
        }
    }
}

/// A search's answer as an item: `found` true with what it found and the
/// objects involved, false, or undecided with why; exact where every piece
/// of its evidence is.
fn searched(
    (check, evidence, related): (Check, Vec<Evidence>, Vec<ObjectId>),
    label: &str,
) -> MeasuredMember {
    let exact = evidence.iter().all(|evidence| evidence.exact);
    let (found, finding) = match check {
        Check::Pass => (truth(false, label.to_owned()), String::new()),
        Check::Fail(message) | Check::Graded(message, _) => {
            (truth(true, label.to_owned()), message)
        }
        Check::Undecided(why) => (undecided(why), String::new()),
    };
    member(
        exact,
        BTreeMap::from([
            ("found", found),
            ("finding", text(finding)),
            ("objects", objects(&related)),
            ("label", text(label)),
        ]),
    )
}

/// Measures the list `call` names of `object`.
pub(super) fn members(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
    let stairs = walking(context)?;
    // A whole stair's lists measure its flights, never the stair itself.
    match call.name() {
        "stairs" => return Ok((stairs_item(call, object, context)?, Vec::new())),
        "stair_continuity" => {
            return Ok((
                stair_continuity(call, object, context, &stairs)?,
                Vec::new(),
            ));
        }
        "stair_clear_widths" => {
            return Ok((
                stair_clear_widths(call, object, context, &stairs)?,
                Vec::new(),
            ));
        }
        _ => {}
    }
    let walked = walked(call, object, context)?;
    let members = match call.name() {
        "flights" => flights(object, &walked)?,
        "tactile_strips" => tactile_strips(call, object, context, (&stairs, &walked))?,
        "clearances" => clearances(call, object, context, &stairs)?,
        "landings" => landings(call, object, context, (&stairs, &walked))?,
        "landing_doors" => doors(call, object, context, (&stairs, &walked), false)?,
        "landing_swings" => doors(call, object, context, (&stairs, &walked), true)?,
        "end_spaces" => end_spaces(call, object, context, (&stairs, &walked))?,
        "clear_widths" => clear_widths(call, object, context, (&stairs, &walked))?,
        "handrail_stretches" | "rail_heights" | "rail_extensions" | "rail_gaps" => {
            rails(call, object, context, (&stairs, &walked))?
        }
        "rail_continuity" => ramp_rails(call, object, context, (&stairs, &walked), true)?,
        "rail_obstructions" => ramp_rails(call, object, context, (&stairs, &walked), false)?,
        _ => return Err(PropertyResolutionError::InvalidRequest),
    };
    Ok((members, Vec::new()))
}

/// The clearance above (`side=above`, to `obstacles`) or below (over the
/// floors of `obstacles`) the flight or ramp: one item, its clearance
/// (`null` where nothing governs it) and the objects governing it.
fn clearances(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    stairs: &WalkingSurfaceServiceHandle,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let (candidates, _) = selected(context, call, "obstacles")?;
    let noun = if of_ramp(call) { "ramp" } else { "flight" };
    let locator = format!("clearances:{object}");
    let (clearance, governing, exact) = if call.choice("side") == Some("below") {
        let request = ClearanceBelowRequest::new(object.clone(), candidates);
        let measured = stairs.measure_clearance_below(&request).map_err(|error| {
            refused(
                call,
                object,
                &format!("headroom below: {}", service_error(&error).1),
            )
        })?;
        (
            measured.clearance(),
            measured.governing().to_vec(),
            measured.evidence().exact,
        )
    } else {
        let request = HeadroomRequest::new(object.clone(), candidates);
        let measured = stairs.measure_headroom(&request).map_err(|error| {
            refused(
                call,
                object,
                &format!("headroom: {}", service_error(&error).1),
            )
        })?;
        (
            measured.clearance(),
            measured.governing().to_vec(),
            measured.evidence().exact,
        )
    };
    Ok(vec![member(
        exact,
        BTreeMap::from([
            ("clearance", number(clearance, locator)),
            ("governing", objects(&governing)),
            ("noun", text(noun)),
        ]),
    )])
}

/// The landing at each end of the flight, or of each of the ramp's runs.
fn landings(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let (candidates, _) = selected(context, call, "landing")?;
    let mut members = Vec::new();
    for end in ends(walked) {
        let label = end.label.clone();
        let locator = |field: &str| format!("landings:{object}:{label}:{field}");
        let mut fields = BTreeMap::from([
            ("label", text(label.clone())),
            ("noun", text(end.noun)),
            ("walking", number(end.walking, locator("walking"))),
            ("outermost", truth(end.outermost, locator("outermost"))),
        ]);
        let exact = match landing(context, stairs, object, end.which, &candidates) {
            Err(why) => {
                let why = format!("landing at {label}: {why}");
                fields.insert("present", undecided(why.clone()));
                for name in ["depth", "width", "scale"] {
                    fields.insert(name, number(None, locator(name)));
                }
                fields.insert("carrier", text(""));
                fields.insert("carriers", objects(&[]));
                true
            }
            Ok(measured) => {
                fields.insert(
                    "present",
                    truth(measured.landing().is_some(), locator("present")),
                );
                fields.insert("scale", plain(landing_scale(&measured), locator("scale")));
                match measured.landing() {
                    None => {
                        fields.insert("depth", number(None, locator("depth")));
                        fields.insert("width", number(None, locator("width")));
                        fields.insert("carrier", text(""));
                        fields.insert("carriers", objects(&[]));
                    }
                    Some(found) => {
                        let carrier = found.carrier();
                        let size = |size: Option<MeasuredInterval>, name: &str| match size {
                            Some(size) => number(Some(size), locator(name)),
                            None => undecided(format!(
                                "the landing {carrier} at {label} fills no rectangle along \
                                 the walking direction, so its size is not measured"
                            )),
                        };
                        let (depth, width) = (measured.depth(), measured.width());
                        let both = depth.is_some() && width.is_some();
                        fields.insert("depth", size(depth.filter(|_| both), "depth"));
                        fields.insert("width", size(width.filter(|_| both), "width"));
                        fields.insert("carrier", text(carrier.to_string()));
                        let related: Vec<ObjectId> = if carrier == object {
                            Vec::new()
                        } else {
                            vec![carrier.clone()]
                        };
                        fields.insert("carriers", objects(&related));
                    }
                }
                measured.evidence().exact
            }
        };
        members.push(member(exact, fields));
    }
    Ok(members)
}

/// Whether a selected door stands on (`landing_doors`) or swings over
/// (`landing_swings`) the landing at each end: a search for each landing
/// measured, none for an end whose landing could not be measured.
pub(super) fn doors(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
    swing: bool,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let (candidates, _) = selected(context, call, "landing")?;
    let (doors, undecided_doors) = selected(context, call, "doors")?;
    let height = length(call, "height").ok_or(PropertyResolutionError::InvalidRequest)?;
    let selector = naming(&doors);
    let check = ramp_ends::DoorCheck::new(&selector, height);
    let free = context.services.get::<FreeSpaceServiceHandle>();
    let swings = swing.then(|| ramp_ends::door_swings(context, &doors));
    let chosen: Selected = Ok((doors.clone(), undecided_doors));
    let mut members = Vec::new();
    for end in ends(walked) {
        let Ok(measured) = landing(context, stairs, object, end.which, &candidates) else {
            continue;
        };
        let (found, mut evidence, related) = match &swings {
            None => ramp_ends::doors(free, &check, &chosen, &measured, end.level, &end.label),
            Some(swings) => ramp_ends::door_swings_over(
                &check,
                swings,
                undecided_doors,
                &measured,
                end.level,
                &end.label,
            ),
        };
        evidence.push(measured.evidence().clone());
        members.push(searched((found, evidence, related), &end.label));
    }
    Ok(members)
}

/// Whether a selected obstacle reaches into the free space at each end of
/// the flight, or in front of the ramp's lowest and beyond its highest
/// run.
pub(super) fn end_spaces(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let obstacles: Selected = Ok(selected(context, call, "obstacles")?);
    let size = (
        length(call, "depth").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "width").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "height").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let selector = naming(&[]);
    let check = ramp_ends::EndSpaceCheck::new(&selector, size);
    let free = context.services.get::<FreeSpaceServiceHandle>();
    let mut members = Vec::new();
    match walked {
        Walked::Flight(flight) => {
            for (end, top, word) in [
                (WalkingEnd::FlightBottom, false, "bottom"),
                (WalkingEnd::FlightTop, true, "top"),
            ] {
                let found = ramp_ends::flight_end_space(
                    stairs,
                    free,
                    &check,
                    &obstacles,
                    flight,
                    top,
                    landing_level(flight, end),
                );
                members.push(searched(found, word));
            }
        }
        Walked::Ramp(surface) => {
            let runs = surface.runs();
            for (run, top, word) in [(runs.first(), false, "bottom"), (runs.last(), true, "top")] {
                let Some(run) = run else { continue };
                let found = ramp_ends::end_space(free, &check, &obstacles, object, run, top);
                members.push(searched(found, word));
            }
        }
    }
    Ok(members)
}

/// The clear width of the flight (and, with `landing`, of the landing at
/// each end the call names, `ends=both|bottom|top|none`), or of each of
/// the ramp's runs.
fn clear_widths(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let (candidates, _) = selected(context, call, "obstacles")?;
    let band = (
        length(call, "band_from").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "band_to").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let selector = naming(&candidates);
    let check = clear_width::ClearWidthCheck::measuring(&selector, band);
    let mut widths = Vec::new();
    match walked {
        Walked::Ramp(surface) => {
            let total = surface.runs().len();
            for index in 0..total {
                widths.push(clear_width::stretch_width(
                    stairs,
                    &check,
                    &candidates,
                    (object, WalkingStretch::Run(index)),
                    &format!("run {} of {total}", index + 1),
                ));
            }
        }
        Walked::Flight(_) => {
            if call.choice("stretch") != Some("no") {
                widths.push(clear_width::stretch_width(
                    stairs,
                    &check,
                    &candidates,
                    (object, WalkingStretch::Flight),
                    "the flight",
                ));
            }
            if let Some(landings) = selection(context, call, "landing")? {
                let landings: Selected = Ok(landings);
                let (bottom, top) = match call.choice("ends") {
                    Some("bottom") => (true, false),
                    Some("top") => (false, true),
                    Some("none") => (false, false),
                    Some("intermediate") => intermediate(call, object, context)?,
                    _ => (true, true),
                };
                for (end, measured) in [(false, bottom), (true, top)] {
                    if measured {
                        widths.extend(clear_width::landing_width(
                            stairs,
                            &check,
                            &candidates,
                            &landings,
                            (object, end),
                        ));
                    }
                }
            }
        }
    }
    Ok(widths
        .iter()
        .map(|width| {
            let locator = format!("clear_widths:{object}:{}", width.label());
            let (value, exact) = match width.interval() {
                Ok((interval, exact)) => (number(Some(interval), locator), exact),
                Err(why) => (undecided(why), true),
            };
            member(
                exact,
                BTreeMap::from([
                    ("label", text(width.label())),
                    ("above", text(width.above())),
                    ("width", value),
                    ("governing", objects(width.governing())),
                    (
                        "place",
                        text(if width.is_landing() {
                            "landing"
                        } else {
                            "stretch"
                        }),
                    ),
                ]),
            )
        })
        .collect())
}

/// The stretches a call measures handrails along: the flight, or each of
/// the ramp's runs, each with how messages name it and its width.
fn stretches(walked: &Walked) -> Vec<(WalkingStretch, String, Option<MeasuredInterval>)> {
    match walked {
        Walked::Flight(flight) => {
            vec![(WalkingStretch::Flight, "the flight".into(), flight.width())]
        }
        Walked::Ramp(surface) => {
            let total = surface.runs().len();
            surface
                .runs()
                .iter()
                .enumerate()
                .map(|(index, run)| {
                    (
                        WalkingStretch::Run(index),
                        format!("run {} of {total}", index + 1),
                        run.width(),
                    )
                })
                .collect()
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RailsKey(ObjectId, usize, Vec<ObjectId>, [u64; 3]);

/// The handrails along one stretch, measured once per run.
fn measured_rails(
    context: &RuleContext<'_>,
    stairs: &WalkingSurfaceServiceHandle,
    check: &HandrailCheck<'_>,
    candidates: &[ObjectId],
    along: &Along<'_>,
    (reach, above, level): (f64, f64, f64),
) -> Result<HandrailEvidence, String> {
    let stretch = match along.stretch {
        WalkingStretch::Flight => 0,
        WalkingStretch::Run(index) => index + 1,
    };
    MeasuredMemo::of(
        context.services,
        RailsKey(
            along.object.clone(),
            stretch,
            candidates.to_vec(),
            [reach.to_bits(), above.to_bits(), level.to_bits()],
        ),
        || handrails::measure(stairs, check, candidates, along),
    )
}

/// The handrail lists: each stretch measured (`handrail_stretches`: the
/// sides a rail runs along), each rail's height (`rail_heights`), each end
/// extension of the handrail along a side or of a rail over the middle
/// (`rail_extensions`), and each gap between consecutive pieces along a side
/// (`rail_gaps`).
#[allow(clippy::too_many_lines)]
fn rails(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let (candidates, _) = selected(context, call, "rails")?;
    let reach = (
        length(call, "reach_across").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "reach_above").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let level = length(call, "level_over").unwrap_or(0.0);
    let selector = naming(&candidates);
    let check = HandrailCheck::measuring(&selector, reach, level);
    let risers = match walked {
        Walked::Flight(flight) if call.choice("from") == Some("riser") => {
            Some(RiserOffsets::of(flight))
        }
        _ => None,
    };
    let from_riser = risers.is_some();
    let mut members = Vec::new();
    for (index, (stretch, label, width)) in stretches(walked).into_iter().enumerate() {
        let numbered = index + 1;
        #[allow(clippy::cast_precision_loss)]
        let key = || {
            plain(
                numbered as f64,
                format!("{}:{object}#{numbered}", call.name()),
            )
        };
        let along = Along {
            object,
            stretch,
            label: &label,
            width,
            risers: None,
        };
        let measured = measured_rails(
            context,
            stairs,
            &check,
            &candidates,
            &along,
            (reach.0, reach.1, level),
        );
        let locator = |field: &str| format!("{}:{object}#{numbered}:{field}", call.name());
        let measured = match measured {
            Ok(measured) => measured,
            Err(why) => {
                if call.name() == "handrail_stretches" {
                    members.push(member(
                        true,
                        BTreeMap::from([
                            ("stretch", key()),
                            ("label", text(label.clone())),
                            ("measured", undecided(why)),
                            ("sides", number(None, locator("sides"))),
                            ("side", text("")),
                            ("on_sides", objects(&[])),
                            ("width", number(width, locator("width"))),
                            ("scale", plain(0.0, locator("scale"))),
                        ]),
                    ));
                }
                continue;
            }
        };
        let exact = measured.evidence().exact;
        let scale = handrails::scale(&measured);
        match call.name() {
            "handrail_stretches" => {
                let found: std::collections::BTreeSet<RailSide> = measured
                    .rails()
                    .iter()
                    .filter_map(|(_, rail)| measured.side(rail))
                    .collect();
                let on_sides: Vec<ObjectId> = measured
                    .rails()
                    .iter()
                    .filter(|(_, rail)| measured.side(rail).is_some())
                    .map(|(rail, _)| rail.clone())
                    .collect();
                #[allow(clippy::cast_precision_loss)]
                let sides = found.len() as f64;
                let side = match found.iter().next() {
                    Some(side) if found.len() == 1 => handrails::side_words(*side),
                    _ => "",
                };
                members.push(member(
                    exact,
                    BTreeMap::from([
                        ("stretch", key()),
                        ("label", text(label.clone())),
                        ("measured", truth(true, locator("measured"))),
                        ("sides", plain(sides, locator("sides"))),
                        ("side", text(side)),
                        ("on_sides", objects(&on_sides)),
                        ("width", number(width, locator("width"))),
                        ("scale", plain(scale, locator("scale"))),
                    ]),
                ));
            }
            "rail_heights" => {
                for (rail, measurement) in measured.rails() {
                    members.push(member(
                        exact,
                        BTreeMap::from([
                            ("stretch", key()),
                            ("label", text(label.clone())),
                            ("rail", text(rail.to_string())),
                            ("rails", objects(std::slice::from_ref(rail))),
                            (
                                "lowest",
                                number(Some(measurement.lowest()), locator("lowest")),
                            ),
                            (
                                "highest",
                                number(Some(measurement.highest()), locator("highest")),
                            ),
                            ("scale", plain(scale, locator("scale"))),
                        ]),
                    ));
                }
            }
            "rail_extensions" => {
                let along = Along {
                    object,
                    stretch,
                    label: &label,
                    width,
                    risers: None,
                };
                let extension = |rail: &ObjectId,
                                 measurement: &axioval_engine::RailMeasurement,
                                 bottom: bool,
                                 over_middle: bool| {
                    let (word, other, rise) = if bottom {
                        ("bottom", "a later", measurement.bottom_rise())
                    } else {
                        ("top", "an earlier", measurement.top_rise())
                    };
                    let reach = if bottom {
                        measured.bottom_extension(measurement)
                    } else {
                        measured.top_extension(measurement)
                    };
                    let from = if from_riser {
                        format!("beyond the {word} riser of {label}")
                    } else {
                        format!("beyond the {word} of {label}")
                    };
                    let place = format!("beyond the {word} of {label}");
                    let reach = match (reach, &risers) {
                        (None, _) => number(None, locator("reach")),
                        (Some(reach), None) => number(Some(reach), locator("reach")),
                        (Some(reach), Some(risers)) => match risers.shift(reach, bottom) {
                            Ok(reach) => number(Some(reach), locator("reach")),
                            Err(why) => undecided(format!(
                                "how far handrail {rail} reaches {from} is not known: {why}"
                            )),
                        },
                    };
                    let rise = match rise {
                        Some(rise) => number(Some(rise), locator("rise")),
                        None => undecided(format!(
                            "whether handrail {rail} runs level over the {} {place} is not measured",
                            crate::level_spacing::metres(level)
                        )),
                    };
                    member(
                        exact,
                        BTreeMap::from([
                            ("stretch", key()),
                            ("label", text(label.clone())),
                            ("rail", text(rail.to_string())),
                            ("rails", objects(std::slice::from_ref(rail))),
                            ("end", text(word)),
                            ("other", text(other)),
                            ("from", text(from)),
                            ("place", text(place)),
                            ("reach", reach),
                            ("rise", rise),
                            ("over_middle", truth(over_middle, locator("over_middle"))),
                            ("scale", plain(scale, locator("scale"))),
                        ]),
                    )
                };
                for side in [RailSide::Left, RailSide::Right] {
                    match measured.side_rail(side) {
                        Ok(pieces) => {
                            for (bottom, piece) in [(true, pieces.first()), (false, pieces.last())]
                            {
                                if let Some((rail, measurement)) = piece {
                                    members.push(extension(rail, measurement, bottom, false));
                                }
                            }
                        }
                        Err(pieces) => {
                            let why = format!(
                                "{}, so its extension is not measured",
                                handrails::unordered(side, &pieces, &along)
                            );
                            members.push(member(
                                exact,
                                BTreeMap::from([
                                    ("stretch", key()),
                                    ("label", text(label.clone())),
                                    ("rail", text("")),
                                    ("rails", objects(&[])),
                                    ("end", text("")),
                                    ("other", text("")),
                                    ("from", text("")),
                                    ("place", text("")),
                                    ("reach", undecided(why.clone())),
                                    ("rise", undecided(why)),
                                    ("over_middle", truth(false, locator("over_middle"))),
                                    ("scale", plain(scale, locator("scale"))),
                                ]),
                            ));
                        }
                    }
                }
                for (rail, measurement) in measured.rails() {
                    if measured.side(measurement).is_some() {
                        continue;
                    }
                    for bottom in [true, false] {
                        members.push(extension(rail, measurement, bottom, true));
                    }
                }
            }
            _ => {
                let along = Along {
                    object,
                    stretch,
                    label: &label,
                    width,
                    risers: None,
                };
                for side in [RailSide::Left, RailSide::Right] {
                    match measured.side_rail(side) {
                        Ok(pieces) => {
                            for pair in pieces.windows(2) {
                                let ((lower, a), (upper, b)) = (pair[0], pair[1]);
                                let named = format!(
                                    "handrail pieces {lower} and {upper} along the {} side of {label}",
                                    handrails::side_words(side)
                                );
                                let gap = match measured.gap(a, b) {
                                    Some(gap) => number(Some(gap), locator("gap")),
                                    None => undecided(format!(
                                        "the gap between {named} is not measured"
                                    )),
                                };
                                members.push(member(
                                    exact,
                                    BTreeMap::from([
                                        ("stretch", key()),
                                        ("label", text(label.clone())),
                                        ("pair", text(named)),
                                        ("rails", objects(&[lower.clone(), upper.clone()])),
                                        ("gap", gap),
                                        ("scale", plain(scale, locator("scale"))),
                                    ]),
                                ));
                            }
                        }
                        Err(pieces) => {
                            let why = format!(
                                "{}, so its continuity is not measured",
                                handrails::unordered(side, &pieces, &along)
                            );
                            members.push(member(
                                exact,
                                BTreeMap::from([
                                    ("stretch", key()),
                                    ("label", text(label.clone())),
                                    ("pair", text("")),
                                    ("rails", objects(&[])),
                                    ("gap", undecided(why)),
                                    ("scale", plain(scale, locator("scale"))),
                                ]),
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(members)
}

/// The searches over a ramp's rails: each side's handrail across every
/// landing between consecutive runs (`rail_continuity`, joined within
/// `tolerance`, else `gap`), and each rail over a selected accessible
/// surface (`rail_obstructions`).
pub(super) fn ramp_rails(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
    continuity: bool,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let Walked::Ramp(surface) = walked else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let rails: Selected = Ok(selected(context, call, "rails")?);
    let reach = (
        length(call, "reach_across").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "reach_above").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let level = length(call, "level_over").unwrap_or(0.0);
    let selector = naming(&[]);
    let check = HandrailCheck::measuring(&selector, reach, level);
    let proximity = context.services.get::<ProximityServiceHandle>();
    let found: Checks = if continuity {
        let gap = length(call, "tolerance")
            .or_else(|| length(call, "gap"))
            .unwrap_or(0.0);
        continuity::across_runs(
            (stairs, proximity),
            &check,
            &rails,
            object,
            (surface.runs(), gap),
        )
    } else {
        let surfaces: Selected = Ok(selected(context, call, "surfaces")?);
        obstruction::obstruction(
            (stairs, proximity),
            &check,
            (&rails, &surfaces),
            object,
            surface.runs(),
        )
    };
    Ok(found
        .into_iter()
        .map(|found| searched(found, "the ramp"))
        .collect())
}

/// The flight itself, one item: its rise and width, the scale of its
/// positions and whether it turns.
fn flights(
    object: &ObjectId,
    walked: &Walked,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let Walked::Flight(flight) = walked else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let locator = |field: &str| format!("flights:{object}:{field}");
    Ok(vec![member(
        flight.evidence().exact,
        BTreeMap::from([
            ("rise", number(Some(flight.rise()), locator("rise"))),
            ("width", number(flight.width(), locator("width"))),
            (
                "scale",
                plain(super::flight_scale(flight), locator("scale")),
            ),
            (
                "turning",
                truth(flight.walking_line().is_turning(), locator("turning")),
            ),
        ]),
    )])
}

/// A whole stair's flights as `stair-geometry`'s whole-stair mode reaches
/// them: those measured, why others may be missing, and the evidence of the
/// path.
struct Stair {
    flights: Vec<TreadFlight>,
    missing: Vec<String>,
    cited: Vec<Evidence>,
}

/// The stair `stair` (the call's `stair` anchor, or the object itself)
/// reaches along `path` among the `flights` selection.
fn stair_of(
    call: &MeasuredCall,
    stair: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Stair, PropertyResolutionError> {
    let name = call.name();
    let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let selection = crate::measured_kinds::selection(context, call, "flights", None)?
        .ok_or(PropertyResolutionError::InvalidRequest)?;
    let everything: Vec<&axioval_ir::Object> = context.project.objects().collect();
    let path = crate::support::Traversal::path(steps).map_err(|(reason, why)| {
        crate::measured_kinds::resolution_error((reason, format!("`{name}` of {stair}: {why}")))
    })?;
    let (parts, cited) = path
        .related(context, stair, &everything)
        .map_err(|(reason, why)| {
            crate::measured_kinds::resolution_error((reason, format!("`{name}` of {stair}: {why}")))
        })?;
    let mut missing: Vec<String> = parts
        .iter()
        .filter(|part| selection.undecided.contains(*part))
        .map(|part| format!("whether {part} is one of its flights is undecided"))
        .collect();
    let mut flights = Vec::new();
    for part in parts
        .iter()
        .filter(|part| selection.matched.contains(*part))
    {
        match flight(context, part, length(call, "walking_line_offset")) {
            Ok(measured) => flights.push(measured),
            Err((_, why)) => missing.push(format!("flight {part} is not measured: {why}")),
        }
    }
    if flights.is_empty() && missing.is_empty() {
        missing.push(format!("{} reaches no selected flight", path.relationship));
    }
    Ok(Stair {
        flights,
        missing,
        cited,
    })
}

/// Whether the bottom and top of `flight` lie on landings between two of
/// the flights of the stair the call names (`stair`, `path`, `flights`);
/// neither without one.
fn intermediate(
    call: &MeasuredCall,
    flight: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<(bool, bool), PropertyResolutionError> {
    let Some(stair) = crate::measured_kinds::selection(context, call, "stair", None)?
        .and_then(|selection| selection.matched.into_iter().next())
    else {
        return Ok((false, false));
    };
    let stair = stair_of(call, &stair, context)?;
    let measured: Vec<&TreadFlight> = stair.flights.iter().collect();
    let (mut bottom, mut top) = (false, false);
    for pair in super::whole::ordered(&measured).windows(2) {
        if super::whole::meets(pair[0], pair[1]) {
            top |= pair[0].object() == flight;
            bottom |= pair[1].object() == flight;
        }
    }
    Ok((bottom, top))
}

/// Whether the tactile objects cover a strip at each end of the flight,
/// `intermediate` true at an end on a landing between two of the stair's
/// flights.
fn tactile_strips(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    (stairs, walked): (&WalkingSurfaceServiceHandle, &Walked),
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let Walked::Flight(flight) = walked else {
        return Err(PropertyResolutionError::InvalidRequest);
    };
    let selection = crate::measured_kinds::selection(context, call, "tactiles", None)?
        .ok_or(PropertyResolutionError::InvalidRequest)?;
    let size = (
        length(call, "offset").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "depth").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let matched: Vec<ObjectId> = selection.matched.into_iter().collect();
    let undecided: Vec<ObjectId> = selection.undecided.into_iter().collect();
    let tactiles = MeasuredMemo::of(
        context.services,
        TactilesKey(matched.clone(), undecided.clone()),
        || super::tactile::read_selected(context, (&matched, &undecided)),
    );
    let selector = naming(&[]);
    let check = super::tactile::TactileCheck::measuring(&selector, size);
    let (bottom, top) = intermediate(call, object, context)?;
    let mut members = Vec::new();
    for (end, at_top, between) in [
        (WalkingEnd::FlightBottom, false, bottom),
        (WalkingEnd::FlightTop, true, top),
    ] {
        let found = super::tactile::strip(
            stairs,
            &check,
            &tactiles,
            flight,
            at_top,
            landing_level(flight, end),
        );
        let mut item = searched(found, if at_top { "top" } else { "bottom" });
        item.fields.insert(
            "intermediate",
            truth(between, format!("tactile_strips:{object}:intermediate")),
        );
        members.push(item);
    }
    Ok(members)
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TactilesKey(Vec<ObjectId>, Vec<ObjectId>);

/// The stair as one item: its rise from its lowest flight's base to its
/// highest flight's top (undecided where no flight is measured), the scale
/// of those two levels, and why flights may be missing.
fn stairs_item(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let stair = stair_of(call, object, context)?;
    let missing = stair.missing.join("; ");
    let locator = |field: &str| format!("stairs:{object}:{field}");
    let exact = stair.cited.iter().all(|evidence| evidence.exact)
        && stair.flights.iter().all(|flight| flight.evidence().exact);
    let (rise, scale) = match super::whole::rise(stair.flights.iter()) {
        Some(rise) => (
            MemberValue::Measured(Measurement::Value {
                lower: rise.lower,
                upper: rise.upper,
                dimension: LENGTH,
                locator: locator("rise"),
            }),
            rise.highest
                .upper_metres()
                .abs()
                .max(rise.lowest.lower_metres().abs()),
        ),
        None => (
            undecided(format!("the stair's rise is not measured: {missing}")),
            0.0,
        ),
    };
    Ok(vec![member(
        exact,
        BTreeMap::from([
            ("rise", rise),
            ("scale", plain(scale, locator("scale"))),
            ("missing", text(missing.clone())),
            (
                "complete",
                truth(stair.missing.is_empty(), locator("complete")),
            ),
        ]),
    )])
}

/// Whether the handrail along each side continues across every landing
/// between consecutive flights of the stair, except where a break door
/// stands: one search per side found broken or undecided.
fn stair_continuity(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    stairs: &WalkingSurfaceServiceHandle,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let stair = stair_of(call, object, context)?;
    let rails: Selected = Ok(selected(context, call, "rails")?);
    let landings: Option<Selected> = selection(context, call, "landing")?.map(Ok);
    let doors: Option<Selected> = selection(context, call, "doors")?.map(Ok);
    let reach = (
        length(call, "reach_across").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "reach_above").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let selector = naming(&[]);
    let check =
        HandrailCheck::measuring(&selector, reach, length(call, "level_over").unwrap_or(0.0))
            .with_gap(length(call, "gap"));
    let across = super::whole::Across {
        context,
        stairs,
        free: context.services.get::<FreeSpaceServiceHandle>(),
        rails: Some(&rails),
        landings: landings.as_ref(),
        doors: doors.as_ref(),
        break_height: doors.as_ref().and_then(|_| length(call, "height")),
    };
    let flights: Vec<&TreadFlight> = stair.flights.iter().collect();
    let exact = stair.cited.iter().all(|evidence| evidence.exact)
        && stair.flights.iter().all(|flight| flight.evidence().exact);
    Ok(across
        .continuity(&check, &flights, &stair.missing)
        .into_iter()
        .map(|found| {
            let mut item = searched(found, "the stair");
            item.exact &= exact;
            item
        })
        .collect())
}

/// The clear widths of a whole stair: each flight's and each landing's
/// between two of its flights, after why flights may be missing (undecided
/// items), each naming its flight.
fn stair_clear_widths(
    call: &MeasuredCall,
    object: &ObjectId,
    context: &RuleContext<'_>,
    stairs: &WalkingSurfaceServiceHandle,
) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
    let stair = stair_of(call, object, context)?;
    let (candidates, _) = selected(context, call, "obstacles")?;
    let landings: Selected = Ok(selection(context, call, "landing")?.unwrap_or_default());
    let band = (
        length(call, "band_from").ok_or(PropertyResolutionError::InvalidRequest)?,
        length(call, "band_to").ok_or(PropertyResolutionError::InvalidRequest)?,
    );
    let selector = naming(&candidates);
    let check = clear_width::ClearWidthCheck::measuring(&selector, band);
    let mut members: Vec<MeasuredMember> = stair
        .missing
        .iter()
        .map(|why| {
            member(
                true,
                BTreeMap::from([
                    ("width", undecided(why.clone())),
                    ("owned", text("")),
                    ("governing", objects(&[])),
                    ("owner", objects(&[])),
                ]),
            )
        })
        .collect();
    let measured: Vec<&TreadFlight> = stair.flights.iter().collect();
    let order = super::whole::ordered(&measured);
    for flight in &measured {
        let id = flight.object();
        let (mut bottom, mut top) = (false, false);
        for pair in order.windows(2) {
            if super::whole::meets(pair[0], pair[1]) {
                top |= pair[0].object() == id;
                bottom |= pair[1].object() == id;
            }
        }
        let mut widths = vec![clear_width::stretch_width(
            stairs,
            &check,
            &candidates,
            (id, WalkingStretch::Flight),
            "the flight",
        )];
        for (end, between) in [(false, bottom), (true, top)] {
            if between {
                widths.extend(clear_width::landing_width(
                    stairs,
                    &check,
                    &candidates,
                    &landings,
                    (id, end),
                ));
            }
        }
        for width in &widths {
            let (value, exact) = match width.interval() {
                Ok((interval, exact)) => (
                    number(Some(interval), format!("stair_clear_widths:{object}:{id}")),
                    exact,
                ),
                Err(why) => (undecided(why), true),
            };
            members.push(member(
                exact,
                BTreeMap::from([
                    ("width", value),
                    ("owned", text(width.owned())),
                    ("governing", objects(width.governing())),
                    ("owner", objects(std::slice::from_ref(id))),
                ]),
            ));
        }
    }
    Ok(members)
}
