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

use std::collections::BTreeSet;
use std::fmt::Write as _;

mod clear_width;
mod continuity;
mod handrails;
mod obstruction;
mod ramp_ends;
mod tactile;
mod whole;

use axioval_engine::{
    CapabilityEvaluation, ClearanceBelowRequest, ColumnKind, CompiledRule, Deviation,
    ElevationInterval, FreeSpaceServiceHandle, HeadroomRequest, Landing, LandingEvidence,
    LandingRequest, MeasuredInterval, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    ProximityServiceHandle, RiserClosure, RuleCapability, RuleContext, SlopedRun, TableColumn,
    Tread, TreadFlight, TreadFlightRequest, WalkingEnd, WalkingStretch, WalkingSurfaceError,
    WalkingSurfaceServiceHandle,
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

fn angle(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.quantity(name)? {
        None => Ok(None),
        Some((value, QuantityDimension::PlaneAngle)) if value >= 0.0 => Ok(Some(value)),
        Some((_, QuantityDimension::PlaneAngle)) => Err(invalid(format!("`{name}` is negative"))),
        Some(_) => Err(invalid(format!("`{name}` is not a plane angle"))),
    }
}

fn count(parameters: &Parameters<'_>, name: &str) -> Result<Option<usize>, Unavailable> {
    parameters
        .integer(name)?
        .map(|value| usize::try_from(value).map_err(|_| invalid(format!("`{name}` is negative"))))
        .transpose()
}

fn range_descriptors(name: &str) -> [ParameterDescriptor; 2] {
    [
        ParameterDescriptor::optional(format!("{name}_minimum"), ParameterType::Quantity),
        ParameterDescriptor::optional(format!("{name}_maximum"), ParameterType::Quantity),
    ]
}

/// The headroom check both capabilities share.
struct HeadroomCheck<'a> {
    minimum: f64,
    obstacles: &'a Selector,
}

fn headroom_check<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<HeadroomCheck<'a>>, Unavailable> {
    let minimum = length(parameters, "minimum_headroom")?;
    let obstacles = parameters.selector("headroom_obstacles")?;
    match (minimum, obstacles) {
        (Some(minimum), Some(obstacles)) => Ok(Some(HeadroomCheck { minimum, obstacles })),
        (None, None) => Ok(None),
        _ => Err(invalid(
            "`minimum_headroom` and `headroom_obstacles` are declared together",
        )),
    }
}

fn headroom_descriptors() -> [ParameterDescriptor; 2] {
    [
        ParameterDescriptor::optional("minimum_headroom", ParameterType::Quantity),
        ParameterDescriptor::optional("headroom_obstacles", ParameterType::Selector),
    ]
}

/// The clearance-below check both capabilities share.
struct BelowCheck<'a> {
    minimum: f64,
    spaces: &'a Selector,
}

fn below_check<'a>(parameters: &Parameters<'a>) -> Result<Option<BelowCheck<'a>>, Unavailable> {
    let minimum = length(parameters, "minimum_headroom_below")?;
    let spaces = parameters.selector("headroom_below_spaces")?;
    match (minimum, spaces) {
        (Some(minimum), Some(spaces)) => Ok(Some(BelowCheck { minimum, spaces })),
        (None, None) => Ok(None),
        _ => Err(invalid(
            "`minimum_headroom_below` and `headroom_below_spaces` are declared together",
        )),
    }
}

/// The landing checks both capabilities share; `required` is the stair's,
/// `end_depth` and `end_width` the ramp's own minimums at its two ends.
struct LandingCheck<'a> {
    objects: &'a Selector,
    depth: Option<f64>,
    width: Option<f64>,
    end_depth: Option<f64>,
    end_width: Option<f64>,
    at_least_walking_width: bool,
    required: bool,
    /// Whether a landing door check asks for each landing; without it and
    /// without a size or presence check, the landings are selected only for
    /// their clear widths.
    doors: bool,
}

impl LandingCheck<'_> {
    fn sizes(&self) -> bool {
        self.depth.is_some()
            || self.width.is_some()
            || self.end_depth.is_some()
            || self.end_width.is_some()
            || self.at_least_walking_width
    }

    /// Whether each landing is measured for its size, presence or doors.
    fn measured(&self) -> bool {
        self.sizes() || self.required || self.doors
    }

    /// The depth and width minimums at a landing: a ramp's end landing takes
    /// its own where declared.
    fn minimums(&self, end: bool) -> (Option<f64>, Option<f64>) {
        if end {
            (self.end_depth.or(self.depth), self.end_width.or(self.width))
        } else {
            (self.depth, self.width)
        }
    }
}

fn landing_check<'a>(
    parameters: &Parameters<'a>,
    doors: bool,
    clear: bool,
) -> Result<Option<LandingCheck<'a>>, Unavailable> {
    let objects = parameters.selector("landing_objects")?;
    let depth = length(parameters, "landing_depth_minimum")?;
    let width = length(parameters, "landing_width_minimum")?;
    let end_depth = length(parameters, "end_landing_depth_minimum")?;
    let end_width = length(parameters, "end_landing_width_minimum")?;
    let at_least_walking_width = parameters
        .boolean("landing_at_least_walking_width")?
        .unwrap_or(false);
    let required = parameters.boolean("landings_required")?.unwrap_or(false);
    let declared = depth.is_some()
        || width.is_some()
        || end_depth.is_some()
        || end_width.is_some()
        || at_least_walking_width
        || required
        || doors
        || clear;
    match (objects, declared) {
        (Some(objects), true) => Ok(Some(LandingCheck {
            objects,
            depth,
            width,
            end_depth,
            end_width,
            at_least_walking_width,
            required,
            doors,
        })),
        (None, false) => Ok(None),
        (None, true) => Err(invalid("a landing check needs `landing_objects`")),
        (Some(_), false) => Err(invalid(
            "`landing_objects` is declared without a landing check",
        )),
    }
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

/// The width, landing, landing-door, clearance-below, handrail and
/// end-space checks both capabilities share.
struct WalkingConfig<'a> {
    width: Range,
    landing: Option<LandingCheck<'a>>,
    below: Option<BelowCheck<'a>>,
    handrail: Option<handrails::HandrailCheck<'a>>,
    end_space: Option<ramp_ends::EndSpaceCheck<'a>>,
    doors: Option<ramp_ends::DoorCheck<'a>>,
    clear: Option<clear_width::ClearWidthCheck<'a>>,
}

impl<'a> WalkingConfig<'a> {
    fn parse(parameters: &Parameters<'a>, ramp: bool) -> Result<Self, Unavailable> {
        let end_space = ramp_ends::parse_end_space(parameters)?;
        let break_doors = parameters.selector("handrail_break_doors")?.is_some();
        let doors = ramp_ends::parse_doors(parameters, break_doors)?;
        let clear = clear_width::parse(parameters)?;
        let landings = clear
            .as_ref()
            .is_some_and(clear_width::ClearWidthCheck::landings);
        Ok(Self {
            width: range(parameters, "width")?,
            landing: landing_check(parameters, doors.is_some() || break_doors, landings)?,
            below: below_check(parameters)?,
            handrail: handrails::parse(parameters, ramp)?,
            end_space,
            doors,
            clear,
        })
    }

    fn declared(&self) -> bool {
        self.width != (None, None)
            || self.landing.is_some()
            || self.below.is_some()
            || self.handrail.is_some()
            || self.end_space.is_some()
            || self.clear.is_some()
    }
}

