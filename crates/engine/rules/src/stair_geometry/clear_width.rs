//! The clear width of a flight or a ramp's run: the narrowest free width
//! across it that the `clear_width_obstacles` (handrails, walls, anything
//! beside or over it) leave between two heights above its pitch line, as
//! the walking-surface service measures it.
//!
//! A stair's landings are measured the same way above their level, across
//! the direction leaving the flight, between the obstacles bounding them
//! (`landing_clear_width_minimum`); a landing a side of which no
//! selected obstacle reaches is not evaluated. The least of a flight's and
//! its landings' clear widths, or in whole-stair mode of every flight and
//! landing between flights of the stair, is judged against
//! `total_clear_width_minimum`.

use axioval_engine::{
    ClearWidthRequest, Deviation, LandingClearWidthRequest, LandingRequest, MeasuredInterval,
    ParameterDescriptor, ParameterType, WalkingEnd, WalkingStretch, WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use super::{Check, Checks, Selected, length, service_error, slack};
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, judge};
use crate::support::{Parameters, Unavailable, invalid};

/// The clear-width checks of one rule.
pub(super) struct ClearWidthCheck<'a> {
    pub(super) obstacles: &'a Selector,
    minimum: Option<f64>,
    landing: Option<f64>,
    total: Option<f64>,
    band: (f64, f64),
}

impl ClearWidthCheck<'_> {
    /// Whether landings are measured: for their own minimum or the total.
    pub(super) fn landings(&self) -> bool {
        self.landing.is_some() || self.total.is_some()
    }

    /// Whether the flight or run itself is measured.
    fn stretches(&self) -> bool {
        self.minimum.is_some() || self.total.is_some()
    }

    /// The least of all flight and landing clear widths, when declared.
    pub(super) fn total(&self) -> Option<f64> {
        self.total
    }
}

pub(super) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("clear_width_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("clear_width_obstacles", ParameterType::Selector),
        ParameterDescriptor::optional("clear_width_band_from", ParameterType::Quantity),
        ParameterDescriptor::optional("clear_width_band_to", ParameterType::Quantity),
    ]
}

/// The landing and total clear widths, which only stairs take.
pub(super) fn stair_descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("landing_clear_width_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("total_clear_width_minimum", ParameterType::Quantity),
    ]
}

pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<ClearWidthCheck<'a>>, Unavailable> {
    let minimum = length(parameters, "clear_width_minimum")?;
    let landing = length(parameters, "landing_clear_width_minimum")?;
    let total = length(parameters, "total_clear_width_minimum")?;
    let obstacles = parameters.selector("clear_width_obstacles")?;
    let from = length(parameters, "clear_width_band_from")?;
    let to = length(parameters, "clear_width_band_to")?;
    let declared = minimum.is_some() || landing.is_some() || total.is_some();
    match (declared, obstacles, from, to) {
        (true, Some(obstacles), Some(from), Some(to)) if from < to => Ok(Some(ClearWidthCheck {
            obstacles,
            minimum,
            landing,
            total,
            band: (from, to),
        })),
        (false, None, None, None) => Ok(None),
        _ => Err(invalid(
            "a clear-width minimum (`clear_width_minimum`, \
             `landing_clear_width_minimum` or `total_clear_width_minimum`), \
             `clear_width_obstacles`, `clear_width_band_from` and `clear_width_band_to` are \
             declared together, the band's bottom below its top",
        )),
    }
}

impl<'a> ClearWidthCheck<'a> {
    /// A check measuring only, over `obstacles` between the heights `band`.
    pub(super) fn measuring(obstacles: &'a Selector, band: (f64, f64)) -> Self {
        Self {
            obstacles,
            minimum: None,
            landing: None,
            total: None,
            band,
        }
    }
}

impl Width {
    /// The measured width and whether its evidence is exact, or why it is
    /// not known.
    pub(super) fn interval(&self) -> Result<(MeasuredInterval, bool), String> {
        self.measured
            .as_ref()
            .map(|measured| (measured.width, measured.evidence.exact))
            .map_err(Clone::clone)
    }
}

/// What a clear width was measured over.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// The flight or run itself.
    Stretch,
    /// The landing at the bottom (`false`) or top (`true`) of a flight.
    Landing(bool),
}

/// One measured clear width, or why it is not known.
#[derive(Clone)]
pub(super) struct Width {
    place: Place,
    /// The flight or ramp it belongs to.
    pub(super) owner: ObjectId,
    /// The stretch or landing in a message: `the flight`, `run 1 of 2`.
    label: String,
    /// What the band stands on in a message: `its pitch line`, `its level`.
    above: &'static str,
    measured: Result<Measured, String>,
}

/// A measured clear width: its interval, the obstacles bounding its
/// narrowest place and the measurement.
#[derive(Clone)]
struct Measured {
    width: MeasuredInterval,
    governing: Vec<ObjectId>,
    evidence: Evidence,
}

impl Width {
    /// The stretch or landing in a message: `the flight`, `run 1 of 2`.
    pub(super) fn label(&self) -> &str {
        &self.label
    }

