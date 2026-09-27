//! The handrail checks `stair-geometry` and `ramp-geometry` share: the
//! height of each rail above the pitch line, the level extension of the
//! handrail along each side beyond the ends, the gaps between its pieces,
//! and the sides a rail runs along.
//!
//! The rails along one side are the pieces of one handrail, put in order by
//! the contract ([`HandrailEvidence::side_rail`]): its extension is its
//! first piece's at the bottom and its last piece's at the top, and the gap
//! between consecutive pieces is its continuity. Heights are each piece's.
//! Pieces the contract cannot put in order leave the side's extension and
//! continuity not evaluated.

use std::collections::BTreeSet;

use axioval_engine::{
    Deviation, HandrailEvidence, HandrailRequest, MeasuredInterval, ParameterDescriptor,
    ParameterType, RailMeasurement, RailSide, RiserClosure, Tread, TreadFlight, WalkingStretch,
    WalkingSurfaceServiceHandle,
};
use axioval_ir::ObjectId;
use axioval_ir::contract::Selector;

use super::{Check, Checks, Range, bound_words, length, range, service_error, slack};
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, deviation, judge};
use crate::support::{Parameters, Unavailable, invalid};

/// A rail's top rising or falling less than this over its extension is
/// level: the rounding placements leave in coordinates, not a slope.
const LEVEL_RISE: f64 = 1e-6;

/// The sides a handrail must run along.
#[derive(Clone, Copy)]
enum Required {
    One,
    Both,
    /// Both when the flight or run is wider than this, one otherwise.
    BothWiderThan(f64),
}

/// The handrail checks of one rule.
pub(super) struct HandrailCheck<'a> {
    pub(super) rails: &'a Selector,
    reach: f64,
    above: f64,
    height: Range,
    /// The least and greatest extension beyond each end.
    extension: Range,
    /// Whether a flight's extension is measured from its first and last
    /// riser rather than its nosings.
    from_riser: bool,
    gap: Option<f64>,
    sides: Option<Required>,
}

pub(super) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("handrail_objects", ParameterType::Selector),
        ParameterDescriptor::optional("handrail_reach_across", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_reach_above", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_height_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_height_maximum", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_extension_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_extension_maximum", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_gap_maximum", ParameterType::Quantity),
        ParameterDescriptor::optional("handrail_sides", ParameterType::String),
        ParameterDescriptor::optional("handrail_both_sides_above_width", ParameterType::Quantity),
    ]
}

/// The handrail checks the rule declares; `ramp` refuses measuring an
/// extension from a riser, which a ramp does not have.
pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
    ramp: bool,
) -> Result<Option<HandrailCheck<'a>>, Unavailable> {
    let rails = parameters.selector("handrail_objects")?;
    let reach = length(parameters, "handrail_reach_across")?;
    let above = length(parameters, "handrail_reach_above")?;
    let height = range(parameters, "handrail_height")?;
    let extension = range(parameters, "handrail_extension")?;
    let from_riser = match (parameters.string("handrail_extension_from")?, ramp) {
        (None | Some("nosing"), _) => false,
        (Some("riser"), false) => true,
        (Some("riser"), true) => {
            return Err(invalid(
                "`handrail_extension_from` `riser` applies only to stairs",
            ));
        }
        (Some(other), _) => {
            return Err(invalid(format!(
                "`handrail_extension_from` `{other}` is unsupported; use `nosing` or `riser`"
            )));
        }
    };
    if from_riser && extension == (None, None) {
        return Err(invalid(
            "`handrail_extension_from` needs `handrail_extension_minimum` or \
             `handrail_extension_maximum`",
        ));
    }
    let gap = length(parameters, "handrail_gap_maximum")?;
    let wider = length(parameters, "handrail_both_sides_above_width")?;
    let sides = match (parameters.string("handrail_sides")?, wider) {
        (None, None) => None,
        (Some("one"), None) => Some(Required::One),
        (Some("one"), Some(width)) => Some(Required::BothWiderThan(width)),
        (Some("both"), None) => Some(Required::Both),
        (Some("both") | None, Some(_)) => {
            return Err(invalid(
                "`handrail_both_sides_above_width` applies only to `handrail_sides` `one`",
            ));
        }
        (Some(other), _) => {
            return Err(invalid(format!(
                "`handrail_sides` `{other}` is unsupported; use `one` or `both`"
            )));
        }
    };
    let declared =
        height != (None, None) || extension != (None, None) || gap.is_some() || sides.is_some();
    match (rails, reach, above, declared) {
        (Some(rails), Some(reach), Some(above), true) => Ok(Some(HandrailCheck {
            rails,
            reach,
            above,
            height,
            extension,
            from_riser,
            gap,
            sides,
        })),
        (None, None, None, false) => Ok(None),
        (_, _, _, true) => Err(invalid(
            "a handrail check needs `handrail_objects`, `handrail_reach_across` and \
             `handrail_reach_above`",
        )),
        (_, _, _, false) => Err(invalid(
            "`handrail_objects` or a handrail reach is declared without a handrail check",
        )),
    }
}

