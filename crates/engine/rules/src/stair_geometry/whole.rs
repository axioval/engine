//! `stair-geometry`'s whole-stair mode: with `stair_path`, the rule selects
//! stairs, reaches each one's flights along that path, checks every flight
//! as a single flight and the stair as a whole: its total rise, and the
//! handrail along each side continuing across the landings between its
//! flights, except where a selected door stands.
//!
//! Consecutive flights are the stair's flights in the order of their bases,
//! each arriving at the level the next one starts from. The handrail along a
//! side continues across a landing when the last piece along that side of
//! the lower flight and the first piece along the same side of the upper
//! one (seen climbing both) are joined by a chain of selected rails, each
//! within `handrail_gap_maximum` of the next (touching without it), as the
//! proximity service measures them in space.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, Deviation, ElevationInterval, HandrailEvidence,
    LandingRequest, ParameterDescriptor, ParameterType, ProximityRequest, ProximityServiceHandle,
    RailSide, RuleContext, TreadFlight, WalkingEnd, WalkingStretch,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use super::handrails::{self, HandrailCheck};
use super::ramp_ends;
use super::{
    Check, Checks, Flights, Selected, landing_level, length, report, selected, service_error, slack,
};
use crate::counts::Population;
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, judge};
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// Flights meet at a landing when the level one arrives at and the next
/// starts from lie within this of each other: the rounding of modelled
/// elevations, never a step.
const MEETING: f64 = 1e-3;

/// The whole-stair mode's declaration.
pub(super) struct StairMode<'a> {
    path: Traversal<'a>,
    flights: &'a Selector,
    maximum_total_rise: Option<f64>,
    /// The doors that break the handrail's continuity where they stand, and
    /// the height of the column over a landing they must reach into.
    break_doors: Option<(&'a Selector, f64)>,
}

impl StairMode<'_> {
    /// Whether the mode declares a check of its own.
    pub(super) fn declared(&self) -> bool {
        self.maximum_total_rise.is_some()
    }
}

pub(super) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("stair_path", ParameterType::StringList),
        ParameterDescriptor::optional("stair_flights", ParameterType::Selector),
        ParameterDescriptor::optional("maximum_total_rise", ParameterType::Quantity),
        ParameterDescriptor::optional(
            "handrail_continuous_across_landings",
            ParameterType::Boolean,
        ),
        ParameterDescriptor::optional("handrail_break_doors", ParameterType::Selector),
    ]
}

/// The whole-stair mode the rule declares, `None` without `stair_path`.
/// `handrail` is the rule's handrail check, `landings` whether it declares
/// `landing_objects`.
pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
    handrail: Option<&HandrailCheck<'_>>,
    landings: bool,
) -> Result<Option<StairMode<'a>>, Unavailable> {
    let path = parameters.strings("stair_path")?;
    let flights = parameters.selector("stair_flights")?;
    let maximum_total_rise = length(parameters, "maximum_total_rise")?;
    let continuous = handrail.is_some_and(|handrail| handrail.continuous);
    let doors = parameters.selector("handrail_break_doors")?;
    let Some(path) = path else {
        if flights.is_some() || maximum_total_rise.is_some() || continuous || doors.is_some() {
            return Err(invalid(
                "`stair_flights`, `maximum_total_rise`, `handrail_continuous_across_landings` \
                 and `handrail_break_doors` need `stair_path`",
            ));
        }
        return Ok(None);
    };
    let flights = flights.ok_or_else(|| invalid("`stair_path` needs `stair_flights`"))?;
    let break_doors = match doors {
        None => None,
        Some(_) if !continuous => {
            return Err(invalid(
                "`handrail_break_doors` needs `handrail_continuous_across_landings`",
            ));
        }
        Some(_) if !landings => {
            return Err(invalid("`handrail_break_doors` needs `landing_objects`"));
        }
        Some(doors) => {
            let height = length(parameters, "landing_door_height")?
                .filter(|height| *height > 0.0)
                .ok_or_else(|| {
                    invalid("`handrail_break_doors` needs a positive `landing_door_height`")
                })?;
            Some((doors, height))
        }
    };
    Ok(Some(StairMode {
        path: Traversal::path(path)?,
        flights,
        maximum_total_rise,
        break_doors,
    }))
}

/// Whether something holds: surely, possibly, or surely not.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tri {
    No,
    Maybe,
    Sure,
}