/// The selections the checks of one rule send with their requests: decided
/// objects and whether the selector left any undecided.
struct Selections {
    headroom: Option<Selected>,
    landings: Option<Selected>,
    below: Option<Selected>,
    rails: Option<Selected>,
    ends: Option<Selected>,
    doors: Option<Selected>,
    clear: Option<Selected>,
    /// The selected landing doors' swings, with `landing_door_swing`.
    swings: Option<Vec<ramp_ends::DoorSwing>>,
}

/// Decided objects and whether the selector left any undecided.
type Selected = Result<(Vec<ObjectId>, bool), Unavailable>;

impl Selections {
    fn select(
        context: &RuleContext<'_>,
        headroom: Option<&HeadroomCheck<'_>>,
        walking: &WalkingConfig<'_>,
    ) -> Self {
        let doors = walking
            .doors
            .as_ref()
            .map(|check| selected(context, check.doors, "door selection"));
        let swings = match (&walking.doors, &doors) {
            (Some(check), Some(Ok((doors, _)))) if check.swing => {
                Some(ramp_ends::door_swings(context, doors))
            }
            _ => None,
        };
        Self {
            headroom: headroom
                .map(|check| selected(context, check.obstacles, "headroom obstacle selection")),
            landings: walking
                .landing
                .as_ref()
                .map(|check| selected(context, check.objects, "landing selection")),
            below: walking
                .below
                .as_ref()
                .map(|check| selected(context, check.spaces, "space selection")),
            rails: walking
                .handrail
                .as_ref()
                .map(|check| selected(context, check.rails, "handrail selection")),
            ends: walking
                .end_space
                .as_ref()
                .map(|check| selected(context, check.obstacles, "end-space obstacle selection")),
            doors,
            clear: walking
                .clear
                .as_ref()
                .map(|check| selected(context, check.obstacles, "clear-width obstacle selection")),
            swings,
        }
    }
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

fn degrees(value: f64) -> String {
    format!("{}°", (value.to_degrees() * 1e4).round() / 1e4)
}

fn ratio(value: f64) -> String {
    format!("{}", (value * 1e6).round() / 1e6)
}

fn shown_ratio(interval: MeasuredInterval) -> String {
    let (low, high) = (ratio(interval.lower()), ratio(interval.upper()));
    if low == high {
        low
    } else {
        format!("between {low} and {high}")
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

/// The worse of two optional deviations.
fn worse(one: Option<Deviation>, other: Option<Deviation>) -> Option<Deviation> {
    match (one, other) {
        (Some(one), Some(other)) => Some(one.worst(other)),
        (one, other) => one.or(other),
    }
}

/// Every `values` interval against `range`, each named `label n of N`.
fn every(label: &str, values: &[MeasuredInterval], range: Range, slack: f64) -> Check {
    let values: Vec<Option<MeasuredInterval>> = values.iter().copied().map(Some).collect();
    every_measured(label, &values, range, slack, metres)
}

/// Every value against `range`, each named `label n of N`, in `unit`; a
/// value not measured leaves the check undecided.
fn every_measured(
    label: &str,
    values: &[Option<MeasuredInterval>],
    (minimum, maximum): Range,
    slack: f64,
    unit: fn(f64) -> String,
) -> Check {
    let (low, high) = (minimum.map(|m| m - slack), maximum.map(|m| m + slack));
    let total = values.len();
    let mut failing = Vec::new();
    let mut missed = None;
    let mut undecided = Vec::new();
    let mut unmeasured = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            unmeasured.push(format!("{label} {} of {total}", index + 1));
            continue;
        };
        let (lower, upper) = (unit(value.lower()), unit(value.upper()));
        let measured = if lower == upper {
            lower
        } else {
            format!("between {lower} and {upper}")
        };
        let named = format!("{label} {} of {total} is {measured}", index + 1);
        match judge(value.lower(), value.upper(), low, high) {
            Verdict::Pass => {}
            Verdict::Fail(_) => {
                failing.push(named);
                missed = worse(
                    missed,
                    deviation(value.lower(), value.upper(), minimum, maximum),
                );
            }
            Verdict::Undecided(_) => undecided.push(named),
        }
    }
    let bound = bound_words(minimum, maximum, unit);
    if !failing.is_empty() {
        Check::failed(format!("{}; {bound} required", failing.join(", ")), missed)
    } else if !undecided.is_empty() || !unmeasured.is_empty() {
        let mut message = Vec::new();
        if !undecided.is_empty() {
            message.push(format!("{}, which straddles {bound}", undecided.join(", ")));
        }
        if !unmeasured.is_empty() {
            message.push(format!("{} not measured", unmeasured.join(", ")));
        }
        Check::Undecided(message.join("; "))
    } else {
        Check::Pass
    }
}

/// Every riser's closure: an open one fails, one not measured is
/// undecided.
fn closed(closures: &[RiserClosure]) -> Check {
    let total = closures.len();
    let named = |wanted: RiserClosure| -> Vec<String> {
        closures
            .iter()
            .enumerate()
            .filter(|(_, closure)| **closure == wanted)
            .map(|(index, _)| format!("riser {} of {total}", index + 1))
            .collect()
    };
    let (open, unknown) = (named(RiserClosure::Open), named(RiserClosure::NotMeasured));
    if !open.is_empty() {
        let open: Vec<String> = open
            .iter()
            .map(|riser| format!("{riser} is open"))
            .collect();
        Check::Fail(format!("{}; closed risers required", open.join(", ")))
    } else if !unknown.is_empty() {
        Check::Undecided(format!(
            "whether {} is closed is not measured",
            unknown.join(" or ")
        ))
    } else {
        Check::Pass
    }
}

/// The difference between the largest and smallest of `values`, as an
/// interval sure to hold it.
fn spread(values: &[MeasuredInterval]) -> Option<(f64, f64)> {
    let most_low = values
        .iter()
        .map(MeasuredInterval::lower)
        .reduce(f64::max)?;
    let most_high = values
        .iter()
        .map(MeasuredInterval::upper)
        .reduce(f64::max)?;
    let least_low = values
        .iter()
        .map(MeasuredInterval::lower)
        .reduce(f64::min)?;
    let least_high = values
        .iter()
        .map(MeasuredInterval::upper)
        .reduce(f64::min)?;
    let lower = (most_low - least_high).next_down().max(0.0);
    let upper = (most_high - least_low).next_up().max(lower);
    Some((lower, upper))
}

/// The spread of `values` against `tolerance`.
fn uniform(
    label: &str,
    values: &[MeasuredInterval],
    tolerance: f64,
    slack: f64,
    unit: fn(f64) -> String,
) -> Check {
    let Some((lower, upper)) = spread(values) else {
        return Check::Pass;
    };
    let listed = values
        .iter()
        .map(|value| {
            if unit(value.lower()) == unit(value.upper()) {
                unit(value.lower())
            } else {
                format!("{}..{}", unit(value.lower()), unit(value.upper()))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let measured = format!(
        "{label}s differ by {} ({listed})",
        if unit(lower) == unit(upper) {
            unit(lower)
        } else {
            format!("between {} and {}", unit(lower), unit(upper))
        }
    );
    match judge(lower, upper, None, Some(tolerance + slack)) {
        Verdict::Pass => Check::Pass,
        Verdict::Fail(_) => Check::Graded(
            format!("{measured}; at most {} allowed", unit(tolerance)),
            Deviation::above(tolerance, lower, upper),
        ),
        Verdict::Undecided(_) => Check::Undecided(format!(
            "{measured}, which straddles the tolerance {}",
            unit(tolerance)
        )),
    }
}

/// Reports `checks` for `object`: each failure a finding, each undecided
/// check a not-evaluated outcome.
fn report(
    evaluation: &mut CapabilityEvaluation,
    rule: &CompiledRule,
    object: &ObjectId,
    checks: Vec<(Check, Vec<Evidence>, Vec<ObjectId>)>,
) {
    for (check, evidence, related) in checks {
        match check {
            Check::Pass => {}
            Check::Fail(message) => {
                evaluation.push_finding(finding(rule, object, message, evidence, related));
            }
            Check::Graded(message, deviation) => evaluation
                .push_graded_finding(finding(rule, object, message, evidence, related), deviation),
            Check::Undecided(message) => evaluation.push_object_not_evaluated(
                object.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                message,
            ),
        }
    }
}

/// The decided objects and whether the selector left any undecided.
fn selected(
    context: &RuleContext<'_>,
    selector: &Selector,
    what: &str,
) -> Result<(Vec<ObjectId>, bool), Unavailable> {
    let (objects, outcomes) = select_objects(context, selector);
    let mut undecided = BTreeSet::new();
    for outcome in outcomes.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(object) => {
                undecided.insert(object.clone());
            }
            None => {
                return Err((
                    outcome.reason().clone(),
                    format!("{what} is undecided: {}", outcome.message()),
                ));
            }
        }
    }
    Ok((
        objects.iter().map(|object| object.id.clone()).collect(),
        !undecided.is_empty(),
    ))
}

/// A least clearance against a minimum: `what` names it (`headroom above
/// the walking surface`), `relation` how it stands to the governing objects
/// (`under`, `over`) and `nothing` what an absent clearance means.
///
/// An object the selector could not decide (`undecided`, named `noun`) can
/// only lower the clearance: too little stands, enough is not evaluated.
struct Clearance<'a> {
    what: &'a str,
    relation: &'a str,
    nothing: &'a str,
    noun: &'a str,
}

impl Clearance<'_> {
    fn judge(
        &self,
        minimum: f64,
        clearance: Option<MeasuredInterval>,
        governing: &[ObjectId],
        undecided: bool,
    ) -> Check {
        let required = format!("at least {} required", metres(minimum));
        let pending = if undecided {
            format!(
                "; {} the selection could not decide may lower it",
                self.noun
            )
        } else {
            String::new()
        };
        let Some(clearance) = clearance else {
            return if undecided {
                Check::Undecided(format!("{}{pending}", self.nothing))
            } else {
                Check::Pass
            };
        };
        let named = governing
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let measured = format!(
            "{} is {} {} {named}",
            self.what,
            shown(clearance.lower(), clearance.upper()),
            self.relation,
        );
        match judge(clearance.lower(), clearance.upper(), Some(minimum), None) {
            Verdict::Fail(_) => Check::Graded(
                format!("{measured}; {required}"),
                Deviation::below(minimum, clearance.lower(), clearance.upper()),
            ),
            Verdict::Pass if !undecided => Check::Pass,
            Verdict::Pass => Check::Undecided(format!("{measured}{pending}")),
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {required}"))
            }
        }
    }
}

