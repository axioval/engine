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

mod handrails;
mod ramp_ends;

use axioval_engine::{
    CapabilityEvaluation, ClearanceBelowRequest, ColumnKind, CompiledRule, ElevationInterval,
    FreeSpaceServiceHandle, HeadroomRequest, Landing, LandingEvidence, LandingRequest,
    MeasuredInterval, NotEvaluatedReason, ParameterDescriptor, ParameterType, RiserClosure,
    RuleCapability, RuleContext, SlopedRun, TableColumn, TreadFlight, TreadFlightRequest,
    WalkingEnd, WalkingStretch, WalkingSurfaceError, WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, QuantityDimension};

use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, judge};
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
///   nosings, which is zero for straight treads;
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
/// - `minimum_headroom_below` bounds the clearance under the flight over
///   the floors of the `headroom_below_spaces` the rule selects;
/// - `handrail_height_minimum`/`handrail_height_maximum` bound the height of
///   each `handrail_objects` rail's top above the nosing line,
///   `handrail_extension_minimum` how far it reaches level beyond the first
///   and last nosing, and `handrail_sides` (`one` or `both`, with
///   `handrail_both_sides_above_width` both for wider flights) the sides a
///   rail runs along. A rail belongs to the flight within
///   `handrail_reach_across` of its sides and `handrail_reach_above` above
///   its nosing line.
///
/// A turning flight's landings and handrails are measured along no one
/// direction, so the service refuses them and those checks are not
/// evaluated; its headroom above and below is measured as a straight
/// flight's.
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
/// column `landing_door_height` high over a landing at a run's end.
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

/// The landing checks both capabilities share; `required` is the stair's.
struct LandingCheck<'a> {
    objects: &'a Selector,
    depth: Option<f64>,
    width: Option<f64>,
    at_least_walking_width: bool,
    required: bool,
}

impl LandingCheck<'_> {
    fn sizes(&self) -> bool {
        self.depth.is_some() || self.width.is_some() || self.at_least_walking_width
    }
}

