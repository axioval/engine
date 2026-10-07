//! `ramp-geometry`'s rail obstruction: with `check_rails_obstruction`, no
//! rail of the ramp may reach over an `accessible_surface_selector`
//! surface (the clear path of a landing or corridor beside its ends) in
//! plan. The ramp's rails are the `handrail_objects` rails measured along
//! its runs and the selected rails joined to them, touching in space, such
//! as a separate extension piece. Each pair is judged by the proximity
//! service's plan overlap (`ProximityProjection::PlanOverlap`), the
//! measurement `distance` takes with `projection: plan_overlap`: related
//! is a finding, unrelated a pass, and an overlap left open (a
//! tessellation within its chord deviation of the surface) is not
//! evaluated.

use std::collections::BTreeMap;

use axioval_engine::{
    ProximityProjection, ProximityRequest, ProximityServiceHandle, SlopedRun, WalkingStretch,
    WalkingSurfaceServiceHandle,
};
#[cfg(feature = "parity-reference")]
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use super::continuity::{Tri, touching};
use super::handrails::{self, HandrailCheck};
use super::{Check, Checks, Selected};
#[cfg(feature = "parity-reference")]
use crate::support::{Parameters, Unavailable, invalid};

/// The rail-obstruction check of one rule.
#[cfg(feature = "parity-reference")]
pub(super) struct ObstructionCheck<'a> {
    pub(super) surfaces: &'a Selector,
}

#[cfg(feature = "parity-reference")]
pub(super) fn parse<'a>(
    parameters: &Parameters<'a>,
    handrail: Option<&HandrailCheck<'_>>,
) -> Result<Option<ObstructionCheck<'a>>, Unavailable> {
    let check = parameters
        .boolean("check_rails_obstruction")?
        .unwrap_or(false);
    let surfaces = parameters.selector("accessible_surface_selector")?;
    match (check, surfaces) {
        (false, None) => Ok(None),
        (true, Some(surfaces)) if handrail.is_some() => Ok(Some(ObstructionCheck { surfaces })),
        (true, Some(_)) => Err(invalid(
            "`check_rails_obstruction` needs `handrail_objects`, `handrail_reach_across` and \
             `handrail_reach_above`",
        )),
        _ => Err(invalid(
            "`check_rails_obstruction` and `accessible_surface_selector` are declared together",
        )),
    }
}

/// The ramp's rails over the selected surfaces.
pub(super) fn obstruction(
    (stairs, proximity): (
        &WalkingSurfaceServiceHandle,
        Option<&ProximityServiceHandle>,
    ),
    check: &HandrailCheck<'_>,
    (rails, surfaces): (&Selected, &Selected),
    object: &ObjectId,
    runs: &[SlopedRun],
) -> Checks {
    let (candidates, undecided_rails) = match rails {
        Ok(rails) => rails,
        Err((_, message)) => return vec![(Check::Undecided(message.clone()), vec![], vec![])],
    };
    let (surfaces, undecided_surfaces) = match surfaces {
        Ok(surfaces) => surfaces,
        Err((_, message)) => return vec![(Check::Undecided(message.clone()), vec![], vec![])],
    };
    let Some(proximity) = proximity else {
        return vec![(
            Check::Undecided(
                "the proximity service is not registered, so whether the ramp's rails reach \
                 over an accessible surface is not measured"
                    .into(),
            ),
            vec![],
            vec![],
        )];
    };
    let members = match members((stairs, proximity), check, candidates, object, runs) {
        Ok(members) => members,
        Err(message) => return vec![(Check::Undecided(message), vec![], vec![])],
    };
    let mut checks = Vec::new();
    let mut open = Vec::new();
    for (rail, member) in &members {
        for surface in surfaces {
            if surface == rail || surface == object {
                continue;
            }
            let (over, evidence) = overlaps(proximity, rail, surface);
            let named = format!("handrail {rail} of the ramp");
            match (over, *member) {
                (Tri::No, _) => {}
                (Tri::Sure, Tri::Sure) => checks.push((
                    Check::Fail(format!(
                        "{named} reaches over the accessible surface {surface} in plan"
                    )),
                    evidence.into_iter().collect(),
                    vec![rail.clone(), surface.clone()],
                )),
                (Tri::Sure, _) => open.push((
                    format!(
                        "rail {rail} reaches over the accessible surface {surface} in plan, \
                         but whether it is joined to the ramp's handrail is not decided"
                    ),
                    evidence,
                )),
                (Tri::Maybe, _) => open.push((
                    format!(
                        "whether {named} reaches over the accessible surface {surface} in plan \
                         is not decided"
                    ),
                    evidence,
                )),
            }
        }
    }
    if !checks.is_empty() {
        return checks;
    }
    if open.is_empty() {
        let pending = match (*undecided_rails, *undecided_surfaces) {
            (false, false) => return vec![],
            (true, _) => "a rail the selection could not decide may reach over one",
            (false, true) => "a surface the selection could not decide may lie under one",
        };
        return vec![(
            Check::Undecided(format!(
                "no rail of the ramp reaches over a selected accessible surface, but {pending}"
            )),
            vec![],
            vec![],
        )];
    }
    open.into_iter()
        .map(|(message, evidence)| {
            (
                Check::Undecided(message),
                evidence.into_iter().collect(),
                vec![],
            )
        })
        .collect()
}