    /// What the band stands on in a message: `its pitch line`, `its level`.
    pub(super) fn above(&self) -> &'static str {
        self.above
    }

    /// Whether it is a landing's.
    pub(super) fn is_landing(&self) -> bool {
        matches!(self.place, Place::Landing(_))
    }

    /// The obstacles bounding its narrowest place, where measured.
    pub(super) fn governing(&self) -> &[ObjectId] {
        self.measured
            .as_ref()
            .map_or(&[], |measured| measured.governing.as_slice())
    }

    fn evidence(&self) -> Vec<Evidence> {
        self.measured
            .as_ref()
            .map(|measured| vec![measured.evidence.clone()])
            .unwrap_or_default()
    }

    /// The width named in a stair's total: the flight's own label replaced
    /// by the flight's identity.
    fn named(&self, owned: bool) -> String {
        if !owned {
            return self.label.clone();
        }
        match self.place {
            Place::Stretch => format!("flight {}", self.owner),
            Place::Landing(top) => format!(
                "the landing at the {} of flight {}",
                if top { "top" } else { "bottom" },
                self.owner
            ),
        }
    }
}

/// The clear width along one stretch, `label` naming it.
pub(super) fn stretch_width(
    stairs: &WalkingSurfaceServiceHandle,
    check: &ClearWidthCheck<'_>,
    candidates: &[ObjectId],
    (object, stretch): (&ObjectId, WalkingStretch),
    label: &str,
) -> Width {
    let measured = ClearWidthRequest::try_new(
        object.clone(),
        stretch,
        candidates.iter().cloned(),
        check.band,
    )
    .and_then(|request| stairs.measure_clear_width(&request))
    .map(|measured| Measured {
        width: measured.width(),
        governing: measured.governing().to_vec(),
        evidence: measured.evidence().clone(),
    })
    .map_err(|error| format!("the clear width of {label}: {}", service_error(&error).1));
    Width {
        place: Place::Stretch,
        owner: object.clone(),
        label: label.to_owned(),
        above: "its pitch line",
        measured,
    }
}

/// The clear width of the landing at one end of a flight: `None` when no
/// selected object carries one there and the landing selection is decided.
pub(super) fn landing_width(
    stairs: &WalkingSurfaceServiceHandle,
    check: &ClearWidthCheck<'_>,
    candidates: &[ObjectId],
    landings: &Selected,
    (object, top): (&ObjectId, bool),
) -> Option<Width> {
    let end = if top {
        WalkingEnd::FlightTop
    } else {
        WalkingEnd::FlightBottom
    };
    let label = format!(
        "the landing at the {} of the flight",
        if top { "top" } else { "bottom" }
    );
    let width = |measured: Result<Measured, String>| Width {
        place: Place::Landing(top),
        owner: object.clone(),
        label: label.clone(),
        above: "its level",
        measured,
    };
    let (carriers, undecided) = match landings {
        Ok(landings) => landings,
        Err((_, message)) => return Some(width(Err(message.clone()))),
    };
    let request = LandingClearWidthRequest::try_new(
        LandingRequest::new(object.clone(), end, carriers.iter().cloned()),
        candidates.iter().cloned(),
        check.band,
    );
    let measured = match request.and_then(|request| stairs.measure_landing_clear_width(&request)) {
        Ok(measured) => measured,
        Err(error) => {
            return Some(width(Err(format!(
                "the clear width of {label}: {}",
                service_error(&error).1
            ))));
        }
    };
    let Some(landing) = measured.landing() else {
        return undecided.then(|| {
            width(Err(format!(
                "no selected slab or landing meets {}, but an object the selection could not \
                 decide may carry one",
                label.trim_start_matches("the landing at ")
            )))
        });
    };
    let (low, high) = landing.bounds();
    let unbounded: Vec<&str> = [(low, "right"), (high, "left")]
        .iter()
        .filter(|(bounds, _)| bounds.is_empty())
        .map(|(_, side)| *side)
        .collect();
    if !unbounded.is_empty() {
        return Some(width(Err(format!(
            "no selected obstacle bounds the {} side of {label} ({}), so its clear width is not \
             measured",
            unbounded.join(" and the "),
            landing.carrier()
        ))));
    }
    Some(width(Ok(Measured {
        width: landing.width(),
        governing: landing.governing().to_vec(),
        evidence: measured.evidence().clone(),
    })))
}

/// The clear widths one flight's checks measure: the flight's own when
/// `clear_width_minimum` or the total asks for it, and its landings at
/// `ends` when the landing minimum or the total does.
pub(super) fn flight_widths(
    stairs: &WalkingSurfaceServiceHandle,
    check: &ClearWidthCheck<'_>,
    (obstacles, landings): (&Selected, Option<&Selected>),
    object: &ObjectId,
    ends: [bool; 2],
) -> Result<Vec<Width>, String> {
    let (candidates, _) = obstacles.as_ref().map_err(|(_, message)| message.clone())?;
    let mut widths = Vec::new();
    if check.stretches() {
        widths.push(stretch_width(
            stairs,
            check,
            candidates,
            (object, WalkingStretch::Flight),
            "the flight",
        ));
    }
    if check.landings()
        && let Some(landings) = landings
    {
        for (top, measured) in [(false, ends[0]), (true, ends[1])] {
            if measured {
                widths.extend(landing_width(
                    stairs,
                    check,
                    candidates,
                    landings,
                    (object, top),
                ));
            }
        }
    }
    Ok(widths)
}

