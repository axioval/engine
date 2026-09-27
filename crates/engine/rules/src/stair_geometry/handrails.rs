//! The handrail checks `stair-geometry` and `ramp-geometry` share: the
//! height of each rail above the pitch line, its level extension beyond the
//! ends, and the sides a rail runs along.

use std::collections::BTreeSet;

use axioval_engine::{
    HandrailEvidence, HandrailRequest, MeasuredInterval, ParameterDescriptor, ParameterType,
    RailMeasurement, RailSide, WalkingStretch, WalkingSurfaceServiceHandle,
};
use axioval_ir::ObjectId;
use axioval_ir::contract::Selector;

use super::{Check, Checks, Range, bound_words, length, range, service_error, slack};
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, judge};
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
    extension: Option<f64>,
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
        ParameterDescriptor::optional("handrail_sides", ParameterType::String),
        ParameterDescriptor::optional("handrail_both_sides_above_width", ParameterType::Quantity),
    ]
}

pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<HandrailCheck<'a>>, Unavailable> {
    let rails = parameters.selector("handrail_objects")?;
    let reach = length(parameters, "handrail_reach_across")?;
    let above = length(parameters, "handrail_reach_above")?;
    let height = range(parameters, "handrail_height")?;
    let extension = length(parameters, "handrail_extension_minimum")?;
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
    let declared = height != (None, None) || extension.is_some() || sides.is_some();
    match (rails, reach, above, declared) {
        (Some(rails), Some(reach), Some(above), true) => Ok(Some(HandrailCheck {
            rails,
            reach,
            above,
            height,
            extension,
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
        check.extension.unwrap_or(0.0),
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
    let slack = slack(scale(&measured));
    let pending = |check: Check| match check {
        Check::Pass if *undecided => Check::Undecided(format!(
            "the handrails along {} pass, but a rail the selection could not decide may run \
             along it",
            along.label
        )),
        other => other,
    };
    let mut checks: Checks = Vec::new();
    let mut push = |check: Check, related: Vec<ObjectId>| {
        checks.push((check, evidence.clone(), related));
    };
    if check.height != (None, None) {
        let mut all = Vec::new();
        for (rail, measurement) in measured.rails() {
            all.extend(height(rail, measurement, check.height, slack, along));
        }
        push_all(&mut push, all, pending(Check::Pass));
    }
    if let Some(minimum) = check.extension {
        let mut all = Vec::new();
        for (rail, measurement) in measured.rails() {
            all.extend(extension(
                &measured,
                rail,
                measurement,
                minimum,
                slack,
                along,
            ));
        }
        push_all(&mut push, all, pending(Check::Pass));
    }
    if let Some(required) = check.sides {
        let (check, related) = sides(&measured, required, slack, along, *undecided);
        push(check, related);
    }
    checks
}

/// Pushes each failing or undecided check with the rail it names, or
/// `passing` when every one passes.
fn push_all(
    push: &mut impl FnMut(Check, Vec<ObjectId>),
    checks: Vec<(Check, ObjectId)>,
    passing: Check,
) {
    let mut any = false;
    for (check, rail) in checks {
        if !matches!(check, Check::Pass) {
            any = true;
            push(check, vec![rail]);
        }
    }
    if !any && !matches!(passing, Check::Pass) {
        push(passing, vec![]);
    }
}

/// The largest magnitude among the positions of the measurement.
fn scale(measured: &HandrailEvidence) -> f64 {
    let (start, end) = measured.pitch();
    let (left, right) = measured.sides();
    let mut positions = vec![start, end, left, right];
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
) -> Vec<(Check, ObjectId)> {
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
            Verdict::Fail(_) => Check::Fail(format!("{measured}; {bound} required")),
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {bound}"))
            }
        };
        checks.push((check, rail.clone()));
    }
    checks
}

/// A rail's extension beyond both ends against the rule's minimum, and
/// whether it runs level over it.
fn extension(
    measured: &HandrailEvidence,
    rail: &ObjectId,
    measurement: &RailMeasurement,
    minimum: f64,
    slack: f64,
    along: &Along<'_>,
) -> Vec<(Check, ObjectId)> {
    let ends = [
        (
            "bottom",
            measured.bottom_extension(measurement),
            measurement.bottom_rise(),
        ),
        (
            "top",
            measured.top_extension(measurement),
            measurement.top_rise(),
        ),
    ];
    let mut checks = Vec::new();
    for (end, reach, rise) in ends {
        let place = format!("beyond the {end} of {}", along.label);
        let measured = format!(
            "handrail {rail} reaches {} {place}",
            shown(reach.lower(), reach.upper())
        );
        let required = format!("at least {} required", metres(minimum));
        let check = match judge(reach.lower(), reach.upper(), Some(minimum - slack), None) {
            Verdict::Fail(_) => Check::Fail(format!("{measured}; {required}")),
            Verdict::Undecided(_) => {
                Check::Undecided(format!("{measured}, which straddles {required}"))
            }
            Verdict::Pass => level(rail, rise, minimum, &place),
        };
        checks.push((check, rail.clone()));
    }
    checks
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