/// The headroom above `object` against the rule's minimum.
fn headroom(
    stairs: &WalkingSurfaceServiceHandle,
    check: &HeadroomCheck<'_>,
    obstacles: &Result<(Vec<ObjectId>, bool), Unavailable>,
    object: &ObjectId,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let (candidates, undecided) = match obstacles {
        Ok(obstacles) => obstacles,
        Err((_, message)) => return (Check::Undecided(message.clone()), vec![], vec![]),
    };
    let request = HeadroomRequest::new(object.clone(), candidates.iter().cloned());
    let measured = match stairs.measure_headroom(&request) {
        Ok(measured) => measured,
        Err(error) => {
            return (
                Check::Undecided(format!("headroom: {}", service_error(&error).1)),
                vec![],
                vec![],
            );
        }
    };
    let clearance = Clearance {
        what: "headroom above the walking surface",
        relation: "under",
        nothing: "nothing selected stands above the walking surface",
        noun: "an obstacle",
    };
    let governing = measured.governing().to_vec();
    let check = clearance.judge(check.minimum, measured.clearance(), &governing, *undecided);
    (check, vec![measured.evidence().clone()], governing)
}

/// The clearance below `object` over the selected spaces' floors against
/// the rule's minimum; `noun` names the object (`flight`, `ramp`).
fn below(
    stairs: &WalkingSurfaceServiceHandle,
    check: &BelowCheck<'_>,
    spaces: &Result<(Vec<ObjectId>, bool), Unavailable>,
    object: &ObjectId,
    noun: &str,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let (candidates, undecided) = match spaces {
        Ok(spaces) => spaces,
        Err((_, message)) => return (Check::Undecided(message.clone()), vec![], vec![]),
    };
    let request = ClearanceBelowRequest::new(object.clone(), candidates.iter().cloned());
    let measured = match stairs.measure_clearance_below(&request) {
        Ok(measured) => measured,
        Err(error) => {
            return (
                Check::Undecided(format!("headroom below: {}", service_error(&error).1)),
                vec![],
                vec![],
            );
        }
    };
    let what = format!("headroom below the {noun}");
    let nothing = format!("the {noun} stands above no selected space's floor");
    let clearance = Clearance {
        what: &what,
        relation: "over the floor of",
        nothing: &nothing,
        noun: "a space",
    };
    let governing = measured.governing().to_vec();
    let check = clearance.judge(check.minimum, measured.clearance(), &governing, *undecided);
    (check, vec![measured.evidence().clone()], governing)
}

/// The flight's width against the rule's range.
fn flight_width(width: MeasuredInterval, (minimum, maximum): Range, slack: f64) -> Check {
    let measured = format!("the flight is {} wide", shown(width.lower(), width.upper()));
    let bound = bound_words(minimum, maximum, metres);
    match judge(
        width.lower(),
        width.upper(),
        minimum.map(|m| m - slack),
        maximum.map(|m| m + slack),
    ) {
        Verdict::Pass => Check::Pass,
        Verdict::Fail(_) => Check::failed(
            format!("{measured}; {bound} required"),
            deviation(width.lower(), width.upper(), minimum, maximum),
        ),
        Verdict::Undecided(_) => Check::Undecided(format!("{measured}, which straddles {bound}")),
    }
}

/// One end of a flight or run whose landing is checked.
struct End<'a> {
    object: &'a ObjectId,
    which: WalkingEnd,
    /// The end in a message: `the top of the flight`.
    label: &'a str,
    /// The width of the flight or run, when measured.
    width: Option<MeasuredInterval>,
    /// `flight` or `run`.
    noun: &'a str,
    /// Whether this is one of a ramp's two outermost ends, where its
    /// end-landing minimums apply.
    outermost: bool,
}

/// A landing dimension against the declared minimum and, when the rule asks
/// for it, the width of the flight or run: the requirement is the larger,
/// an interval when that width is one.
fn landing_dimension(
    value: MeasuredInterval,
    stated: Option<f64>,
    walking: Option<MeasuredInterval>,
    slack: f64,
) -> Verdict {
    let stated = stated.unwrap_or(0.0);
    let (low, high) = walking.map_or((stated, stated), |walking| {
        (stated.max(walking.lower()), stated.max(walking.upper()))
    });
    match judge(value.lower(), value.upper(), Some(low - slack), None) {
        Verdict::Fail(message) => Verdict::Fail(message),
        _ => match judge(value.lower(), value.upper(), Some(high - slack), None) {
            Verdict::Pass => Verdict::Pass,
            _ => Verdict::Undecided(String::new()),
        },
    }
}

