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

use axioval_engine::{
    ElevationInterval, FreeSpaceServiceHandle, LandingRequest, ParameterDescriptor, ParameterType,
    ProximityServiceHandle, RailSide, RuleContext, TreadFlight, WalkingEnd, WalkingStretch,
    WalkingSurfaceServiceHandle,
};
use axioval_ir::Evidence;

use super::continuity::{Continuity, Side, Tri};
use super::handrails::{self, HandrailCheck};
use super::ramp_ends;
use super::{Check, Checks, Selected, landing_level, service_error};

/// Flights meet at a landing when the level one arrives at and the next
/// starts from lie within this of each other: the rounding of modelled
/// elevations, never a step.
pub(super) const MEETING: f64 = 1e-3;

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

/// What the handrail across a stair's landings is judged with: the
/// services, the selections of rails, landings and break doors, and the
/// height of the column a break door must reach into.
pub(super) struct Across<'a> {
    pub(super) context: &'a RuleContext<'a>,
    pub(super) stairs: &'a WalkingSurfaceServiceHandle,
    pub(super) free: Option<&'a FreeSpaceServiceHandle>,
    pub(super) rails: Option<&'a Selected>,
    pub(super) landings: Option<&'a Selected>,
    pub(super) doors: Option<&'a Selected>,
    pub(super) break_height: Option<f64>,
}

impl Across<'_> {
    /// The handrail along each side across every landing between
    /// consecutive flights.
    pub(super) fn continuity(
        &self,
        check: &HandrailCheck<'_>,
        flights: &[&TreadFlight],
        missing: &[String],
    ) -> Checks {
        if !missing.is_empty() {
            return vec![(
                Check::Undecided(format!(
                    "whether the handrails continue across the stair's landings is not judged: {}",
                    missing.join("; ")
                )),
                vec![],
                vec![],
            )];
        }
        let mut checks = Vec::new();
        for pair in ordered(flights).windows(2) {
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
        if !meets(lower, upper) {
            return vec![(
                Check::Undecided(format!(
                    "{a} and {b} do not meet at one level, so the handrail across {named} is not \
                     judged"
                )),
                vec![],
                related,
            )];
        }
        let Some(Ok((rails, undecided))) = &self.rails else {
            let message = match &self.rails {
                Some(Err((_, message))) => message.clone(),
                _ => "no handrail is selected".into(),
            };
            return vec![(Check::Undecided(message), vec![], related)];
        };
        let measure = |flight: &TreadFlight| {
            handrails::measure(
                self.stairs,
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
            let continuity = Continuity {
                proximity: self.context.services.get::<ProximityServiceHandle>(),
                rails,
                undecided: *undecided,
                gap: check.gap(),
                noun: "flight",
            };
            let judged = continuity.side((&below, &above), side, &named);
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

    /// Whether a selected break door stands at the landing `lower` arrives
    /// at: reaches into the column over the landing's rectangle grown by
    /// the rails' reach across, where a handrail would run.
    fn exempt(
        &self,
        lower: &TreadFlight,
        check: &HandrailCheck<'_>,
        named: &str,
    ) -> (Tri, String, Vec<Evidence>) {
        let (Some(height), Some(doors)) = (self.break_height, self.doors) else {
            return (Tri::No, String::new(), vec![]);
        };
        let Some(Ok((candidates, _))) = &self.landings else {
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
        let measured = match self.stairs.measure_landing(&request) {
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
            self.free,
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

/// A stair's flights in the order of their bases.
pub(super) fn ordered<'f>(flights: &[&'f TreadFlight]) -> Vec<&'f TreadFlight> {
    let mut order = flights.to_vec();
    order.sort_by(|a, b| {
        middle(a.base())
            .total_cmp(&middle(b.base()))
            .then_with(|| a.object().cmp(b.object()))
    });
    order
}

/// Whether `upper` starts where `lower` arrives, from a surely higher base:
/// the landing between them is an intermediate one.
pub(super) fn meets(lower: &TreadFlight, upper: &TreadFlight) -> bool {
    let (arrives, starts) = (lower.top(), upper.base());
    arrives.lower_metres() - MEETING <= starts.upper_metres()
        && starts.lower_metres() - MEETING <= arrives.upper_metres()
        && upper.base().lower_metres() > lower.base().upper_metres()
}

fn middle(value: ElevationInterval) -> f64 {
    f64::midpoint(value.lower_metres(), value.upper_metres())
}

/// A whole stair's rise from its lowest flight's base to its highest
/// flight's top, `[lower, upper]` sure to hold it, with those two
/// elevations.
pub(super) struct Rise {
    pub(super) lower: f64,
    pub(super) upper: f64,
    pub(super) lowest: ElevationInterval,
    pub(super) highest: ElevationInterval,
}

/// The rise of the stair `flights` make up, or `None` without a flight:
/// the one computation `maximum_total_rise` judges and the measured
/// `stair_rise` answers.
pub(super) fn rise<'f>(flights: impl Iterator<Item = &'f TreadFlight> + Clone) -> Option<Rise> {
    let lowest = flights.clone().map(TreadFlight::base).reduce(lower_of)?;
    let highest = flights
        .map(TreadFlight::top)
        .reduce(higher_of)
        .unwrap_or(lowest);
    Some(Rise {
        lower: (highest.lower_metres() - lowest.upper_metres()).next_down(),
        upper: (highest.upper_metres() - lowest.lower_metres()).next_up(),
        lowest,
        highest,
    })
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
