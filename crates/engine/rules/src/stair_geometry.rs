//! `stair-geometry` and `ramp-geometry`: step dimensions, ramp slopes,
//! widths, landings and headroom above and below, measured from each
//! flight's or ramp's body.
//!
//! Both judge the positions a [`WalkingSurfaceServiceHandle`] measured. Every
//! length and slope is an interval: a verdict needs the whole interval on
//! one side of its bound, widened by a few units in the last place for the
//! binary rounding of decimal coordinates; one straddling it leaves that
//! check not evaluated. Each check that fails is its own finding, so a
//! flight with an irregular riser and too little headroom is found twice.
//! A turning flight is walked along the line the rule places
//! (`walking_line_offset`), and its goings are measured along it.

// The replaced implementations (`reference.rs`) read the shared search code
// and helpers here; without them some of it is only the searches'.
#![cfg_attr(not(feature = "parity-reference"), allow(dead_code, unused_imports))]

use std::collections::BTreeSet;
use std::fmt::Write as _;

mod clear_width;
mod continuity;
mod defects;
mod handrails;
mod items;
mod measured;
mod obstruction;
mod ramp_ends;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod tactile;
mod template;
mod whole;

pub(crate) use items::StairItems;
pub(crate) use measured::StairMeasures;

use axioval_engine::{
    CapabilityEvaluation, ClearanceBelowRequest, ColumnKind, CompiledRule, Deviation,
    ElevationInterval, FreeSpaceServiceHandle, HeadroomRequest, Landing, LandingEvidence,
    MeasuredInterval, NotEvaluatedReason, ParameterDescriptor, ParameterType, RiserClosure,
    RuleCapability, RuleContext, SlopedRun, TableColumn, Tread, TreadFlight, TreadFlightRequest,
    WalkingEnd, WalkingSurfaceError, WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::counts::real;
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, deviation, judge};
use crate::selection::select_objects;
use crate::support::table::Row;
use crate::support::{Parameters, Unavailable, finding, invalid, si_quantity};

/// Requires each selected stair flight's steps, measured from its body, to
/// lie within the declared ranges.
///
/// The flight is measured through the walking-surface service: its treads
/// are its upward-facing horizontal faces, its risers the height
/// differences from its lowest point through the treads to its top, its
/// goings the distances between consecutive nosings along its walking line:
/// the direction a straight flight climbs, or for a turning flight a line
/// through its treads midway across them or `walking_line_offset` from the
/// side it turns towards. Every declared check is judged on its own:
///
/// - `riser_minimum`/`riser_maximum` bound every riser, `going_minimum`/
///   `going_maximum` every going and `nosing_minimum`/`nosing_maximum`
///   every nosing projection;
/// - `step_length_minimum`/`step_length_maximum` bound `2r + g` for every
///   step whose riser `r` climbs onto a tread and whose going `g` leaves it;
/// - `minimum_risers`/`maximum_risers` bound the number of risers and
///   `maximum_rise` the flight's rise;
/// - `riser_tolerance`/`going_tolerance` bound the difference between the
///   largest and smallest riser or going of the flight;
/// - `winder_angle_maximum` bounds the plan angle between consecutive
///   nosings, which is zero for straight treads, and `winder_angle_minimum`
///   every winder's (an angle not surely zero);
/// - `forbid_open_risers` makes every open riser a finding;
/// - `minimum_headroom` bounds the vertical clearance above the treads to
///   the `headroom_obstacles` the rule selects;
/// - `width_minimum`/`width_maximum` bound the flight's width, its
///   narrowest tread's across the direction it climbs; a winder tapers and
///   has no width, so a flight with winders leaves the check not evaluated;
/// - `landing_depth_minimum`, `landing_width_minimum` and
///   `landing_at_least_walking_width` bound the landing at each end, the
///   level surface of a `landing_objects` object meeting it, and
///   `landings_required` requires one at both ends;
/// - `landing_doors` forbids a door standing in the column
///   `landing_door_height` high over a landing at either end, and with
///   `landing_door_swing` a door swinging over one;
/// - `minimum_headroom_below` bounds the clearance under the flight over
///   the floors of the `headroom_below_spaces` the rule selects;
/// - `handrail_height_minimum`/`handrail_height_maximum` bound the height of
///   each `handrail_objects` rail's top above the nosing line,
///   `handrail_extension_minimum` how far the handrail along each side
///   reaches level beyond the first and last nosing (its first and last
///   piece), `handrail_gap_maximum` the gaps between its pieces, and
///   `handrail_sides` (`one` or `both`, with
///   `handrail_both_sides_above_width` both for wider flights) the sides a
///   rail runs along. A rail belongs to the flight within
///   `handrail_reach_across` of its sides and `handrail_reach_above` above
///   its nosing line. `handrail_extension_maximum` bounds the extension
///   from above, and `handrail_extension_from` `riser` measures it from the
///   first and last riser;
/// - `end_space_depth`, `end_space_width` and `end_space_height` place a
///   free space before the first riser and beyond the last, which no
///   `end_space_obstacles` object may reach into.
///
/// `clear_width_minimum` bounds the narrowest free width across the flight
/// the `clear_width_obstacles` leave between `clear_width_band_from` and
/// `clear_width_band_to` above its pitch line, as the walking-surface
/// service measures it; ramps take the same per run.
///
/// With `tactile_objects`, `tactile_offset` and `tactile_depth`, a tactile
/// strip that deep must lie that far before the first riser and beyond the
/// last, across the flight, covered by a selected object on the level
/// there.
///
/// With `stair_path`, the rule selects whole stairs: each flight
/// `stair_flights` picks among the parts that path reaches is checked as
/// above, and the stair as a whole against `maximum_total_rise` and, with
/// `handrail_continuous_across_landings`, for a handrail joined across every
/// landing between consecutive flights, except where a
/// `handrail_break_doors` door stands.
///
/// A turning flight's landing is placed along the tread meeting it and
/// compared with that tread's width; its handrails are measured in its
/// straight parts, a side's extension taken from the parts at its ends. A
/// service refusing them (a winder at an end, a rail it cannot place)
/// leaves those checks not evaluated; its headroom above and below is
/// measured as a straight flight's.
pub struct StairGeometryCheck;