/// The landing at one end against the rule's landing checks.
fn landing(
    stairs: &WalkingSurfaceServiceHandle,
    check: &LandingCheck<'_>,
    candidates: &Selected,
    at: &End<'_>,
) -> (Checks, Option<LandingEvidence>) {
    let (candidates, undecided) = match candidates {
        Ok(candidates) => candidates,
        Err((_, message)) => {
            return (
                vec![(Check::Undecided(message.clone()), vec![], vec![])],
                None,
            );
        }
    };
    let request = LandingRequest::new(at.object.clone(), at.which, candidates.iter().cloned());
    let measured = match stairs.measure_landing(&request) {
        Ok(measured) => measured,
        Err(error) => {
            return (
                vec![(
                    Check::Undecided(format!(
                        "landing at {}: {}",
                        at.label,
                        service_error(&error).1
                    )),
                    vec![],
                    vec![],
                )],
                None,
            );
        }
    };
    let checks = landing_sizes(check, &measured, *undecided, at);
    (checks, Some(measured))
}

/// A measured landing against the rule's landing checks.
fn landing_sizes(
    check: &LandingCheck<'_>,
    measured: &LandingEvidence,
    undecided: bool,
    at: &End<'_>,
) -> Checks {
    let undecided = &undecided;
    let evidence = vec![measured.evidence().clone()];
    let pending = if *undecided {
        "; an object the selection could not decide may carry it"
    } else {
        ""
    };
    let Some(landing) = measured.landing() else {
        if !check.required {
            return vec![];
        }
        let message = format!("no selected slab or landing meets {}", at.label);
        let check = if *undecided {
            Check::Undecided(format!("{message}{pending}"))
        } else {
            Check::Fail(message)
        };
        return vec![(check, evidence, vec![])];
    };
    if !check.sizes() {
        return vec![];
    }
    let carrier = landing.carrier();
    let related = if carrier == at.object {
        vec![]
    } else {
        vec![carrier.clone()]
    };
    let (Some(depth), Some(width)) = (measured.depth(), measured.width()) else {
        return vec![(
            Check::Undecided(format!(
                "the landing {carrier} at {} fills no rectangle along the walking direction, so \
                 its size is not measured",
                at.label
            )),
            evidence,
            related,
        )];
    };
    let walking = if check.at_least_walking_width {
        match at.width {
            Some(width) => Some(width),
            None => {
                return vec![(
                    Check::Undecided(format!(
                        "the {}'s width is not measured, so the landing at {} is not compared \
                         with it",
                        at.noun, at.label
                    )),
                    evidence,
                    related,
                )];
            }
        }
    } else {
        None
    };
    let slack = 2.0 * slack(landing_scale(measured));
    let mut checks = Vec::new();
    let (depth_minimum, width_minimum) = check.minimums(at.outermost);
    for (value, stated, words) in [
        (depth, depth_minimum, "deep"),
        (width, width_minimum, "wide"),
    ] {
        if stated.is_none() && walking.is_none() {
            continue;
        }
        let dimension = Dimension {
            value,
            stated,
            walking,
            words,
        };
        let check = dimension.judge(at, slack, (*undecided).then_some(pending));
        checks.push((check, evidence.clone(), related.clone()));
    }
    checks
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

/// One dimension of a landing and what it must reach.
struct Dimension<'a> {
    value: MeasuredInterval,
    stated: Option<f64>,
    walking: Option<MeasuredInterval>,
    /// `deep` or `wide`.
    words: &'a str,
}

impl Dimension<'_> {
    /// How far the dimension falls short of its requirement, the larger of
    /// the stated minimum and the walking width, relative to it.
    fn shortfall(&self) -> Option<Deviation> {
        let stated = self.stated.unwrap_or(0.0);
        let (low, high) = self.walking.map_or((stated, stated), |walking| {
            (stated.max(walking.lower()), stated.max(walking.upper()))
        });
        let (lower, upper) = (self.value.lower(), self.value.upper());
        // A shortfall relative to the requirement grows with it.
        Deviation::try_new(
            Deviation::below(low, lower, upper).lower(),
            Deviation::below(high, lower, upper).upper(),
        )
    }

    /// The dimension against its requirement; with `pending`, an object the
    /// selection could not decide might carry a larger landing, so a
    /// shortfall is not evaluated.
    fn judge(&self, at: &End<'_>, slack: f64, pending: Option<&str>) -> Check {
        let mut required = Vec::new();
        if let Some(stated) = self.stated {
            required.push(metres(stated));
        }
        if let Some(walking) = self.walking {
            required.push(format!(
                "the {}'s width ({})",
                at.noun,
                shown(walking.lower(), walking.upper())
            ));
        }
        let required = format!("at least {} required", required.join(" and "));
        let measured = format!(
            "the landing at {} is {} {}",
            at.label,
            shown(self.value.lower(), self.value.upper()),
            self.words
        );
        match landing_dimension(self.value, self.stated, self.walking, slack) {
            Verdict::Pass => Check::Pass,
            Verdict::Fail(_) => match pending {
                Some(pending) => Check::Undecided(format!("{measured}; {required}{pending}")),
                None => Check::failed(format!("{measured}; {required}"), self.shortfall()),
            },
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {required}"))
            }
        }
    }
}

struct StairConfig<'a> {
    walking_line_offset: Option<f64>,
    winder_angle_maximum: Option<f64>,
    winder_angle_minimum: Option<f64>,
    forbid_open_risers: bool,
    riser: Range,
    going: Range,
    step_length: Range,
    nosing: Range,
    risers: (Option<usize>, Option<usize>),
    maximum_rise: Option<f64>,
    riser_tolerance: Option<f64>,
    going_tolerance: Option<f64>,
    headroom: Option<HeadroomCheck<'a>>,
    walking: WalkingConfig<'a>,
    /// With `stair_path`, the rule selects whole stairs.
    stair: Option<whole::StairMode<'a>>,
    tactile: Option<tactile::TactileCheck<'a>>,
}

impl<'a> StairConfig<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let walking_line_offset = length(&parameters, "walking_line_offset")?;
        if walking_line_offset == Some(0.0) {
            return Err(invalid("`walking_line_offset` is zero"));
        }
        let config = Self {
            walking_line_offset,
            winder_angle_maximum: angle(&parameters, "winder_angle_maximum")?,
            winder_angle_minimum: angle(&parameters, "winder_angle_minimum")?,
            forbid_open_risers: parameters.boolean("forbid_open_risers")?.unwrap_or(false),
            riser: range(&parameters, "riser")?,
            going: range(&parameters, "going")?,
            step_length: range(&parameters, "step_length")?,
            nosing: range(&parameters, "nosing")?,
            risers: (
                count(&parameters, "minimum_risers")?,
                count(&parameters, "maximum_risers")?,
            ),
            maximum_rise: length(&parameters, "maximum_rise")?,
            riser_tolerance: length(&parameters, "riser_tolerance")?,
            going_tolerance: length(&parameters, "going_tolerance")?,
            headroom: headroom_check(&parameters)?,
            walking: WalkingConfig::parse(&parameters, false)?,
            stair: None,
            tactile: tactile::parse(&parameters)?,
        };
        let stair = whole::parse(
            &parameters,
            config.walking.handrail.as_ref(),
            parameters.selector("landing_objects")?.is_some(),
        )?;
        let config = Self { stair, ..config };
        if let (Some(minimum), Some(maximum)) = config.risers
            && minimum > maximum
        {
            return Err(invalid("`minimum_risers` exceeds `maximum_risers`"));
        }
        if let (Some(minimum), Some(maximum)) =
            (config.winder_angle_minimum, config.winder_angle_maximum)
            && minimum > maximum
        {
            return Err(invalid(
                "`winder_angle_minimum` exceeds `winder_angle_maximum`",
            ));
        }
        let none = |range: Range| range == (None, None);
        if none(config.riser)
            && none(config.going)
            && none(config.step_length)
            && none(config.nosing)
            && config.risers == (None, None)
            && config.maximum_rise.is_none()
            && config.riser_tolerance.is_none()
            && config.going_tolerance.is_none()
            && config.winder_angle_maximum.is_none()
            && config.winder_angle_minimum.is_none()
            && !config.forbid_open_risers
            && config.headroom.is_none()
            && !config.walking.declared()
            && !config
                .stair
                .as_ref()
                .is_some_and(whole::StairMode::declared)
            && config.tactile.is_none()
        {
            return Err(invalid("declare at least one stair check"));
        }
        Ok(config)
    }
}

