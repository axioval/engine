//! The clear width of a flight or a ramp's run: the narrowest free width
//! across it that the `clear_width_obstacles` (handrails, walls, anything
//! beside or over it) leave between two heights above its pitch line, as
//! the walking-surface service measures it.

use axioval_engine::{
    ClearWidthRequest, Deviation, ParameterDescriptor, ParameterType, WalkingStretch,
    WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use super::{Check, length, service_error, slack};
use crate::level_spacing::{metres, shown};
use crate::plan_area::{Verdict, judge};
use crate::support::{Parameters, Unavailable, invalid};

/// The clear-width check of one rule.
pub(super) struct ClearWidthCheck<'a> {
    pub(super) obstacles: &'a Selector,
    minimum: f64,
    band: (f64, f64),
}

pub(super) fn descriptors() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("clear_width_minimum", ParameterType::Quantity),
        ParameterDescriptor::optional("clear_width_obstacles", ParameterType::Selector),
        ParameterDescriptor::optional("clear_width_band_from", ParameterType::Quantity),
        ParameterDescriptor::optional("clear_width_band_to", ParameterType::Quantity),
    ]
}

pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
) -> Result<Option<ClearWidthCheck<'a>>, Unavailable> {
    let minimum = length(parameters, "clear_width_minimum")?;
    let obstacles = parameters.selector("clear_width_obstacles")?;
    let from = length(parameters, "clear_width_band_from")?;
    let to = length(parameters, "clear_width_band_to")?;
    match (minimum, obstacles, from, to) {
        (Some(minimum), Some(obstacles), Some(from), Some(to)) if from < to => {
            Ok(Some(ClearWidthCheck {
                obstacles,
                minimum,
                band: (from, to),
            }))
        }
        (None, None, None, None) => Ok(None),
        _ => Err(invalid(
            "`clear_width_minimum`, `clear_width_obstacles`, `clear_width_band_from` and \
             `clear_width_band_to` are declared together, the band's bottom below its top",
        )),
    }
}

/// The clear width along one stretch against the rule's minimum. An
/// obstacle the selection could not decide can only narrow it: too narrow
/// stands, wide enough is not evaluated.
pub(super) fn clear_width(
    stairs: &WalkingSurfaceServiceHandle,
    check: &ClearWidthCheck<'_>,
    obstacles: &Result<(Vec<ObjectId>, bool), Unavailable>,
    (object, stretch): (&ObjectId, WalkingStretch),
    label: &str,
) -> (Check, Vec<Evidence>, Vec<ObjectId>) {
    let (candidates, undecided) = match obstacles {
        Ok(obstacles) => obstacles,
        Err((_, message)) => return (Check::Undecided(message.clone()), vec![], vec![]),
    };
    let measured = ClearWidthRequest::try_new(
        object.clone(),
        stretch,
        candidates.iter().cloned(),
        check.band,
    )
    .and_then(|request| stairs.measure_clear_width(&request));
    let measured = match measured {
        Ok(measured) => measured,
        Err(error) => {
            return (
                Check::Undecided(format!(
                    "the clear width of {label}: {}",
                    service_error(&error).1
                )),
                vec![],
                vec![],
            );
        }
    };
    let width = measured.width();
    let governing = measured.governing().to_vec();
    let between = if governing.is_empty() {
        "between its own sides".to_owned()
    } else {
        format!(
            "beside {}",
            governing
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" and ")
        )
    };
    let words = format!(
        "the clear width of {label} {} to {} above its pitch line is {} {between}",
        metres(check.band.0),
        metres(check.band.1),
        shown(width.lower(), width.upper())
    );
    let required = format!("at least {} required", metres(check.minimum));
    let slack = slack(width.upper());
    let evidence = vec![measured.evidence().clone()];
    let check = match judge(
        width.lower(),
        width.upper(),
        Some(check.minimum - slack),
        None,
    ) {
        Verdict::Fail(_) => Check::failed(
            format!("{words}; {required}"),
            Some(Deviation::below(
                check.minimum,
                width.lower(),
                width.upper(),
            )),
        ),
        Verdict::Pass if !undecided => Check::Pass,
        Verdict::Pass => Check::Undecided(format!(
            "{words}; an obstacle the selection could not decide may narrow it"
        )),
        Verdict::Undecided(_) => Check::Undecided(format!("{words}, which straddles {required}")),
    };
    (check, evidence, governing)
}