/// Checks every selected stair and, once each, the flights it reaches.
pub(super) fn evaluate(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    mode: &StairMode<'_>,
    flights: &Flights<'_, '_>,
    stairs: &[&Object],
    evaluation: &mut CapabilityEvaluation,
) {
    let population = Population::of(context, mode.flights);
    let everything: Vec<&Object> = context.project.objects().collect();
    let doors = mode
        .break_doors
        .map(|(doors, _)| selected(context, doors, "break door selection"));
    let mut measured: BTreeMap<ObjectId, Result<TreadFlight, Unavailable>> = BTreeMap::new();
    let mut reported = BTreeSet::new();
    for stair in stairs {
        let (parts, cited) = match mode.path.related(context, &stair.id, &everything) {
            Ok(reached) => reached,
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(stair.id.clone(), reason, message);
                continue;
            }
        };
        let undecided: Vec<&ObjectId> = parts
            .iter()
            .filter(|part| population.undecided.contains(*part))
            .collect();
        let ids: Vec<&ObjectId> = parts
            .iter()
            .filter(|part| population.matched.contains(*part))
            .collect();
        for id in &ids {
            let flight = measured
                .entry((*id).clone())
                .or_insert_with(|| flights.measure(id));
            if reported.insert((*id).clone()) {
                match flight {
                    Ok(flight) => report(evaluation, rule, id, flights.checks(flight)),
                    Err((reason, message)) => evaluation.push_object_not_evaluated(
                        (*id).clone(),
                        reason.clone(),
                        message.clone(),
                    ),
                }
            }
        }
        let mut missing: Vec<String> = undecided
            .iter()
            .map(|part| format!("whether {part} is one of its flights is undecided"))
            .collect();
        let mut ok = Vec::new();
        for id in &ids {
            match &measured[*id] {
                Ok(flight) => ok.push(flight),
                Err((_, message)) => {
                    missing.push(format!("flight {id} is not measured: {message}"));
                }
            }
        }
        if ok.is_empty() && missing.is_empty() {
            missing.push(format!(
                "{} reaches no selected flight",
                mode.path.relationship
            ));
        }
        let whole = Whole {
            context,
            mode,
            flights,
            ok: &ok,
            missing: &missing,
            doors: doors.as_ref(),
        };
        let mut checks = whole.checks();
        for (_, evidence, _) in &mut checks {
            evidence.extend(cited.iter().cloned());
            evidence.extend(ok.iter().map(|flight| flight.evidence().clone()));
        }
        report(evaluation, rule, &stair.id, checks);
    }
}

/// One stair's flights and how the rule judges them together.
struct Whole<'w, 's, 'a> {
    context: &'w RuleContext<'w>,
    mode: &'w StairMode<'a>,
    flights: &'w Flights<'s, 'a>,
    /// The flights measured.
    ok: &'w [&'w TreadFlight],
    /// Why flights may be missing: undecided or unmeasured ones.
    missing: &'w [String],
    doors: Option<&'w Selected>,
}

