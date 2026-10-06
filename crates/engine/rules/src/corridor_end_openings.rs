//! Corridor-end openings: no window in the wall a corridor ends at.
//!
//! Each selected space (a corridor, by type) reaches its openings through
//! `opening_path` (for example `axioval:derived.adjacent-space:backward`),
//! filtered by `opening_selector` (windows). The plan-span service finds the
//! ends of the paths through the space's footprint and the wall each runs
//! into, and measures every opening against each such wall: its plan gap to
//! the wall segment and the length of the segment it faces.
//!
//! An opening sits in an end wall when it lies within `wall_depth` of it
//! (half a metre by default: the depth of a thick wall behind its face) and
//! faces more than `facing` of it (a tenth of a metre by default, so a
//! window in a side wall beside the corner, which faces none of it, does
//! not). It surely sits there when the whole gap interval lies within the
//! depth and the whole facing interval beyond the minimum; surely not when
//! either lies wholly on the other side. Anything else is undecided, and so
//! is every opening of a space one of whose ends runs into a wall the
//! service could not name: the opening might sit in it.
//!
//! The ends themselves come from an approximate skeleton, so their evidence
//! is approximate; the gap and facing lengths are measured on the
//! footprints. An opening whose selection is undecided is found only as
//! not evaluated, and only where it would sit in an end wall.

use std::sync::LazyLock;

use axioval_engine::template::Template;
use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CorridorEnds, EndWall, NotEvaluatedReason,
    ParameterDescriptor, PlanSpanError, RuleCapability, RuleContext, WallContact,
};

use crate::plan_area::shown;
use crate::support::Unavailable;

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
mod template;

pub(crate) use measured::CorridorEndSearch;

/// Finds openings in the wall a selected corridor ends at.
///
/// It runs as a template ([`axioval_engine::template`]): the openings a
/// corridor reaches, each searched against the corridor's end walls (the
/// measured list `corridor_end_openings`), judged on the opening by
/// whether it sits in one and whether it is selected.
pub struct CorridorEndOpenings;

static TEMPLATE: LazyLock<Template> = LazyLock::new(template::template);

/// The plans of the rules bound to it, kept across runs.
static PLANS: crate::templates::Plans = crate::templates::Plans::new();

impl RuleCapability for CorridorEndOpenings {
    fn id(&self) -> &'static str {
        template::ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        TEMPLATE.parameters.clone()
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        crate::templates::run((&TEMPLATE, &PLANS), context, rule)
    }

    fn template(&self) -> Option<&Template> {
        Some(&TEMPLATE)
    }
}

/// How far an opening may lie from an end wall, and how much of it it
/// must face, to sit in it.
#[derive(Clone, Copy)]
pub(crate) struct Margins {
    pub(crate) wall_depth: f64,
    pub(crate) facing: f64,
}

/// Where an opening stands against one end wall.
enum Standing {
    In,
    Out,
    Unknown(String),
}

fn standing(margins: Margins, contact: &WallContact) -> Standing {
    let (gap, facing) = (contact.gap(), contact.facing());
    if gap.lower_metres() > margins.wall_depth || facing.upper_metres() <= margins.facing {
        return Standing::Out;
    }
    if gap.upper_metres() <= margins.wall_depth && facing.lower_metres() > margins.facing {
        return Standing::In;
    }
    Standing::Unknown(format!(
        "it lies {} m from an end wall and faces {} m of it, which straddles \
         within {} m and more than {} m",
        shown(gap.lower_metres(), gap.upper_metres()),
        shown(facing.lower_metres(), facing.upper_metres()),
        margins.wall_depth,
        margins.facing
    ))
}

/// How an opening stands against every end of a corridor.
pub(crate) enum Judged<'a> {
    /// In these end walls.
    In(Vec<([f64; 2], [f64; 2], &'a WallContact)>),
    /// In none.
    Out,
    /// Possibly in one; why.
    Unknown(Vec<String>),
}

/// How the opening `index` of the request stands against every end.
pub(crate) fn judge(margins: Margins, ends: &CorridorEnds, index: usize) -> Judged<'_> {
    let mut walls = Vec::new();
    let mut unknown = Vec::new();
    for corridor_end in ends.ends() {
        let [x, y] = corridor_end.point();
        match corridor_end.wall() {
            EndWall::Undecided(why) => unknown.push(format!(
                "the wall the end near ({}, {}) runs into is undecided: {why}",
                shown(x, x),
                shown(y, y)
            )),
            EndWall::Decided {
                start,
                end,
                contacts,
            } => match standing(margins, &contacts[index]) {
                Standing::In => walls.push((*start, *end, &contacts[index])),
                Standing::Out => {}
                Standing::Unknown(why) => unknown.push(format!("{why} ({})", wall(*start, *end))),
            },
        }
    }
    if !walls.is_empty() {
        Judged::In(walls)
    } else if unknown.is_empty() {
        Judged::Out
    } else {
        Judged::Unknown(unknown)
    }
}

/// The end walls an opening sits in, worded as a finding names them.
pub(crate) fn described(walls: &[([f64; 2], [f64; 2], &WallContact)]) -> String {
    walls
        .iter()
        .map(|(start, end, contact)| {
            format!(
                "{} m from the wall {} and facing {} m of it",
                shown(contact.gap().lower_metres(), contact.gap().upper_metres()),
                wall(*start, *end),
                shown(
                    contact.facing().lower_metres(),
                    contact.facing().upper_metres()
                ),
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn wall(start: [f64; 2], end: [f64; 2]) -> String {
    format!(
        "({}, {})–({}, {})",
        shown(start[0], start[0]),
        shown(start[1], start[1]),
        shown(end[0], end[0]),
        shown(end[1], end[1])
    )
}

pub(crate) fn unavailable(error: &PlanSpanError) -> Unavailable {
    let reason = match error {
        PlanSpanError::UnknownObject(_) | PlanSpanError::Unavailable(_) => {
            NotEvaluatedReason::IncompleteEvidence
        }
        PlanSpanError::InvalidMeasurement | PlanSpanError::InexactEvidence => {
            NotEvaluatedReason::InvalidEvidence
        }
    };
    (
        reason,
        format!("the corridor ends cannot be measured: {error}"),
    )
}