static STAIR: std::sync::LazyLock<axioval_engine::template::Template> =
    std::sync::LazyLock::new(|| template::stair(stair_parameters()));
static STAIR_PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for StairGeometryCheck {
    fn id(&self) -> &'static str {
        template::STAIR
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        STAIR.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        crate::templates::run((&STAIR, &STAIR_PLANS), context, rule)
    }

    fn template(&self) -> Option<&axioval_engine::template::Template> {
        Some(&STAIR)
    }
}

/// Requires each selected ramp's sloped runs, measured from its body, to fit
/// a slope limit, and its runs to share one slope.
///
/// `slope_limits` rows each state a `maximum_slope` (rise over horizontal
/// length, `0.0833` for 1:12) and optionally the longest run
/// (`maximum_length`, horizontal) and highest rise (`maximum_rise`) it
/// admits; a run conforms when some row holds. A slope that depends on the
/// run's length or rise is one row per step. `slope_tolerance` bounds the
/// difference between the steepest and shallowest run, and
/// `minimum_headroom` the clearance to the `headroom_obstacles`. Widths,
/// landings at each run's ends, the clearance below and handrails along the
/// run's surface are declared as for stairs, per run. `end_space_depth`,
/// `end_space_width` and `end_space_height` place a free space in front of
/// the lowest run and beyond the highest, which no `end_space_obstacles`
/// object may reach into, and no `landing_doors` object may reach into the
/// column `landing_door_height` high over a landing at a run's end; with
/// `landing_door_swing`, no such door's leaves may swing over the landing.
/// `end_landing_depth_minimum` and `end_landing_width_minimum` judge the
/// landings before the lowest and beyond the highest run instead of the
/// landing minimums.
pub struct RampGeometryCheck;

static RAMP: std::sync::LazyLock<axioval_engine::template::Template> =
    std::sync::LazyLock::new(|| template::ramp(ramp_parameters()));
static RAMP_PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for RampGeometryCheck {
    fn id(&self) -> &'static str {
        template::RAMP
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        RAMP.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&RAMP, &RAMP_PLANS), context, rule)
    }

    fn template(&self) -> Option<&axioval_engine::template::Template> {
        Some(&RAMP)
    }
}

const SLOPE_LIMITS: &[TableColumn] = &[
    TableColumn::required("maximum_slope", ColumnKind::Number),
    TableColumn::optional("maximum_length", ColumnKind::Quantity),
    TableColumn::optional("maximum_rise", ColumnKind::Quantity),
];

/// Inclusive bounds on a length.
type Range = (Option<f64>, Option<f64>);

fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::Length)) if value >= 0.0 => Ok(Some(value)),
        Some((_, QuantityDimension::Length)) => Err(invalid(format!("`{name}` is negative"))),
        Some(_) => Err(invalid(format!("`{name}` is not a length"))),
    }
}