impl RuleCapability for StairGeometryCheck {
    fn id(&self) -> &'static str {
        "axioval:capability.stair-geometry"
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
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
            ParameterDescriptor::optional("winder_angle_maximum", ParameterType::Quantity)
                .per_object(),
            ParameterDescriptor::optional("winder_angle_minimum", ParameterType::Quantity)
                .per_object(),
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

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        if crate::object_parameters::has_object_parameters(rule) {
            return crate::object_parameters::per_object(self, context, rule);
        }
        let config = match StairConfig::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("stair-geometry: {message}"),
                );
            }
        };
        let Some(stairs) = context.services.get::<WalkingSurfaceServiceHandle>() else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "walking-surface service is not registered",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let selections = Selections::select(context, config.headroom.as_ref(), &config.walking);
        let free = context.services.get::<FreeSpaceServiceHandle>();
        let tactiles = config
            .tactile
            .as_ref()
            .map(|check| tactile::read(context, check));
        let flights = Flights {
            stairs,
            free,
            config: &config,
            selections: &selections,
            tactiles: tactiles.as_deref(),
        };
        if let Some(mode) = &config.stair {
            whole::evaluate(context, rule, mode, &flights, &selected, &mut evaluation);
            return evaluation;
        }
        for object in selected {
            let flight = match flights.measure(&object.id) {
                Ok(flight) => flight,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            report(
                &mut evaluation,
                rule,
                &object.id,
                flights.checks(&flight, Intermediate::default()).0,
            );
        }
        evaluation
    }
}

/// One rule's flights: how each is measured and checked.
struct Flights<'s, 'a> {
    stairs: &'s WalkingSurfaceServiceHandle,
    free: Option<&'s FreeSpaceServiceHandle>,
    config: &'s StairConfig<'a>,
    selections: &'s Selections,
    /// The objects that may be tactile surfaces, with a tactile check.
    tactiles: Option<&'s [tactile::Tactile]>,
}

/// Which ends of a flight lie on a landing between two flights of a stair.
#[derive(Clone, Copy, Default)]
struct Intermediate {
    bottom: bool,
    top: bool,
}

impl Flights<'_, '_> {
    /// The flight `object`, walked where the rule places its line.
    fn measure(&self, object: &ObjectId) -> Result<TreadFlight, Unavailable> {
        let request = match self.config.walking_line_offset {
            None => Ok(TreadFlightRequest::new(object.clone())),
            Some(offset) => TreadFlightRequest::from_inner_side(object.clone(), offset),
        };
        request
            .and_then(|request| self.stairs.measure_tread_flight(&request))
            .map_err(|error| service_error(&error))
    }

    /// Every check the rule declares on one flight, each citing it; the
    /// tactile strips on `intermediate` ends only where the rule asks for
    /// them. In whole-stair mode the landings' clear widths are measured on
    /// `intermediate` ends only, and returned with the flight's own for the
    /// stair's total.
    fn checks(
        &self,
        flight: &TreadFlight,
        intermediate: Intermediate,
    ) -> (Checks, Vec<clear_width::Width>) {
        let (stairs, config, selections) = (self.stairs, self.config, self.selections);
        let object = flight.object();
        let mut checks = stair_checks(config, flight);
        if let (Some(check), Some(obstacles)) = (&config.headroom, &selections.headroom) {
            checks.push(headroom(stairs, check, obstacles, object));
        }
        let width = flight.width();
        if config.walking.width != (None, None) {
            checks.push(match width {
                Some(width) => {
                    let slack = slack(flight_scale(flight));
                    (
                        flight_width(width, config.walking.width, slack),
                        vec![],
                        vec![],
                    )
                }
                None => (
                    Check::Undecided(
                        "the flight's width is not measured: a tread fills no rectangle along \
                         the direction it climbs, as a winder never does"
                            .into(),
                    ),
                    vec![],
                    vec![],
                ),
            });
        }
        let door = LandingDoors {
            free: self.free,
            walking: &config.walking,
            selections,
        };
        checks.extend(flight_landings(stairs, &door, flight));
        if let (Some(check), Some(obstacles)) = (&config.walking.end_space, &selections.ends) {
            for (end, top) in [
                (WalkingEnd::FlightBottom, false),
                (WalkingEnd::FlightTop, true),
            ] {
                checks.push(ramp_ends::flight_end_space(
                    stairs,
                    self.free,
                    check,
                    obstacles,
                    flight,
                    top,
                    landing_level(flight, end),
                ));
            }
        }
        if let (Some(check), Some(tactiles)) = (&config.tactile, self.tactiles) {
            for (end, top, between) in [
                (WalkingEnd::FlightBottom, false, intermediate.bottom),
                (WalkingEnd::FlightTop, true, intermediate.top),
            ] {
                if between && !check.intermediate {
                    continue;
                }
                checks.push(tactile::strip(
                    stairs,
                    check,
                    tactiles,
                    flight,
                    top,
                    landing_level(flight, end),
                ));
            }
        }
        if let (Some(check), Some(spaces)) = (&config.walking.below, &selections.below) {
            checks.push(below(stairs, check, spaces, object, "flight"));
        }
        let (found, widths) = self.clear_widths(object, intermediate);
        checks.extend(found);
        if let (Some(check), Some(rails)) = (&config.walking.handrail, &selections.rails) {
            let along = handrails::Along {
                object,
                stretch: WalkingStretch::Flight,
                label: "the flight",
                width,
                risers: Some(handrails::RiserOffsets::of(flight)),
            };
            checks.extend(handrails::handrails(stairs, check, rails, &along));
        }
        let checks = checks
            .into_iter()
            .map(|(check, mut evidence, related)| {
                evidence.insert(0, flight.evidence().clone());
                (check, evidence, related)
            })
            .collect();
        (checks, widths)
    }

    /// The clear widths of a flight and its landings against the rule's
    /// minimums, and the widths measured.
    fn clear_widths(
        &self,
        object: &ObjectId,
        intermediate: Intermediate,
    ) -> (Checks, Vec<clear_width::Width>) {
        let (stairs, config, selections) = (self.stairs, self.config, self.selections);
        let mut checks = Vec::new();
        let mut widths = Vec::new();
        if let (Some(check), Some(obstacles)) = (&config.walking.clear, &selections.clear) {
            let whole = config.stair.is_some();
            let ends = if whole {
                [intermediate.bottom, intermediate.top]
            } else {
                [true, true]
            };
            match clear_width::flight_widths(
                stairs,
                check,
                (obstacles, selections.landings.as_ref()),
                object,
                ends,
            ) {
                Ok(measured) => widths = measured,
                Err(message) => checks.push((Check::Undecided(message), vec![], vec![])),
            }
            let undecided = matches!(obstacles, Ok((_, true)));
            checks.extend(clear_width::judge_each(check, &widths, undecided));
            if let (Some(total), false, Ok(_)) = (check.total(), whole, obstacles) {
                checks.push(clear_width::judge_total(
                    total,
                    &widths,
                    undecided,
                    ("the flight and its landings", false),
                    &[],
                ));
            }
        }
        (checks, widths)
    }
}