impl Whole<'_, '_, '_> {
    fn checks(&self) -> Checks {
        let mut checks = Vec::new();
        if let Some(maximum) = self.mode.maximum_total_rise {
            checks.push((self.total_rise(maximum), vec![], vec![]));
        }
        if let Some(check) = &self.flights.config.walking.handrail
            && check.continuous
        {
            checks.extend(self.continuity(check));
        }
        checks
    }

    /// The stair's rise, from its lowest flight's base to its highest
    /// flight's top, against the maximum. A flight that may be missing can
    /// only raise it: too high stands, a pass does not.
    fn total_rise(&self, maximum: f64) -> Check {
        let Some(lowest) = self.ok.iter().map(|flight| flight.base()).reduce(lower_of) else {
            return Check::Undecided(format!(
                "the stair's rise is not measured: {}",
                self.missing.join("; ")
            ));
        };
        let highest = self
            .ok
            .iter()
            .map(|flight| flight.top())
            .reduce(higher_of)
            .unwrap_or(lowest);
        let lower = (highest.lower_metres() - lowest.upper_metres()).next_down();
        let upper = (highest.upper_metres() - lowest.lower_metres()).next_up();
        let scale = highest
            .upper_metres()
            .abs()
            .max(lowest.lower_metres().abs());
        let measured = format!(
            "the stair rises {} from its lowest flight's base to its highest flight's top",
            shown(lower, upper)
        );
        let allowed = format!("at most {} allowed", metres(maximum));
        match judge(lower, upper, None, Some(maximum + slack(scale))) {
            Verdict::Fail(_) => Check::failed(
                format!("{measured}; {allowed}"),
                Some(Deviation::above(maximum, lower, upper)),
            ),
            Verdict::Pass if self.missing.is_empty() => Check::Pass,
            Verdict::Pass => {
                Check::Undecided(format!("{measured}, but {}", self.missing.join("; ")))
            }
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {allowed}"))
            }
        }
    }

    /// The handrail along each side across every landing between
    /// consecutive flights.
    fn continuity(&self, check: &HandrailCheck<'_>) -> Checks {
        if !self.missing.is_empty() {
            return vec![(
                Check::Undecided(format!(
                    "whether the handrails continue across the stair's landings is not judged: {}",
                    self.missing.join("; ")
                )),
                vec![],
                vec![],
            )];
        }
        let mut order: Vec<&TreadFlight> = self.ok.to_vec();
        order.sort_by(|a, b| {
            middle(a.base())
                .total_cmp(&middle(b.base()))
                .then_with(|| a.object().cmp(b.object()))
        });
        let mut checks = Vec::new();
        for pair in order.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            checks.extend(self.across(check, lower, upper));
        }
        checks
    }

    /// The handrail along each side across the landing between `lower` and
    /// `upper`, the flight that starts where `lower` arrives.
    fn across(
        &self,
        check: &HandrailCheck<'_>,
        lower: &TreadFlight,
        upper: &TreadFlight,
    ) -> Checks {
        let (a, b) = (lower.object(), upper.object());
        let named = format!("the landing between {a} and {b}");
        let related = vec![a.clone(), b.clone()];
        let arrives = lower.top();
        let starts = upper.base();
        if arrives.lower_metres() - MEETING > starts.upper_metres()
            || starts.lower_metres() - MEETING > arrives.upper_metres()
            || (upper.base().lower_metres() <= lower.base().upper_metres())
        {
            return vec![(
                Check::Undecided(format!(
                    "{a} and {b} do not meet at one level, so the handrail across {named} is not \
                     judged"
                )),
                vec![],
                related,
            )];
        }
        let Some(Ok((rails, undecided))) = &self.flights.selections.rails else {
            let message = match &self.flights.selections.rails {
                Some(Err((_, message))) => message.clone(),
                _ => "no handrail is selected".into(),
            };
            return vec![(Check::Undecided(message), vec![], related)];
        };
        let measure = |flight: &TreadFlight| {
            handrails::measure(
                self.flights.stairs,
                check,
                rails,
                &handrails::Along {
                    object: flight.object(),
                    stretch: WalkingStretch::Flight,
                    label: "the flight",
                    width: flight.width(),
                    risers: None,
                },
            )
        };
        let (below, above) = match (measure(lower), measure(upper)) {
            (Ok(below), Ok(above)) => (below, above),
            (Err(message), _) | (_, Err(message)) => {
                return vec![(Check::Undecided(message), vec![], related)];
            }
        };
        let evidence = [below.evidence().clone(), above.evidence().clone()];
        let mut exemption: Option<(Tri, String, Vec<Evidence>)> = None;
        let mut checks = Vec::new();
        for side in [RailSide::Left, RailSide::Right] {
            let words = match side {
                RailSide::Left => "left",
                RailSide::Right => "right",
            };
            let judged = self.side(check, (&below, &above), side, (rails, *undecided), &named);
            let (verdict, message, mut cited, mut objects) = match judged {
                Side::Nothing => continue,
                Side::Joined => (Tri::No, String::new(), vec![], vec![]),
                Side::Broken(sure, message, cited, rails) => (sure, message, cited, rails),
            };
            if verdict == Tri::No {
                continue;
            }
            let (exempt, why, cited_doors) = exemption
                .get_or_insert_with(|| self.exempt(lower, check, &named))
                .clone();
            cited.extend(cited_doors);
            cited.extend(evidence.iter().cloned());
            objects.extend(related.iter().cloned());
            let message = format!("the handrail along the {words} side {message}");
            checks.push(match (verdict, exempt) {
                (_, Tri::Sure) => continue,
                (Tri::Sure, Tri::No) => (Check::Fail(message), cited, objects),
                (_, Tri::Maybe) => (
                    Check::Undecided(format!("{message}; {why}")),
                    cited,
                    objects,
                ),
                (_, _) => (Check::Undecided(message), cited, objects),
            });
        }
        checks
    }

    /// Whether the handrail along one side continues across a landing.
    fn side(
        &self,
        check: &HandrailCheck<'_>,
        (below, above): (&HandrailEvidence, &HandrailEvidence),
        side: RailSide,
        (rails, undecided): (&[ObjectId], bool),
        named: &str,
    ) -> Side {
        let last = match below.side_rail(side) {
            Ok(pieces) => pieces.last().map(|(rail, _)| (*rail).clone()),
            Err(_) => {
                return Side::Broken(
                    Tri::Maybe,
                    format!("cannot be put in order below {named}"),
                    vec![],
                    vec![],
                );
            }
        };
        let first = match above.side_rail(side) {
            Ok(pieces) => pieces.first().map(|(rail, _)| (*rail).clone()),
            Err(_) => {
                return Side::Broken(
                    Tri::Maybe,
                    format!("cannot be put in order above {named}"),
                    vec![],
                    vec![],
                );
            }
        };
        let sure = if undecided { Tri::Maybe } else { Tri::Sure };
        match (last, first) {
            (None, None) => Side::Nothing,
            (Some(rail), None) | (None, Some(rail)) => Side::Broken(
                sure,
                format!("runs along one flight only and stops at {named} ({rail})"),
                vec![],
                vec![rail],
            ),
            (Some(from), Some(to)) => {
                let gap = check.gap();
                let (joined, cited) = self.joined(rails, &from, &to, gap);
                match (joined, undecided) {
                    (Tri::Sure, _) => Side::Joined,
                    (Tri::No, false) => Side::Broken(
                        Tri::Sure,
                        format!(
                            "stops at {named}: {from} and {to} are not joined by selected rails \
                             within {} of each other",
                            metres(gap)
                        ),
                        cited,
                        vec![from, to],
                    ),
                    _ => Side::Broken(
                        Tri::Maybe,
                        format!(
                            "may stop at {named}: whether {from} and {to} are joined by rails \
                             within {} of each other is not decided",
                            metres(gap)
                        ),
                        cited,
                        vec![from, to],
                    ),
                }
            }
        }
    }

    /// Whether a chain of selected rails, each within `gap` of the next,
    /// joins `from` to `to`: surely through measured touching pairs, surely
    /// not when no pair that may touch leads there.
    fn joined(
        &self,
        rails: &[ObjectId],
        from: &ObjectId,
        to: &ObjectId,
        gap: f64,
    ) -> (Tri, Vec<Evidence>) {
        if from == to {
            return (Tri::Sure, vec![]);
        }
        let proximity = self.context.services.get::<ProximityServiceHandle>();
        let mut nodes: Vec<ObjectId> = rails.to_vec();
        for end in [from, to] {
            if !nodes.contains(end) {
                nodes.push(end.clone());
            }
        }
        let mut edges: BTreeMap<(usize, usize), Tri> = BTreeMap::new();
        let mut cited = Vec::new();
        let mut edge = |a: usize, b: usize, cited: &mut Vec<Evidence>| -> Tri {
            let key = (a.min(b), a.max(b));
            if let Some(known) = edges.get(&key) {
                return *known;
            }
            let found = touching(proximity, &nodes[key.0], &nodes[key.1], gap, cited);
            edges.insert(key, found);
            found
        };
        let reach = |least: Tri, edge: &mut dyn FnMut(usize, usize) -> Tri| -> bool {
            let start = nodes.iter().position(|node| node == from).unwrap_or(0);
            let goal = nodes.iter().position(|node| node == to).unwrap_or(0);
            let mut seen = vec![false; nodes.len()];
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(at) = stack.pop() {
                if at == goal {
                    return true;
                }
                for (next, visited) in seen.iter_mut().enumerate() {
                    if !*visited && edge(at, next) >= least {
                        *visited = true;
                        stack.push(next);
                    }
                }
            }
            false
        };
        let mut sure_edge = |a: usize, b: usize| edge(a, b, &mut cited);
        if reach(Tri::Sure, &mut sure_edge) {
            return (Tri::Sure, cited);
        }
        let mut any_edge = |a: usize, b: usize| edge(a, b, &mut cited);
        let possible = reach(Tri::Maybe, &mut any_edge);
        (if possible { Tri::Maybe } else { Tri::No }, cited)
    }

    /// Whether a selected break door stands at the landing `lower` arrives
    /// at: reaches into the column over the landing's rectangle grown by
    /// the rails' reach across, where a handrail would run.
    fn exempt(
        &self,
        lower: &TreadFlight,
        check: &HandrailCheck<'_>,
        named: &str,
    ) -> (Tri, String, Vec<Evidence>) {
        let (Some((_, height)), Some(doors)) = (self.mode.break_doors, self.doors) else {
            return (Tri::No, String::new(), vec![]);
        };
        let Some(Ok((candidates, _))) = &self.flights.selections.landings else {
            return (
                Tri::Maybe,
                "the landing selection is undecided, so a door there is not looked for".into(),
                vec![],
            );
        };
        let request = LandingRequest::new(
            lower.object().clone(),
            WalkingEnd::FlightTop,
            candidates.iter().cloned(),
        );
        let measured = match self.flights.stairs.measure_landing(&request) {
            Ok(measured) => measured,
            Err(error) => {
                return (
                    Tri::Maybe,
                    format!(
                        "whether a door stands at {named} is not known: {}",
                        service_error(&error).1
                    ),
                    vec![],
                );
            }
        };
        if measured.landing().is_none() {
            return (Tri::No, String::new(), vec![measured.evidence().clone()]);
        }
        let level = landing_level(lower, WalkingEnd::FlightTop);
        let placed = match ramp_ends::landing_column(&measured, level, check.reach(), height, named)
        {
            Ok(placed) => placed,
            Err(why) => {
                return (
                    Tri::Maybe,
                    format!("whether a door stands there is not known: {why}"),
                    vec![measured.evidence().clone()],
                );
            }
        };
        let what = format!("the column over {named}");
        let (found, mut evidence, _) = ramp_ends::reaches_into(
            self.flights.free,
            &placed,
            lower.object(),
            doors,
            &what,
            ToOwned::to_owned,
        );
        evidence.insert(0, measured.evidence().clone());
        match found {
            Check::Fail(_) | Check::Graded(..) => (Tri::Sure, String::new(), evidence),
            Check::Pass => (Tri::No, String::new(), evidence),
            Check::Undecided(message) => (
                Tri::Maybe,
                format!("a selected door may stand there: {message}"),
                evidence,
            ),
        }
    }
}