/// One flight or run whose handrails are checked.
pub(super) struct Along<'a> {
    pub(super) object: &'a ObjectId,
    pub(super) stretch: WalkingStretch,
    /// The stretch in a message: `the flight`, `run 1 of 2`.
    pub(super) label: &'a str,
    /// Its width, when measured.
    pub(super) width: Option<MeasuredInterval>,
    /// Where a flight's first and last risers lie from its pitch line's
    /// ends; `None` for a ramp's run.
    pub(super) risers: Option<RiserOffsets>,
}

/// How far a flight's first riser lies beyond the pitch line's bottom end
/// (towards the top) and its last riser before the top end, as intervals,
/// or why they are not known. An extension measured from the riser is the
/// one from the pitch line plus the bottom offset, or less the top one.
pub(super) struct RiserOffsets {
    bottom: Result<(f64, f64), String>,
    top: Result<(f64, f64), String>,
}

impl RiserOffsets {
    /// The offsets of `flight`'s risers. The first riser lies under the
    /// first nosing when it is closed flush below it, and somewhere under
    /// the first tread when that is not measured; the last lies at the
    /// upper floor's edge when a closed final riser arrives there, else at
    /// the back of the tread below the last, the last tread's nosing
    /// overhanging it. An open riser is no riser to measure from.
    pub(super) fn of(flight: &TreadFlight) -> Self {
        let treads = flight.treads();
        let bottom = match treads.first() {
            None => Err("the flight has no tread".into()),
            Some(first) => match first.riser_below() {
                RiserClosure::Closed => Ok((0.0, 0.0)),
                RiserClosure::NotMeasured => Ok((0.0, first.depth().upper().max(0.0))),
                RiserClosure::Open => Err("the flight's first riser is open".into()),
            },
        };
        let closures = flight.riser_closures();
        let top = if flight.ends_in_riser() {
            match closures.last() {
                Some(RiserClosure::Closed) => Ok((0.0, 0.0)),
                Some(RiserClosure::Open) => Err("the flight's last riser is open".into()),
                _ => Err("where the flight's last riser lies is not measured".into()),
            }
        } else {
            match (
                treads.last().map(Tread::riser_below),
                flight.nosings().last(),
            ) {
                (Some(RiserClosure::Closed), Some(overhang)) => {
                    Ok((overhang.lower().max(0.0), overhang.upper().max(0.0)))
                }
                (Some(RiserClosure::Open), _) => Err("the flight's last riser is open".into()),
                _ => Err("where the flight's last riser lies is not measured".into()),
            }
        };
        Self { bottom, top }
    }
}