/// The landing at each end of a flight against the rule's landing checks,
/// and the doors on it; nothing without a landing check.
fn flight_landings(
    stairs: &WalkingSurfaceServiceHandle,
    door: &LandingDoors<'_, '_>,
    flight: &TreadFlight,
) -> Checks {
    let (Some(check), Some(candidates)) = (&door.walking.landing, &door.selections.landings) else {
        return Vec::new();
    };
    if !check.measured() {
        return Vec::new();
    }
    let mut checks = Vec::new();
    for (end, label) in [
        (WalkingEnd::FlightBottom, "the bottom of the flight"),
        (WalkingEnd::FlightTop, "the top of the flight"),
    ] {
        // A turning flight's winders have no width: its landing is compared
        // with the tread that meets it.
        let width = if flight.walking_line().is_turning() {
            let tread = match end {
                WalkingEnd::FlightBottom => flight.treads().first(),
                _ => flight.treads().last(),
            };
            tread.and_then(Tread::width)
        } else {
            flight.width()
        };
        let at = End {
            object: flight.object(),
            which: end,
            label,
            width,
            noun: "flight",
            outermost: false,
        };
        let (found, measured) = landing(stairs, check, candidates, &at);
        checks.extend(found);
        if let Some(measured) = &measured {
            checks.extend(door.check(measured, landing_level(flight, end), label));
        }
    }
    checks
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

fn stair_checks(
    config: &StairConfig<'_>,
    flight: &TreadFlight,
) -> Vec<(Check, Vec<Evidence>, Vec<ObjectId>)> {
    let slack = slack(flight_scale(flight));
    let risers = flight.risers();
    let goings = flight.goings();
    let mut checks = Vec::new();
    let mut push = |check: Check| checks.push((check, Vec::new(), Vec::new()));
    let declared = |range: Range| range != (None, None);
    if declared(config.riser) {
        push(every("riser", &risers, config.riser, slack));
    }
    if declared(config.going) {
        push(every("going", &goings, config.going, slack));
    }
    if declared(config.nosing) {
        push(every("nosing", &flight.nosings(), config.nosing, slack));
    }
    if declared(config.step_length) {
        // Riser `i + 1` climbs onto tread `i`, whose going `i` leaves it.
        let steps: Vec<MeasuredInterval> = goings
            .iter()
            .zip(risers.iter().skip(1))
            .filter_map(|(going, riser)| {
                let lower = 2.0f64.mul_add(riser.lower(), going.lower()).next_down();
                let upper = 2.0f64.mul_add(riser.upper(), going.upper()).next_up();
                MeasuredInterval::try_new(lower, upper).ok()
            })
            .collect();
        push(every(
            "step length (2r + g)",
            &steps,
            config.step_length,
            3.0 * slack,
        ));
    }
    let (minimum, maximum) = config.risers;
    if minimum.is_some() || maximum.is_some() {
        let number = risers.len();
        let too_few = minimum.is_some_and(|minimum| number < minimum);
        let too_many = maximum.is_some_and(|maximum| number > maximum);
        if too_few || too_many {
            let words = match (minimum, maximum) {
                (Some(minimum), Some(maximum)) => format!("{minimum} to {maximum}"),
                (Some(minimum), None) => format!("at least {minimum}"),
                (_, Some(maximum)) => format!("at most {maximum}"),
                (None, None) => String::new(),
            };
            let count = |value: usize| real(i64::try_from(value).unwrap_or(i64::MAX));
            let missed = match (minimum.filter(|_| too_few), maximum.filter(|_| too_many)) {
                (Some(minimum), _) => {
                    Deviation::below(count(minimum), count(number), count(number))
                }
                (None, Some(maximum)) => {
                    Deviation::above(count(maximum), count(number), count(number))
                }
                (None, None) => unreachable!("a bound was missed"),
            };
            push(Check::Graded(
                format!("the flight has {number} risers; {words} allowed"),
                missed,
            ));
        }
    }
    if let Some(maximum) = config.maximum_rise {
        let rise = flight.rise();
        let measured = format!("the flight rises {}", shown(rise.lower(), rise.upper()));
        push(
            match judge(rise.lower(), rise.upper(), None, Some(maximum + slack)) {
                Verdict::Pass => Check::Pass,
                Verdict::Fail(_) => Check::Graded(
                    format!("{measured}; at most {} allowed", metres(maximum)),
                    Deviation::above(maximum, rise.lower(), rise.upper()),
                ),
                Verdict::Undecided(_) => Check::Undecided(format!(
                    "{measured}, which straddles at most {}",
                    metres(maximum)
                )),
            },
        );
    }
    if let Some(tolerance) = config.riser_tolerance {
        push(uniform("riser", &risers, tolerance, 2.0 * slack, metres));
    }
    if let Some(tolerance) = config.going_tolerance {
        push(uniform("going", &goings, tolerance, 2.0 * slack, metres));
    }
    checks.extend(
        turning_checks(config, flight)
            .into_iter()
            .map(|check| (check, Vec::new(), Vec::new())),
    );
    checks
}

/// The checks a turning or open flight adds: winder angles and open
/// risers.
fn turning_checks(config: &StairConfig<'_>, flight: &TreadFlight) -> Vec<Check> {
    let mut checks = Vec::new();
    if let Some(maximum) = config.winder_angle_maximum {
        checks.push(every_measured(
            "winder angle",
            &flight.winder_angles(),
            (None, Some(maximum)),
            ANGLE_SLACK,
            degrees,
        ));
    }
    // A straight flight has no winder to judge.
    if let Some(minimum) = config.winder_angle_minimum
        && flight.walking_line().is_turning()
    {
        checks.push(winders_at_least(&flight.winder_angles(), minimum));
    }
    if config.forbid_open_risers {
        checks.push(closed(&flight.riser_closures()));
    }
    checks
}

/// Every winder's angle against a minimum: a straight tread (an angle surely
/// zero) is no winder and is skipped; an angle that may be zero or a
/// winder's too small one, or one not measured, is undecided.
fn winders_at_least(angles: &[Option<MeasuredInterval>], minimum: f64) -> Check {
    let total = angles.len();
    let mut failing = Vec::new();
    let mut worst = None;
    let mut undecided = Vec::new();
    for (index, angle) in angles.iter().enumerate() {
        let named = |angle: MeasuredInterval| {
            let (lower, upper) = (degrees(angle.lower()), degrees(angle.upper()));
            let measured = if lower == upper {
                lower
            } else {
                format!("between {lower} and {upper}")
            };
            format!("winder angle {} of {total} is {measured}", index + 1)
        };
        let Some(angle) = angle else {
            undecided.push(format!(
                "winder angle {} of {total} not measured",
                index + 1
            ));
            continue;
        };
        if angle.upper() <= ANGLE_SLACK || angle.lower() >= minimum - ANGLE_SLACK {
            continue;
        }
        if angle.lower() > ANGLE_SLACK && angle.upper() < minimum - ANGLE_SLACK {
            failing.push(named(*angle));
            worst = worse(
                worst,
                Some(Deviation::below(minimum, angle.lower(), angle.upper())),
            );
        } else {
            undecided.push(format!(
                "{}, which may be a straight tread or straddles at least {}",
                named(*angle),
                degrees(minimum)
            ));
        }
    }
    if !failing.is_empty() {
        Check::failed(
            format!(
                "{}; at least {} required for a winder",
                failing.join(", "),
                degrees(minimum)
            ),
            worst,
        )
    } else if !undecided.is_empty() {
        Check::Undecided(undecided.join("; "))
    } else {
        Check::Pass
    }
}

/// Decimal coordinates read in binary turn a nosing by a few units in the
/// last place of a radian.
const ANGLE_SLACK: f64 = 1e-9;

/// One row of `slope_limits`, in canonical units.
struct SlopeLimit {
    slope: f64,
    length: Option<f64>,
    rise: Option<f64>,
}

impl SlopeLimit {
    fn read(row: Row<'_>) -> Result<Self, Unavailable> {
        let slope = row
            .number("maximum_slope")?
            .ok_or_else(|| invalid("a `slope_limits` row needs `maximum_slope`"))?;
        if slope < 0.0 {
            return Err(invalid("`maximum_slope` is negative"));
        }
        let cell = |column: &str| -> Result<Option<f64>, Unavailable> {
            match row.quantity(column)? {
                None => Ok(None),
                Some((value, unit)) => match si_quantity(value, unit)? {
                    (value, QuantityDimension::Length) if value >= 0.0 => Ok(Some(value)),
                    _ => Err(invalid(format!("`{column}` is not a non-negative length"))),
                },
            }
        };
        Ok(Self {
            slope,
            length: cell("maximum_length")?,
            rise: cell("maximum_rise")?,
        })
    }

    fn describe(&self) -> String {
        let mut words = format!("slope at most {}", ratio(self.slope));
        if let Some(length) = self.length {
            let _ = write!(words, " over at most {}", metres(length));
        }
        if let Some(rise) = self.rise {
            let _ = write!(words, " rising at most {}", metres(rise));
        }
        words
    }
}

struct RampConfig<'a> {
    limits: Vec<SlopeLimit>,
    slope_tolerance: Option<f64>,
    headroom: Option<HeadroomCheck<'a>>,
    walking: WalkingConfig<'a>,
    obstruction: Option<obstruction::ObstructionCheck<'a>>,
}