/// Where the handrail along one side stands across a landing.
enum Side {
    /// Neither flight has a rail along it.
    Nothing,
    /// Its pieces are joined across the landing.
    Joined,
    /// It surely or possibly stops there: why, with the evidence and the
    /// rails involved.
    Broken(Tri, String, Vec<Evidence>, Vec<ObjectId>),
}

/// Whether two rails lie within `gap` of each other in space.
fn touching(
    proximity: Option<&ProximityServiceHandle>,
    a: &ObjectId,
    b: &ObjectId,
    gap: f64,
    cited: &mut Vec<Evidence>,
) -> Tri {
    let Some(proximity) = proximity else {
        return Tri::Maybe;
    };
    let margin = gap + slack(gap);
    if let (Ok(first), Ok(second)) = (proximity.bounds(a), proximity.bounds(b))
        && first.enclosing().gap(&second.enclosing()) > margin
    {
        return Tri::No;
    }
    let Ok(request) = ProximityRequest::try_new(a.clone(), b.clone()) else {
        return Tri::Maybe;
    };
    match proximity.measure_distance(&request) {
        Ok(measured) => {
            let (lower, upper) = measured.interval_metres();
            if upper <= margin {
                cited.push(measured.evidence().clone());
                Tri::Sure
            } else if lower > margin {
                Tri::No
            } else {
                Tri::Maybe
            }
        }
        Err(_) => Tri::Maybe,
    }
}

fn middle(value: ElevationInterval) -> f64 {
    f64::midpoint(value.lower_metres(), value.upper_metres())
}

/// The interval holding the lower of two elevations.
fn lower_of(a: ElevationInterval, b: ElevationInterval) -> ElevationInterval {
    ElevationInterval::try_new(
        a.lower_metres().min(b.lower_metres()),
        a.upper_metres().min(b.upper_metres()),
    )
    .unwrap_or(a)
}

/// The interval holding the higher of two elevations.
fn higher_of(a: ElevationInterval, b: ElevationInterval) -> ElevationInterval {
    ElevationInterval::try_new(
        a.lower_metres().max(b.lower_metres()),
        a.upper_metres().max(b.upper_metres()),
    )
    .unwrap_or(a)
}