/// The handrails along one stretch against the rule's handrail checks.
pub(super) fn handrails(
    stairs: &WalkingSurfaceServiceHandle,
    check: &HandrailCheck<'_>,
    rails: &Result<(Vec<ObjectId>, bool), Unavailable>,
    along: &Along<'_>,
) -> Checks {
    let (candidates, undecided) = match rails {
        Ok(rails) => rails,
        Err((_, message)) => return vec![(Check::Undecided(message.clone()), vec![], vec![])],
    };
    let request = match HandrailRequest::try_new(
        along.object.clone(),
        along.stretch,
        candidates.iter().cloned(),
        (check.reach, check.above),
        check.extension.0.unwrap_or(0.0),
    ) {
        Ok(request) => request,
        Err(error) => {
            return vec![(
                Check::Undecided(format!("handrails: {}", service_error(&error).1)),
                vec![],
                vec![],
            )];
        }
    };
    let measured = match stairs.measure_handrails(&request) {
        Ok(measured) => measured,
        Err(error) => {
            return vec![(
                Check::Undecided(format!(
                    "handrails along {}: {}",
                    along.label,
                    service_error(&error).1
                )),
                vec![],
                vec![],
            )];
        }
    };
    let evidence = vec![measured.evidence().clone()];
    let judged = Judged {
        measured: &measured,
        slack: slack(scale(&measured)),
        along,
        undecided: *undecided,
        risers: if check.from_riser {
            along.risers.as_ref()
        } else {
            None
        },
    };
    let mut checks: Checks = Vec::new();
    let mut push = |check: Check, related: Vec<ObjectId>| {
        checks.push((check, evidence.clone(), related));
    };
    if check.height != (None, None) {
        let mut all = Vec::new();
        for (rail, measurement) in measured.rails() {
            all.extend(height(rail, measurement, check.height, judged.slack, along));
        }
        push_all(&mut push, all, judged.pending(Check::Pass));
    }
    if check.extension != (None, None) {
        push_all(
            &mut push,
            judged.extensions(check.extension),
            judged.pending(Check::Pass),
        );
    }
    if let Some(maximum) = check.gap {
        push_all(&mut push, judged.gaps(maximum), Check::Pass);
    }
    if let Some(required) = check.sides {
        let (check, related) = sides(&measured, required, judged.slack, along, *undecided);
        push(check, related);
    }
    checks
}

/// Pushes each failing or undecided check with the rails it names, or
/// `passing` when every one passes.
fn push_all(
    push: &mut impl FnMut(Check, Vec<ObjectId>),
    checks: Vec<(Check, Vec<ObjectId>)>,
    passing: Check,
) {
    let mut any = false;
    for (check, rails) in checks {
        if !matches!(check, Check::Pass) {
            any = true;
            push(check, rails);
        }
    }
    if !any && !matches!(passing, Check::Pass) {
        push(passing, vec![]);
    }
}

/// The largest magnitude among the positions of the measurement.
fn scale(measured: &HandrailEvidence) -> f64 {
    let (start, end) = measured.pitch();
    let mut positions = vec![start, end];
    for part in measured.parts() {
        let (left, right) = part.sides();
        positions.extend([left, right]);
    }
    for (_, rail) in measured.rails() {
        let (low, high) = rail.sides();
        positions.extend([rail.start(), rail.end(), low, high]);
    }
    positions.iter().fold(0.0_f64, |scale, position| {
        scale
            .max(position.lower_metres().abs())
            .max(position.upper_metres().abs())
    })
}

/// A rail's height above the pitch line against the rule's range.
fn height(
    rail: &ObjectId,
    measurement: &RailMeasurement,
    (minimum, maximum): Range,
    slack: f64,
    along: &Along<'_>,
) -> Vec<(Check, Vec<ObjectId>)> {
    let mut checks = Vec::new();
    let bound = bound_words(minimum, maximum, metres);
    for (value, words, limit) in [
        (
            measurement.lowest(),
            "at its lowest",
            (minimum.map(|m| m - slack), None),
        ),
        (
            measurement.highest(),
            "at its highest",
            (None, maximum.map(|m| m + slack)),
        ),
    ] {
        if limit == (None, None) {
            continue;
        }
        let measured = format!(
            "handrail {rail} runs {} above the pitch line of {} {words}",
            shown(value.lower(), value.upper()),
            along.label
        );
        let check = match judge(value.lower(), value.upper(), limit.0, limit.1) {
            Verdict::Pass => Check::Pass,
            Verdict::Fail(_) => Check::failed(
                format!("{measured}; {bound} required"),
                deviation(
                    value.lower(),
                    value.upper(),
                    limit.0.and(minimum),
                    limit.1.and(maximum),
                ),
            ),
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {bound}"))
            }
        };
        checks.push((check, vec![rail.clone()]));
    }
    checks
}