/// Each width against its own minimum: a flight's or run's against
/// `clear_width_minimum`, a landing's against the landing minimum. An
/// obstacle the selection could not decide can only narrow it: too narrow
/// stands, wide enough is not evaluated.
pub(super) fn judge_each(check: &ClearWidthCheck<'_>, widths: &[Width], undecided: bool) -> Checks {
    let mut checks = Vec::new();
    for width in widths {
        let minimum = match width.place {
            Place::Stretch => check.minimum,
            Place::Landing(_) => check.landing,
        };
        let Some(minimum) = minimum else {
            continue;
        };
        let measured = match &width.measured {
            Ok(measured) => measured,
            Err(message) => {
                checks.push((Check::Undecided(message.clone()), vec![], vec![]));
                continue;
            }
        };
        let between = if measured.governing.is_empty() {
            "between its own sides".to_owned()
        } else {
            format!("beside {}", joined(&measured.governing))
        };
        let words = format!(
            "the clear width of {} {} to {} above {} is {} {between}",
            width.label,
            metres(check.band.0),
            metres(check.band.1),
            width.above,
            shown(measured.width.lower(), measured.width.upper())
        );
        let found = against(minimum, measured.width, undecided, &words, |pending| {
            format!("{words}; {pending}")
        });
        checks.push((found, width.evidence(), measured.governing.clone()));
    }
    checks
}

/// The least of `widths` against `minimum`, the total over `what` (`the
/// flight and its landings`); `owned` names each width's flight, and
/// `missing` says why widths may be missing. A width not measured can only
/// lower the least: too narrow stands, wide enough is not evaluated.
pub(super) fn judge_total(
    minimum: f64,
    widths: &[Width],
    undecided: bool,
    (what, owned): (&str, bool),
    missing: &[String],
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let mut unknown: Vec<String> = missing.to_vec();
    let mut known: Vec<(&Width, &Measured)> = Vec::new();
    for width in widths {
        match &width.measured {
            Ok(measured) => known.push((width, measured)),
            Err(message) => unknown.push(message.clone()),
        }
    }
    let evidence: Vec<Evidence> = widths.iter().flat_map(Width::evidence).collect();
    // The narrowest is the width of least upper bound; the least lies
    // between the least lower and the least upper bound.
    let Some((narrowest, measured)) = known
        .iter()
        .min_by(|a, b| a.1.width.upper().total_cmp(&b.1.width.upper()))
        .copied()
    else {
        return (
            Check::Undecided(format!(
                "the least clear width of {what} is not measured: {}",
                unknown.join("; ")
            )),
            evidence,
            vec![],
        );
    };
    let lower = known
        .iter()
        .map(|(_, measured)| measured.width.lower())
        .fold(f64::INFINITY, f64::min);
    let interval = MeasuredInterval::try_new(lower, measured.width.upper().max(lower))
        .unwrap_or(measured.width);
    let mut related = measured.governing.clone();
    if owned && !related.contains(&narrowest.owner) {
        related.push(narrowest.owner.clone());
    }
    let words = format!(
        "the least clear width of {what} is {}, at {}",
        shown(interval.lower(), interval.upper()),
        narrowest.named(owned)
    );
    let found = against(minimum, interval, undecided, &words, |pending| {
        format!("{words}; {pending}")
    });
    let found = match found {
        Check::Pass if !unknown.is_empty() => Check::Undecided(format!(
            "{words}, but {} may be narrower: {}",
            if unknown.len() == 1 {
                "a width"
            } else {
                "widths"
            },
            unknown.join("; ")
        )),
        other => other,
    };
    (found, evidence, related)
}

/// A width against a minimum, worded by `words`; `pending` words a pass an
/// undecided obstacle may narrow.
fn against(
    minimum: f64,
    width: MeasuredInterval,
    undecided: bool,
    words: &str,
    pending: impl Fn(&str) -> String,
) -> Check {
    let required = format!("at least {} required", metres(minimum));
    let slack = slack(width.upper());
    match judge(width.lower(), width.upper(), Some(minimum - slack), None) {
        Verdict::Fail(_) => Check::failed(
            format!("{words}; {required}"),
            Some(Deviation::below(minimum, width.lower(), width.upper())),
        ),
        Verdict::Pass if !undecided => Check::Pass,
        Verdict::Pass => Check::Undecided(pending(
            "an obstacle the selection could not decide may narrow it",
        )),
        Verdict::Undecided(_) => Check::Undecided(format!("{words}, which straddles {required}")),
    }
}

fn joined(objects: &[ObjectId]) -> String {
    objects
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" and ")
}