/// Whether a rail's footprint overlaps a surface's with positive area.
fn overlaps(
    proximity: &ProximityServiceHandle,
    rail: &ObjectId,
    surface: &ObjectId,
) -> (Tri, Option<Evidence>) {
    let Ok(request) = ProximityRequest::projected(
        rail.clone(),
        surface.clone(),
        ProximityProjection::PlanOverlap,
    ) else {
        return (Tri::Maybe, None);
    };
    match proximity.measure_distance(&request) {
        Ok(measured) => {
            let (lower, upper) = measured.interval_metres();
            let evidence = Some(measured.evidence().clone());
            if upper <= 0.0 {
                (Tri::Sure, evidence)
            } else if lower > 0.0 {
                (Tri::No, evidence)
            } else {
                (Tri::Maybe, evidence)
            }
        }
        Err(_) => (Tri::Maybe, None),
    }
}

/// The ramp's rails: those measured along its runs, surely, and the
/// selected rails joined to them, surely through touching pairs, possibly
/// through pairs that may touch.
fn members(
    (stairs, proximity): (&WalkingSurfaceServiceHandle, &ProximityServiceHandle),
    check: &HandrailCheck<'_>,
    candidates: &[ObjectId],
    object: &ObjectId,
    runs: &[SlopedRun],
) -> Result<BTreeMap<ObjectId, Tri>, String> {
    // The rails along its runs, surely the ramp's.
    let mut members: BTreeMap<ObjectId, Tri> = BTreeMap::new();
    let total = runs.len();
    for (index, run) in runs.iter().enumerate() {
        let label = format!("run {} of {total}", index + 1);
        #[cfg(not(feature = "parity-reference"))]
        let _ = run;
        let along = handrails::Along {
            object,
            stretch: WalkingStretch::Run(index),
            label: &label,
            #[cfg(feature = "parity-reference")]
            width: run.width(),
            #[cfg(feature = "parity-reference")]
            risers: None,
        };
        match handrails::measure(stairs, check, candidates, &along) {
            Ok(measured) => {
                for (rail, _) in measured.rails() {
                    members.insert(rail.clone(), Tri::Sure);
                }
            }
            Err(message) => return Err(message),
        }
    }
    // The selected rails joined to them: surely through touching pairs,
    // possibly through pairs that may touch.
    let mut cited = Vec::new();
    let mut queue: Vec<ObjectId> = members.keys().cloned().collect();
    while let Some(rail) = queue.pop() {
        let level = members[&rail];
        for candidate in candidates {
            if *candidate == rail || members.get(candidate).is_some_and(|known| *known >= level) {
                continue;
            }
            let joined = touching(Some(proximity), &rail, candidate, 0.0, &mut cited).min(level);
            if joined > members.get(candidate).copied().unwrap_or(Tri::No) {
                members.insert(candidate.clone(), joined);
                queue.push(candidate.clone());
            }
        }
    }
    Ok(members)
}