/// Which end of the pitch line a rail extends beyond.
#[derive(Clone, Copy)]
enum End {
    Bottom,
    Top,
}

/// One stretch's handrails and how they are judged.
struct Judged<'m> {
    measured: &'m HandrailEvidence,
    slack: f64,
    along: &'m Along<'m>,
    /// Whether the selection left an object undecided, which may be a
    /// rail.
    undecided: bool,
    /// With `handrail_extension_from` `riser`, where the flight's risers
    /// lie from the pitch line's ends.
    risers: Option<&'m RiserOffsets>,
}

impl Judged<'_> {
    /// A pass not evaluated when an undecided object may be a rail.
    fn pending(&self, check: Check) -> Check {
        match check {
            Check::Pass if self.undecided => Check::Undecided(format!(
                "the handrails along {} pass, but a rail the selection could not decide may run \
                 along it",
                self.along.label
            )),
            other => other,
        }
    }

    /// A failure not evaluated when an undecided object may be another
    /// piece of the handrail: `why` says what it may do.
    fn continued(&self, check: Check, why: &str) -> Check {
        match check {
            Check::Fail(message) | Check::Graded(message, _) if self.undecided => Check::Undecided(
                format!("{message}; a rail the selection could not decide may {why}"),
            ),
            other => other,
        }
    }

    /// The extension of the handrail along each side, from its first piece
    /// at the bottom and its last at the top, and of each rail reaching
    /// over the middle on its own.
    fn extensions(&self, bounds: Range) -> Vec<(Check, Vec<ObjectId>)> {
        let mut checks = Vec::new();
        for side in [RailSide::Left, RailSide::Right] {
            match self.measured.side_rail(side) {
                Ok(pieces) => {
                    for (end, piece) in [(End::Bottom, pieces.first()), (End::Top, pieces.last())] {
                        let Some((rail, measurement)) = piece else {
                            continue;
                        };
                        let check = self.extension(rail, measurement, end, bounds);
                        checks.push((self.continued(check, "continue it"), vec![rail.clone()]));
                    }
                }
                Err(pieces) => checks.push((
                    Check::Undecided(format!(
                        "{}, so its extension is not measured",
                        unordered(side, &pieces, self.along)
                    )),
                    vec![],
                )),
            }
        }
        for (rail, measurement) in self.measured.rails() {
            if self.measured.side(measurement).is_some() {
                continue;
            }
            for end in [End::Bottom, End::Top] {
                let check = match self.extension(rail, measurement, end, bounds) {
                    // Falling short, it may be one piece of a longer rail;
                    // reaching too far, it reaches too far whatever it is.
                    Check::Fail(message) | Check::Graded(message, _)
                        if !self.too_far(measurement, end, bounds) =>
                    {
                        Check::Undecided(format!(
                            "{message}; it reaches over the middle of {}, so it may be one piece of a \
                         longer rail",
                            self.along.label
                        ))
                    }
                    other => other,
                };
                checks.push((check, vec![rail.clone()]));
            }
        }
        checks
    }

    /// A rail's extension beyond one end, measured from the pitch line's
    /// end or, with `handrail_extension_from` `riser`, from the flight's
    /// riser there; `Err` names why it is not known.
    fn reach(
        &self,
        measurement: &RailMeasurement,
        end: End,
    ) -> Option<Result<MeasuredInterval, String>> {
        let reach = match end {
            End::Bottom => self.measured.bottom_extension(measurement),
            End::Top => self.measured.top_extension(measurement),
        }?;
        let Some(risers) = self.risers else {
            return Some(Ok(reach));
        };
        let shifted = match end {
            End::Bottom => risers.bottom.as_ref().map(|(low, high)| {
                (
                    (reach.lower() + low).next_down(),
                    (reach.upper() + high).next_up(),
                )
            }),
            End::Top => risers.top.as_ref().map(|(low, high)| {
                (
                    (reach.lower() - high).next_down(),
                    (reach.upper() - low).next_up(),
                )
            }),
        };
        Some(match shifted {
            Ok((lower, upper)) => MeasuredInterval::try_new(lower, upper)
                .map_err(|error| format!("the extension from the riser: {error}")),
            Err(why) => Err(why.clone()),
        })
    }

    /// Whether a rail surely reaches beyond one end farther than the
    /// maximum allows.
    fn too_far(&self, measurement: &RailMeasurement, end: End, (_, maximum): Range) -> bool {
        maximum.is_some_and(|maximum| {
            matches!(self.reach(measurement, end), Some(Ok(reach)) if reach.lower() > maximum + self.slack)
        })
    }

    /// A rail's extension beyond one end against the rule's bounds, and
    /// whether it runs level over the minimum.
    fn extension(
        &self,
        rail: &ObjectId,
        measurement: &RailMeasurement,
        end: End,
        (minimum, maximum): Range,
    ) -> Check {
        let (words, rise) = match end {
            End::Bottom => ("bottom", measurement.bottom_rise()),
            End::Top => ("top", measurement.top_rise()),
        };
        let from = if self.risers.is_some() {
            format!("beyond the {words} riser of {}", self.along.label)
        } else {
            format!("beyond the {words} of {}", self.along.label)
        };
        let place = format!("beyond the {words} of {}", self.along.label);
        let bound = bound_words(minimum, maximum, metres);
        let Some(reach) = self.reach(measurement, end) else {
            // Measured along another part of a turning flight: the handrail
            // does not run along the end part at all.
            let other = match end {
                End::Bottom => "a later",
                End::Top => "an earlier",
            };
            let words = format!(
                "handrail {rail} runs along {other} straight part of {} only, so it does not \
                 reach {from}",
                self.along.label,
            );
            return match minimum {
                Some(_) => Check::Fail(format!("{words}; {bound} required")),
                None => Check::Pass,
            };
        };
        let reach = match reach {
            Ok(reach) => reach,
            Err(why) => {
                return Check::Undecided(format!(
                    "how far handrail {rail} reaches {from} is not known: {why}"
                ));
            }
        };
        let measured = format!(
            "handrail {rail} reaches {} {from}",
            shown(reach.lower(), reach.upper())
        );
        match judge(
            reach.lower(),
            reach.upper(),
            minimum.map(|minimum| minimum - self.slack),
            maximum.map(|maximum| maximum + self.slack),
        ) {
            Verdict::Fail(_) => Check::failed(
                format!("{measured}; {bound} required"),
                deviation(reach.lower(), reach.upper(), minimum, maximum),
            ),
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {bound}"))
            }
            Verdict::Pass => match minimum {
                Some(minimum) => level(rail, rise, minimum, &place),
                None => Check::Pass,
            },
        }
    }

    /// The gaps between consecutive pieces of the handrail along each side
    /// against the rule's maximum.
    fn gaps(&self, maximum: f64) -> Vec<(Check, Vec<ObjectId>)> {
        let mut checks = Vec::new();
        let allowed = format!("at most {} allowed", metres(maximum));
        for side in [RailSide::Left, RailSide::Right] {
            let pieces = match self.measured.side_rail(side) {
                Ok(pieces) => pieces,
                Err(pieces) => {
                    checks.push((
                        Check::Undecided(format!(
                            "{}, so its continuity is not measured",
                            unordered(side, &pieces, self.along)
                        )),
                        vec![],
                    ));
                    continue;
                }
            };
            for pair in pieces.windows(2) {
                let ((lower, a), (upper, b)) = (pair[0], pair[1]);
                let named = format!(
                    "handrail pieces {lower} and {upper} along the {} side of {}",
                    side_words(side),
                    self.along.label
                );
                let related = vec![lower.clone(), upper.clone()];
                let Some(gap) = self.measured.gap(a, b) else {
                    checks.push((
                        Check::Undecided(format!("the gap between {named} is not measured")),
                        related,
                    ));
                    continue;
                };
                let measured = format!(
                    "{named} leave a gap of {} in plan",
                    shown(gap.lower(), gap.upper())
                );
                let check = match judge(
                    gap.lower(),
                    gap.upper(),
                    None,
                    Some(maximum + 2.0 * self.slack),
                ) {
                    Verdict::Pass => Check::Pass,
                    Verdict::Fail(_) => Check::Graded(
                        format!("{measured}; {allowed}"),
                        Deviation::above(maximum, gap.lower(), gap.upper()),
                    ),
                    Verdict::Undecided(_) => {
                        Check::Undecided(format!("{measured}, which straddles {allowed}"))
                    }
                };
                checks.push((self.continued(check, "bridge it"), related));
            }
        }
        checks
    }
}