fn landing_check<'a>(
    parameters: &Parameters<'a>,
    doors: bool,
) -> Result<Option<LandingCheck<'a>>, Unavailable> {
    let objects = parameters.selector("landing_objects")?;
    let depth = length(parameters, "landing_depth_minimum")?;
    let width = length(parameters, "landing_width_minimum")?;
    let at_least_walking_width = parameters
        .boolean("landing_at_least_walking_width")?
        .unwrap_or(false);
    let required = parameters.boolean("landings_required")?.unwrap_or(false);
    let declared =
        depth.is_some() || width.is_some() || at_least_walking_width || required || doors;
    match (objects, declared) {
        (Some(objects), true) => Ok(Some(LandingCheck {
            objects,
            depth,
            width,
            at_least_walking_width,
            required,
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
    parameters
}

/// The width, landing, clearance-below and handrail checks both
/// capabilities share, and the ramp's end spaces and landing doors.
struct WalkingConfig<'a> {
    width: Range,
    landing: Option<LandingCheck<'a>>,
    below: Option<BelowCheck<'a>>,
    handrail: Option<handrails::HandrailCheck<'a>>,
    end_space: Option<ramp_ends::EndSpaceCheck<'a>>,
    doors: Option<ramp_ends::DoorCheck<'a>>,
}

impl<'a> WalkingConfig<'a> {
    fn parse(parameters: &Parameters<'a>, ramp: bool) -> Result<Self, Unavailable> {
        let (end_space, doors) = if ramp {
            (
                ramp_ends::parse_end_space(parameters)?,
                ramp_ends::parse_doors(parameters)?,
            )
        } else {
            (None, None)
        };
        Ok(Self {
            width: range(parameters, "width")?,
            landing: landing_check(parameters, doors.is_some())?,
            below: below_check(parameters)?,
            handrail: handrails::parse(parameters)?,
            end_space,
            doors,
        })
    }

    fn declared(&self) -> bool {
        self.width != (None, None)
            || self.landing.is_some()
            || self.below.is_some()
            || self.handrail.is_some()
            || self.end_space.is_some()
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
}

/// Decided objects and whether the selector left any undecided.
type Selected = Result<(Vec<ObjectId>, bool), Unavailable>;

impl Selections {
    fn select(
        context: &RuleContext<'_>,
        headroom: Option<&HeadroomCheck<'_>>,
        walking: &WalkingConfig<'_>,
    ) -> Self {
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
            doors: walking
                .doors
                .as_ref()
                .map(|check| selected(context, check.doors, "door selection")),
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
    Undecided(String),
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
            Verdict::Fail(_) => failing.push(named),
            Verdict::Undecided(_) => undecided.push(named),
        }
    }
    let bound = bound_words(minimum, maximum, unit);
    if !failing.is_empty() {
        Check::Fail(format!("{}; {bound} required", failing.join(", ")))
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
        Verdict::Fail(_) => Check::Fail(format!("{measured}; at most {} allowed", unit(tolerance))),
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
            Verdict::Fail(_) => Check::Fail(format!("{measured}; {required}")),
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
        Verdict::Fail(_) => Check::Fail(format!("{measured}; {bound} required")),
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
    for (value, stated, words) in [(depth, check.depth, "deep"), (width, check.width, "wide")] {
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
                None => Check::Fail(format!("{measured}; {required}")),
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
        };
        if let (Some(minimum), Some(maximum)) = config.risers
            && minimum > maximum
        {
            return Err(invalid("`minimum_risers` exceeds `maximum_risers`"));
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
            && !config.forbid_open_risers
            && config.headroom.is_none()
            && !config.walking.declared()
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

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = Vec::new();
        for name in ["riser", "going", "step_length", "nosing"] {
            parameters.extend(range_descriptors(name));
        }
        parameters.extend([
            ParameterDescriptor::optional("minimum_risers", ParameterType::Integer),
            ParameterDescriptor::optional("maximum_risers", ParameterType::Integer),
            ParameterDescriptor::optional("maximum_rise", ParameterType::Quantity),
            ParameterDescriptor::optional("riser_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("going_tolerance", ParameterType::Quantity),
            ParameterDescriptor::optional("walking_line_offset", ParameterType::Quantity),
            ParameterDescriptor::optional("winder_angle_maximum", ParameterType::Quantity),
            ParameterDescriptor::optional("forbid_open_risers", ParameterType::Boolean),
        ]);
        parameters.extend(headroom_descriptors());
        parameters.extend(walking_descriptors());
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
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
        for object in selected {
            let request = match config.walking_line_offset {
                None => Ok(TreadFlightRequest::new(object.id.clone())),
                Some(offset) => TreadFlightRequest::from_inner_side(object.id.clone(), offset),
            };
            let flight = match request.and_then(|request| stairs.measure_tread_flight(&request)) {
                Ok(flight) => flight,
                Err(error) => {
                    let (reason, message) = service_error(&error);
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let mut checks = stair_checks(&config, &flight);
            if let (Some(check), Some(obstacles)) = (&config.headroom, &selections.headroom) {
                checks.push(headroom(stairs, check, obstacles, &object.id));
            }
            let width = flight.width();
            if config.walking.width != (None, None) {
                checks.push(match width {
                    Some(width) => {
                        let slack = slack(flight_scale(&flight));
                        (
                            flight_width(width, config.walking.width, slack),
                            vec![],
                            vec![],
                        )
                    }
                    None => (
                        Check::Undecided(
                            "the flight's width is not measured: a tread fills no rectangle \
                             along the direction it climbs, as a winder never does"
                                .into(),
                        ),
                        vec![],
                        vec![],
                    ),
                });
            }
            if let (Some(check), Some(candidates)) = (&config.walking.landing, &selections.landings)
            {
                for (end, label) in [
                    (WalkingEnd::FlightBottom, "the bottom of the flight"),
                    (WalkingEnd::FlightTop, "the top of the flight"),
                ] {
                    let at = End {
                        object: &object.id,
                        which: end,
                        label,
                        width,
                        noun: "flight",
                    };
                    checks.extend(landing(stairs, check, candidates, &at).0);
                }
            }
            if let (Some(check), Some(spaces)) = (&config.walking.below, &selections.below) {
                checks.push(below(stairs, check, spaces, &object.id, "flight"));
            }
            if let (Some(check), Some(rails)) = (&config.walking.handrail, &selections.rails) {
                let along = handrails::Along {
                    object: &object.id,
                    stretch: WalkingStretch::Flight,
                    label: "the flight",
                    width,
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
            report(&mut evaluation, rule, &object.id, checks);
        }
        evaluation
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
            push(Check::Fail(format!(
                "the flight has {number} risers; {words} allowed"
            )));
        }
    }
    if let Some(maximum) = config.maximum_rise {
        let rise = flight.rise();
        let measured = format!("the flight rises {}", shown(rise.lower(), rise.upper()));
        push(
            match judge(rise.lower(), rise.upper(), None, Some(maximum + slack)) {
                Verdict::Pass => Check::Pass,
                Verdict::Fail(_) => {
                    Check::Fail(format!("{measured}; at most {} allowed", metres(maximum)))
                }
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
    if config.forbid_open_risers {
        checks.push(closed(&flight.riser_closures()));
    }
    checks
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
        })
    }
}

impl RuleCapability for RampGeometryCheck {
    fn id(&self) -> &'static str {
        "axioval:capability.ramp-geometry"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::optional("slope_limits", ParameterType::Table(SLOPE_LIMITS)),
            ParameterDescriptor::optional("slope_tolerance", ParameterType::Number),
        ];
        parameters.extend(headroom_descriptors());
        parameters.extend(walking_descriptors());
        parameters.extend(ramp_ends::descriptors());
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
        for object in selected {
            let checks = ramp(stairs, free, &config, &selections, object);
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

fn ramp(
    stairs: &WalkingSurfaceServiceHandle,
    free: Option<&FreeSpaceServiceHandle>,
    config: &RampConfig<'_>,
    selections: &Selections,
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
    if let (Some(check), Some(candidates)) = (&config.walking.landing, &selections.landings) {
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
                };
                let (found, measured) = landing(stairs, check, candidates, &at);
                let mut found = found;
                if let (Some(doors), Some(selected), Some(measured)) =
                    (&config.walking.doors, &selections.doors, &measured)
                {
                    let elevation = if place == "top" {
                        run.top()
                    } else {
                        run.bottom()
                    };
                    found.push(ramp_ends::doors(
                        free, doors, selected, measured, elevation, &label,
                    ));
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
    for (check, mut cited, related) in
        rails_and_ends(stairs, free, config, selections, object, runs)
    {
        cited.insert(0, evidence.clone());
        checks.push((check, cited, related));
    }
    Ok(checks)
}

/// The handrails along each run of a ramp and the free space at its ends.
fn rails_and_ends(
    stairs: &WalkingSurfaceServiceHandle,
    free: Option<&FreeSpaceServiceHandle>,
    config: &RampConfig<'_>,
    selections: &Selections,
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
            };
            checks.extend(handrails::handrails(stairs, check, rails, &along));
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
        Check::Fail(format!("{measured}; required {rows}"))
    } else {
        Check::Undecided(format!(
            "{measured}, which straddles a slope limit ({rows})"
        ))
    }
}
