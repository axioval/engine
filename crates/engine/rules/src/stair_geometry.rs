//! `stair-geometry` and `ramp-geometry`: step dimensions, ramp slopes and
//! headroom measured from each flight's or ramp's body.
//!
//! Both judge the positions a [`WalkingSurfaceServiceHandle`] measured. Every
//! length and slope is an interval: a verdict needs the whole interval on
//! one side of its bound, widened by a few units in the last place for the
//! binary rounding of decimal coordinates; one straddling it leaves that
//! check not evaluated. Each check that fails is its own finding, so a
//! flight with an irregular riser and too little headroom is found twice.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use axioval_engine::{
    CapabilityEvaluation, ColumnKind, CompiledRule, HeadroomRequest, MeasuredInterval,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, RuleCapability, RuleContext, SlopedRun,
    TableColumn, TreadFlight, WalkingSurfaceError, WalkingSurfaceServiceHandle,
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
/// goings the distances between consecutive nosings along the direction it
/// climbs. Every declared check is judged on its own:
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
/// - `minimum_headroom` bounds the vertical clearance above the treads to
///   the `headroom_obstacles` the rule selects.
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
/// `minimum_headroom` the clearance to the `headroom_obstacles`.
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
fn every(label: &str, values: &[MeasuredInterval], (minimum, maximum): Range, slack: f64) -> Check {
    let (low, high) = (minimum.map(|m| m - slack), maximum.map(|m| m + slack));
    let total = values.len();
    let mut failing = Vec::new();
    let mut undecided = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let named = format!(
            "{label} {} of {total} is {}",
            index + 1,
            shown(value.lower(), value.upper())
        );
        match judge(value.lower(), value.upper(), low, high) {
            Verdict::Pass => {}
            Verdict::Fail(_) => failing.push(named),
            Verdict::Undecided(_) => undecided.push(named),
        }
    }
    let bound = bound_words(minimum, maximum, metres);
    if !failing.is_empty() {
        Check::Fail(format!("{}; {bound} required", failing.join(", ")))
    } else if !undecided.is_empty() {
        Check::Undecided(format!("{}, which straddles {bound}", undecided.join(", ")))
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

/// The decided obstacles and whether the selector left any undecided.
fn obstacles(
    context: &RuleContext<'_>,
    selector: &Selector,
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
                    format!(
                        "headroom obstacle selection is undecided: {}",
                        outcome.message()
                    ),
                ));
            }
        }
    }
    Ok((
        objects.iter().map(|object| object.id.clone()).collect(),
        !undecided.is_empty(),
    ))
}

/// The headroom above `object` against the rule's minimum.
///
/// An obstacle the selector could not decide can only lower the headroom:
/// too little headroom stands, enough is not evaluated.
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
    let evidence = vec![measured.evidence().clone()];
    let governing = measured.governing().to_vec();
    let required = format!("at least {} required", metres(check.minimum));
    let pending = if *undecided {
        "; an obstacle the selection could not decide may lower it"
    } else {
        ""
    };
    let Some(clearance) = measured.clearance() else {
        let check = if *undecided {
            Check::Undecided(format!(
                "nothing selected stands above the walking surface{pending}"
            ))
        } else {
            Check::Pass
        };
        return (check, evidence, governing);
    };
    let under = governing
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let measured = format!(
        "headroom above the walking surface is {} under {under}",
        shown(clearance.lower(), clearance.upper())
    );
    let check = match judge(
        clearance.lower(),
        clearance.upper(),
        Some(check.minimum),
        None,
    ) {
        Verdict::Fail(_) => Check::Fail(format!("{measured}; {required}")),
        Verdict::Pass if !*undecided => Check::Pass,
        Verdict::Pass => Check::Undecided(format!("{measured}{pending}")),
        Verdict::Undecided(_) => {
            Check::Undecided(format!("{measured}, which straddles {required}"))
        }
    };
    (check, evidence, governing)
}

struct StairConfig<'a> {
    riser: Range,
    going: Range,
    step_length: Range,
    nosing: Range,
    risers: (Option<usize>, Option<usize>),
    maximum_rise: Option<f64>,
    riser_tolerance: Option<f64>,
    going_tolerance: Option<f64>,
    headroom: Option<HeadroomCheck<'a>>,
}

impl<'a> StairConfig<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let config = Self {
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
            && config.headroom.is_none()
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
        ]);
        parameters.extend(headroom_descriptors());
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
        let obstacles = config
            .headroom
            .as_ref()
            .map(|check| obstacles(context, check.obstacles));
        for object in selected {
            let flight = match stairs.measure_tread_flight(&object.id) {
                Ok(flight) => flight,
                Err(error) => {
                    let (reason, message) = service_error(&error);
                    evaluation.push_object_not_evaluated(object.id.clone(), reason, message);
                    continue;
                }
            };
            let mut checks = stair_checks(&config, &flight);
            if let (Some(check), Some(obstacles)) = (&config.headroom, &obstacles) {
                checks.push(headroom(stairs, check, obstacles, &object.id));
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
            .max(tread.elevation().upper_metres().abs());
    }
    scale
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
    checks
}

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
        if limits.is_empty() && slope_tolerance.is_none() && headroom.is_none() {
            return Err(invalid(
                "declare `slope_limits`, `slope_tolerance` or `minimum_headroom`",
            ));
        }
        Ok(Self {
            limits,
            slope_tolerance,
            headroom,
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
        let obstacles = config
            .headroom
            .as_ref()
            .map(|check| obstacles(context, check.obstacles));
        for object in selected {
            let checks = ramp(stairs, &config, obstacles.as_ref(), object);
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
    config: &RampConfig<'_>,
    obstacles: Option<&Result<(Vec<ObjectId>, bool), Unavailable>>,
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
    if let (Some(check), Some(obstacles)) = (&config.headroom, obstacles) {
        let (check, mut cited, related) = headroom(stairs, check, obstacles, &object.id);
        cited.insert(0, evidence);
        checks.push((check, cited, related));
    }
    Ok(checks)
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
    .fold(0.0_f64, |scale, value| scale.max(value.abs()))
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