impl<'a> RampConfig<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let limits = parameters
            .table("slope_limits")?
            .unwrap_or_default()
            .into_iter()
            .map(SlopeLimit::read)
            .collect::<Result<Vec<_>, _>>()?;
        let slope_tolerance = parameters.number("slope_tolerance")?;
        if slope_tolerance.is_some_and(|tolerance| tolerance < 0.0) {
            return Err(invalid("`slope_tolerance` is negative"));
        }
        let headroom = headroom_check(&parameters)?;
        let walking = WalkingConfig::parse(&parameters, true)?;
        let obstruction = obstruction::parse(&parameters, walking.handrail.as_ref())?;
        if limits.is_empty()
            && slope_tolerance.is_none()
            && headroom.is_none()
            && !walking.declared()
        {
            return Err(invalid("declare at least one ramp check"));
        }
        Ok(Self {
            limits,
            slope_tolerance,
            headroom,
            walking,
            obstruction,
        })
    }
}

impl RuleCapability for RampGeometryCheck {
    fn id(&self) -> &'static str {
        "axioval:capability.ramp-geometry"
    }

    fn grades_deviation(&self) -> bool {
        true
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
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

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match RampConfig::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("ramp-geometry: {message}"),
                );
            }
        };
        let Some(stairs) = context.services.get::<WalkingSurfaceServiceHandle>() else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "walking-surface service is not registered",
            );
        };
        let (selected, mut evaluation) = select_objects(context, &rule.selector);
        let selections = Selections::select(context, config.headroom.as_ref(), &config.walking);
        let free = context.services.get::<FreeSpaceServiceHandle>();
        let proximity = context.services.get::<ProximityServiceHandle>();
        let surfaces = config
            .obstruction
            .as_ref()
            .map(|check| self::selected(context, check.surfaces, "accessible surface selection"));
        for object in selected {
            let checks = ramp(
                (stairs, free, proximity),
                &config,
                (&selections, surfaces.as_ref()),
                object,
            );
            match checks {
                Ok(checks) => report(&mut evaluation, rule, &object.id, checks),
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

type Checks = Vec<(Check, Vec<Evidence>, Vec<ObjectId>)>;

/// The services a ramp's checks ask: walking surfaces, free space and
/// proximity.
type Services<'s> = (
    &'s WalkingSurfaceServiceHandle,
    Option<&'s FreeSpaceServiceHandle>,
    Option<&'s ProximityServiceHandle>,
);

fn ramp(
    (stairs, free, proximity): Services<'_>,
    config: &RampConfig<'_>,
    (selections, surfaces): (&Selections, Option<&Selected>),
    object: &Object,
) -> Result<Checks, Unavailable> {
    let measured = stairs
        .measure_sloped_runs(&object.id)
        .map_err(|error| service_error(&error))?;
    let evidence = measured.evidence().clone();
    let runs = measured.runs();
    let mut checks: Checks = Vec::new();
    if !config.limits.is_empty() {
        for (index, run) in runs.iter().enumerate() {
            let check = slope_limits(&config.limits, run, index, runs.len());
            checks.push((check, vec![evidence.clone()], vec![]));
        }
    }
    if let Some(tolerance) = config.slope_tolerance {
        let slopes: Vec<MeasuredInterval> = runs.iter().map(SlopedRun::slope).collect();
        let slack = runs.iter().map(slope_slack).fold(0.0_f64, f64::max);
        checks.push((
            uniform("run slope", &slopes, tolerance, 2.0 * slack, ratio),
            vec![evidence.clone()],
            vec![],
        ));
    }
    if let (Some(check), Some(obstacles)) = (&config.headroom, &selections.headroom) {
        let (check, mut cited, related) = headroom(stairs, check, obstacles, &object.id);
        cited.insert(0, evidence.clone());
        checks.push((check, cited, related));
    }
    if config.walking.width != (None, None) {
        let widths: Option<Vec<MeasuredInterval>> = runs.iter().map(SlopedRun::width).collect();
        let check = match widths {
            Some(widths) => {
                let scale = runs.iter().map(run_scale).fold(0.0_f64, f64::max);
                every("run width", &widths, config.walking.width, slack(scale))
            }
            None => Check::Undecided(
                "a run's width is not measured: it fills no rectangle along its slope".into(),
            ),
        };
        checks.push((check, vec![evidence.clone()], vec![]));
    }
    if let (Some(check), Some(candidates)) = (&config.walking.landing, &selections.landings)
        && check.measured()
    {
        let total = runs.len();
        for (index, run) in runs.iter().enumerate() {
            for (end, place) in [
                (WalkingEnd::RunBottom(index), "bottom"),
                (WalkingEnd::RunTop(index), "top"),
            ] {
                let label = format!("the {place} of run {} of {total}", index + 1);
                let at = End {
                    object: &object.id,
                    which: end,
                    label: &label,
                    width: run.width(),
                    noun: "run",
                    outermost: (index == 0 && place == "bottom")
                        || (index + 1 == total && place == "top"),
                };
                let (mut found, measured) = landing(stairs, check, candidates, &at);
                if let Some(measured) = &measured {
                    let elevation = if place == "top" {
                        run.top()
                    } else {
                        run.bottom()
                    };
                    let door = LandingDoors {
                        free,
                        walking: &config.walking,
                        selections,
                    };
                    found.extend(door.check(measured, elevation, &label));
                }
                for (check, mut cited, related) in found {
                    cited.insert(0, evidence.clone());
                    checks.push((check, cited, related));
                }
            }
        }
    }
    if let (Some(check), Some(spaces)) = (&config.walking.below, &selections.below) {
        let (check, mut cited, related) = below(stairs, check, spaces, &object.id, "ramp");
        cited.insert(0, evidence.clone());
        checks.push((check, cited, related));
    }
    for (check, mut cited, related) in rails_and_ends(
        (stairs, free, proximity),
        config,
        (selections, surfaces),
        object,
        runs,
    ) {
        cited.insert(0, evidence.clone());
        checks.push((check, cited, related));
    }
    Ok(checks)
}

/// The landing-door checks of one rule, asked at each measured landing.
struct LandingDoors<'s, 'a> {
    free: Option<&'s FreeSpaceServiceHandle>,
    walking: &'s WalkingConfig<'a>,
    selections: &'s Selections,
}

impl LandingDoors<'_, '_> {
    /// No selected door standing on the landing `measured` at `elevation`,
    /// and with `landing_door_swing` none swinging over it; nothing when
    /// the rule declares no landing doors.
    fn check(
        &self,
        measured: &LandingEvidence,
        elevation: ElevationInterval,
        label: &str,
    ) -> Checks {
        let (Some(doors), Some(selected)) = (&self.walking.doors, &self.selections.doors) else {
            return Vec::new();
        };
        let mut checks = vec![ramp_ends::doors(
            self.free, doors, selected, measured, elevation, label,
        )];
        if doors.swing {
            checks.push(match (selected, &self.selections.swings) {
                (Ok((_, undecided)), Some(swings)) => ramp_ends::door_swings_over(
                    doors, swings, *undecided, measured, elevation, label,
                ),
                (Err((_, message)), _) => (Check::Undecided(message.clone()), vec![], vec![]),
                (Ok(_), None) => unreachable!("swings are read with the selection"),
            });
        }
        checks
    }
}

/// The handrails along each run of a ramp and the free space at its ends.
fn rails_and_ends(
    (stairs, free, proximity): Services<'_>,
    config: &RampConfig<'_>,
    (selections, surfaces): (&Selections, Option<&Selected>),
    object: &Object,
    runs: &[SlopedRun],
) -> Checks {
    let mut checks: Checks = Vec::new();
    if let (Some(check), Some(rails)) = (&config.walking.handrail, &selections.rails) {
        let total = runs.len();
        for (index, run) in runs.iter().enumerate() {
            let label = format!("run {} of {total}", index + 1);
            let along = handrails::Along {
                object: &object.id,
                stretch: WalkingStretch::Run(index),
                label: &label,
                width: run.width(),
                risers: None,
            };
            checks.extend(handrails::handrails(stairs, check, rails, &along));
        }
        if let Some(gap) = check.ramp_continuity {
            checks.extend(continuity::across_runs(
                (stairs, proximity),
                check,
                rails,
                &object.id,
                (runs, gap),
            ));
        }
    }
    if let (Some(check), Some(obstacles)) = (&config.walking.clear, &selections.clear) {
        match obstacles {
            Ok((candidates, undecided)) => {
                let total = runs.len();
                let widths: Vec<clear_width::Width> = (0..total)
                    .map(|index| {
                        clear_width::stretch_width(
                            stairs,
                            check,
                            candidates,
                            (&object.id, WalkingStretch::Run(index)),
                            &format!("run {} of {total}", index + 1),
                        )
                    })
                    .collect();
                checks.extend(clear_width::judge_each(check, &widths, *undecided));
            }
            Err((_, message)) => checks.push((Check::Undecided(message.clone()), vec![], vec![])),
        }
    }
    if let (Some(check), Some(obstacles)) = (&config.walking.end_space, &selections.ends) {
        let ends = [(runs.first(), false), (runs.last(), true)];
        for (run, top) in ends {
            let Some(run) = run else { continue };
            checks.push(ramp_ends::end_space(
                free, check, obstacles, &object.id, run, top,
            ));
        }
    }
    if let (Some(check), Some(rails), Some(surfaces)) =
        (&config.walking.handrail, &selections.rails, surfaces)
    {
        checks.extend(obstruction::obstruction(
            (stairs, proximity),
            check,
            (rails, surfaces),
            &object.id,
            runs,
        ));
    }
    checks
}

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

/// How far a run violating `limit` misses it: the worst of its failing
/// quantities, each relative to its bound.
fn missed(limit: &SlopeLimit, run: &SlopedRun) -> Option<Deviation> {
    let (slope, length, rise) = (run.slope(), run.length(), run.rise());
    let over = |value: MeasuredInterval, maximum: Option<f64>| {
        deviation(value.lower(), value.upper(), None, maximum)
    };
    worse(
        over(slope, Some(limit.slope)),
        worse(over(length, limit.length), over(rise, limit.rise)),
    )
}

/// Whether one row holds for a run: `Some(true)` holds, `Some(false)`
/// violated, `None` undecided.
fn holds(limit: &SlopeLimit, run: &SlopedRun) -> Option<bool> {
    let slack = slack(run_scale(run));
    let slope = run.slope();
    let mut verdicts = vec![judge(
        slope.lower(),
        slope.upper(),
        None,
        Some(limit.slope + slope_slack(run)),
    )];
    if let Some(maximum) = limit.length {
        let length = run.length();
        verdicts.push(judge(
            length.lower(),
            length.upper(),
            None,
            Some(maximum + slack),
        ));
    }
    if let Some(maximum) = limit.rise {
        let rise = run.rise();
        verdicts.push(judge(
            rise.lower(),
            rise.upper(),
            None,
            Some(maximum + slack),
        ));
    }
    if verdicts
        .iter()
        .any(|verdict| matches!(verdict, Verdict::Fail(_)))
    {
        Some(false)
    } else if verdicts
        .iter()
        .all(|verdict| matches!(verdict, Verdict::Pass))
    {
        Some(true)
    } else {
        None
    }
}

fn slope_limits(limits: &[SlopeLimit], run: &SlopedRun, index: usize, total: usize) -> Check {
    let (rise, length, slope) = (run.rise(), run.length(), run.slope());
    let measured = format!(
        "run {} of {total} rises {} over {}, a slope of {}",
        index + 1,
        shown(rise.lower(), rise.upper()),
        shown(length.lower(), length.upper()),
        shown_ratio(slope),
    );
    let verdicts: Vec<Option<bool>> = limits.iter().map(|limit| holds(limit, run)).collect();
    if verdicts.contains(&Some(true)) {
        return Check::Pass;
    }
    let rows = limits
        .iter()
        .map(SlopeLimit::describe)
        .collect::<Vec<_>>()
        .join("; or ");
    if verdicts.iter().all(|verdict| *verdict == Some(false)) {
        // Any row would do, so the run misses by as little as the nearest
        // row; a row failing only within the rounding slack grades nothing.
        let nearest = limits
            .iter()
            .map(|limit| missed(limit, run))
            .try_fold(None, |nearest: Option<Deviation>, missed| {
                let missed = missed?;
                Some(Some(
                    nearest.map_or(missed, |nearest| nearest.least(missed)),
                ))
            })
            .flatten();
        Check::failed(format!("{measured}; required {rows}"), nearest)
    } else {
        Check::Undecided(format!(
            "{measured}, which straddles a slope limit ({rows})"
        ))
    }
}