fn range(parameters: &Parameters<'_>, name: &str) -> Result<Range, Unavailable> {
    let minimum = length(parameters, &format!("{name}_minimum"))?;
    let maximum = length(parameters, &format!("{name}_maximum"))?;
    if let (Some(minimum), Some(maximum)) = (minimum, maximum)
        && minimum > maximum
    {
        return Err(invalid(format!(
            "`{name}_minimum` exceeds `{name}_maximum`"
        )));
    }
    Ok((minimum, maximum))
}

fn range_descriptors(name: &str) -> [ParameterDescriptor; 2] {
    [
        ParameterDescriptor::optional(format!("{name}_minimum"), ParameterType::Quantity),
        ParameterDescriptor::optional(format!("{name}_maximum"), ParameterType::Quantity),
    ]
}

fn headroom_descriptors() -> [ParameterDescriptor; 2] {
    [
        ParameterDescriptor::optional("minimum_headroom", ParameterType::Quantity),
        ParameterDescriptor::optional("headroom_obstacles", ParameterType::Selector),
    ]
}

fn walking_descriptors() -> Vec<ParameterDescriptor> {
    let mut parameters = range_descriptors("width").to_vec();
    parameters.extend([
        ParameterDescriptor::optional("landing_objects", ParameterType::Selector),
        ParameterDescriptor::optional("landing_depth_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("landing_width_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("landing_at_least_walking_width", ParameterType::Boolean),
        ParameterDescriptor::optional("landings_required", ParameterType::Boolean),
        ParameterDescriptor::optional("minimum_headroom_below", ParameterType::Quantity),
        ParameterDescriptor::optional("headroom_below_spaces", ParameterType::Selector),
    ]);
    parameters.extend(handrails::descriptors());
    parameters.extend(ramp_ends::door_descriptors());
    parameters.extend(ramp_ends::end_space_descriptors());
    parameters.extend(clear_width::descriptors());
    parameters
}

/// Decided objects and whether the selector left any undecided.
pub(super) type Selected = Result<(Vec<ObjectId>, bool), Unavailable>;

/// A step's length `2r + g`, an interval sure to hold it: the one
/// computation `step_length_*` judges and the `steps` member list's
/// `step_length` measures.
pub(crate) fn step_length(
    riser: MeasuredInterval,
    going: MeasuredInterval,
) -> Option<MeasuredInterval> {
    let lower = 2.0f64.mul_add(riser.lower(), going.lower()).next_down();
    let upper = 2.0f64.mul_add(riser.upper(), going.upper()).next_up();
    MeasuredInterval::try_new(lower, upper).ok()
}

/// A few units in the last place of the largest magnitude involved: decimal
/// coordinates read in binary differ from what was meant by that much.
fn slack(scale: f64) -> f64 {
    8.0 * f64::EPSILON * scale.abs().max(1.0)
}

fn service_error(error: &WalkingSurfaceError) -> Unavailable {
    let reason = match error {
        WalkingSurfaceError::UnknownObject(_) | WalkingSurfaceError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        WalkingSurfaceError::Unsupported(_) | WalkingSurfaceError::InexactGeometry(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
        WalkingSurfaceError::InvalidMeasurement | WalkingSurfaceError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (reason, error.to_string())
}

/// `lower` and `upper` shown as a bound's words: `at most 0.19 m`.
fn bound_words(minimum: Option<f64>, maximum: Option<f64>, unit: fn(f64) -> String) -> String {
    match (minimum, maximum) {
        (Some(minimum), Some(maximum)) => format!("{} to {}", unit(minimum), unit(maximum)),
        (Some(minimum), None) => format!("at least {}", unit(minimum)),
        (None, Some(maximum)) => format!("at most {}", unit(maximum)),
        (None, None) => String::new(),
    }
}

/// The outcome of one check on one object: a finding message, or why it
/// could not be decided.
enum Check {
    Pass,
    Fail(String),
    /// A failure of a value missing a bound by the deviation.
    Graded(String, Deviation),
    Undecided(String),
}

impl Check {
    /// A failure, graded when its deviation is known.
    fn failed(message: String, deviation: Option<Deviation>) -> Self {
        match deviation {
            Some(deviation) => Self::Graded(message, deviation),
            None => Self::Fail(message),
        }
    }
}

/// The largest magnitude among a landing's positions.
fn landing_scale(measured: &LandingEvidence) -> f64 {
    let edge = measured.edge();
    let mut positions = vec![edge];
    if let Some(extent) = measured.landing().and_then(Landing::extent) {
        let (left, right) = extent.sides();
        positions.extend([extent.far(), left, right]);
    }
    positions.iter().fold(0.0_f64, |scale, position| {
        scale
            .max(position.lower_metres().abs())
            .max(position.upper_metres().abs())
    })
}

/// The elevation of the landing at one end of a flight: the level it stands
/// on at its bottom, its top at its top (the upper floor a final riser
/// arrives at, or the last tread itself).
fn landing_level(flight: &TreadFlight, end: WalkingEnd) -> ElevationInterval {
    match (end, flight.treads().last()) {
        (WalkingEnd::FlightBottom, _) => flight.base(),
        (_, Some(last)) if !flight.ends_in_riser() => last.elevation(),
        _ => flight.top(),
    }
}

/// The largest magnitude among a flight's positions.
fn flight_scale(flight: &TreadFlight) -> f64 {
    let mut scale = flight
        .base()
        .lower_metres()
        .abs()
        .max(flight.top().upper_metres().abs());
    for tread in flight.treads() {
        scale = scale
            .max(tread.front().lower_metres().abs())
            .max(tread.back().upper_metres().abs())
            .max(tread.elevation().upper_metres().abs())
            .max(sides_scale(tread.sides()));
    }
    scale
}

/// The largest magnitude among the positions of a surface's sides.
fn sides_scale(sides: Option<(ElevationInterval, ElevationInterval)>) -> f64 {
    sides.map_or(0.0, |(left, right)| {
        left.lower_metres().abs().max(right.upper_metres().abs())
    })
}

type Checks = Vec<(Check, Vec<Evidence>, Vec<ObjectId>)>;

/// The largest magnitude among a run's positions.
fn run_scale(run: &SlopedRun) -> f64 {
    [
        run.bottom().lower_metres(),
        run.top().upper_metres(),
        run.start().lower_metres(),
        run.end().upper_metres(),
    ]
    .iter()
    .fold(sides_scale(run.sides()), |scale, value| {
        scale.max(value.abs())
    })
}

/// The binary rounding of decimal coordinates, carried into a slope: rise
/// and length each off by `s`, so the slope by `s·(1 + slope)/length`.
fn slope_slack(run: &SlopedRun) -> f64 {
    let length = run.length().lower().max(f64::MIN_POSITIVE);
    slack(run_scale(run)) * (1.0 + run.slope().upper()) / length
}

/// The capability's parameters, unchanged by its template.
fn stair_parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = Vec::new();
    for name in ["riser", "going", "step_length", "nosing"] {
        parameters.extend(range_descriptors(name));
    }
    parameters.extend([
        ParameterDescriptor::optional("minimum_risers", ParameterType::Integer).per_object(),
        ParameterDescriptor::optional("maximum_risers", ParameterType::Integer).per_object(),
        ParameterDescriptor::optional("maximum_rise", ParameterType::Quantity).per_object(),
        ParameterDescriptor::optional("riser_tolerance", ParameterType::Quantity).per_object(),
        ParameterDescriptor::optional("going_tolerance", ParameterType::Quantity).per_object(),
        ParameterDescriptor::optional("walking_line_offset", ParameterType::Quantity),
        ParameterDescriptor::optional("winder_angle_maximum", ParameterType::Quantity).per_object(),
        ParameterDescriptor::optional("winder_angle_minimum", ParameterType::Quantity).per_object(),
        ParameterDescriptor::optional("forbid_open_risers", ParameterType::Boolean),
        ParameterDescriptor::optional("handrail_extension_from", ParameterType::String),
    ]);
    parameters.extend(headroom_descriptors());
    parameters.extend(walking_descriptors());
    parameters.extend(whole::descriptors());
    parameters.extend(tactile::descriptors());
    parameters.extend(clear_width::stair_descriptors());
    parameters
}

/// The capability's parameters, unchanged by its template.
fn ramp_parameters() -> Vec<ParameterDescriptor> {
    let mut parameters = vec![
        ParameterDescriptor::optional("slope_limits", ParameterType::Table(SLOPE_LIMITS)),
        ParameterDescriptor::optional("slope_tolerance", ParameterType::Number).per_object(),
    ];
    parameters.extend(headroom_descriptors());
    parameters.extend(walking_descriptors());
    parameters.extend([
        ParameterDescriptor::optional("end_landing_depth_minimum", ParameterType::Quantity)
            .per_object(),
        ParameterDescriptor::optional("end_landing_width_minimum", ParameterType::Quantity)
            .per_object(),
    ]);
    parameters.extend(handrails::ramp_descriptors());
    parameters
}