/// Why the pieces along a side cannot be put in order.
fn unordered(side: RailSide, pieces: &[ObjectId], along: &Along<'_>) -> String {
    let names = pieces
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "the handrails along the {} side of {} ({names}) lie beside or within one another, not \
         one after another",
        side_words(side),
        along.label
    )
}

/// Whether a rail's top runs level over the extension it must reach.
fn level(rail: &ObjectId, rise: Option<MeasuredInterval>, minimum: f64, place: &str) -> Check {
    let Some(rise) = rise else {
        return Check::Undecided(format!(
            "whether handrail {rail} runs level over the {} {place} is not measured",
            metres(minimum)
        ));
    };
    let measured = format!(
        "the top of handrail {rail} rises or falls {} over the {} {place}",
        shown(rise.lower(), rise.upper()),
        metres(minimum)
    );
    match judge(rise.lower(), rise.upper(), None, Some(LEVEL_RISE)) {
        Verdict::Pass => Check::Pass,
        Verdict::Fail(_) => Check::Fail(format!("{measured}; it must continue level")),
        Verdict::Undecided(_) => {
            Check::Undecided(format!("{measured}, which may or may not be level"))
        }
    }
}

fn side_words(side: RailSide) -> &'static str {
    match side {
        RailSide::Left => "left",
        RailSide::Right => "right",
    }
}

/// The sides handrails run along against the sides required.
fn sides(
    measured: &HandrailEvidence,
    required: Required,
    slack: f64,
    along: &Along<'_>,
    undecided: bool,
) -> (Check, Vec<ObjectId>) {
    let found: BTreeSet<RailSide> = measured
        .rails()
        .iter()
        .filter_map(|(_, rail)| measured.side(rail))
        .collect();
    let related: Vec<ObjectId> = measured
        .rails()
        .iter()
        .filter(|(_, rail)| measured.side(rail).is_some())
        .map(|(rail, _)| rail.clone())
        .collect();
    if found.len() == 2 {
        return (Check::Pass, related);
    }
    let both = match required {
        Required::One => false,
        Required::Both => true,
        Required::BothWiderThan(threshold) => {
            let Some(width) = along.width else {
                return (
                    Check::Undecided(format!(
                        "the width of {} is not measured, so whether it needs handrails on both \
                         sides is not known",
                        along.label
                    )),
                    related,
                );
            };
            match judge(width.lower(), width.upper(), None, Some(threshold + slack)) {
                Verdict::Pass => false,
                Verdict::Fail(_) => true,
                Verdict::Undecided(_) => {
                    if found.is_empty() {
                        // At least one side is missing either way.
                        false
                    } else {
                        return (
                            Check::Undecided(format!(
                                "{} is {} wide, which straddles the {} above which handrails \
                                 are required on both sides",
                                along.label,
                                shown(width.lower(), width.upper()),
                                metres(threshold)
                            )),
                            related,
                        );
                    }
                }
            }
        }
    };
    let needed = if both { 2 } else { 1 };
    if found.len() >= needed {
        return (Check::Pass, related);
    }
    let words = if both { "both sides" } else { "one side" };
    let message = match found.iter().next() {
        None => format!(
            "no selected handrail runs along a side of {}; {words} required",
            along.label
        ),
        Some(side) => format!(
            "a handrail runs along the {} side of {} only (seen climbing); {words} required",
            side_words(*side),
            along.label
        ),
    };
    let check = if undecided {
        Check::Undecided(format!(
            "{message}; a rail the selection could not decide may run along it"
        ))
    } else {
        Check::Fail(message)
    };
    (check, related)
}
