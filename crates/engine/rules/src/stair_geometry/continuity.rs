//! Handrail continuity across a landing, which a stair's whole-stair mode
//! and a ramp's intermediate landings share: the last piece along a side
//! of the stretch below the landing and the first piece along the same
//! side of the stretch above it (seen climbing both) must be joined by a
//! chain of selected rails, each within the allowed gap of the next as the
//! proximity service measures them in space, never by plan position alone.
//! A distance straddling the gap leaves the side undecided.

use std::collections::BTreeMap;

use axioval_engine::{
    HandrailEvidence, ProximityRequest, ProximityServiceHandle, RailSide, SlopedRun,
    WalkingStretch, WalkingSurfaceServiceHandle,
};
use axioval_ir::{Evidence, ObjectId};

use super::handrails::{self, HandrailCheck};
use super::whole::MEETING;
use super::{Check, Checks, Selected, slack};
use crate::level_spacing::{metres, shown};

/// Whether something holds: surely, possibly, or surely not.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Tri {
    No,
    Maybe,
    Sure,
}

/// Where the handrail along one side stands across a landing.
pub(super) enum Side {
    /// Neither stretch has a rail along it.
    Nothing,
    /// Its pieces are joined across the landing.
    Joined,
    /// It surely or possibly stops there: why, with the evidence and the
    /// rails involved.
    Broken(Tri, String, Vec<Evidence>, Vec<ObjectId>),
}

/// The rails that may join a handrail across landings and how far apart
/// consecutive ones may lie.
pub(super) struct Continuity<'a> {
    pub(super) proximity: Option<&'a ProximityServiceHandle>,
    /// The selected rails.
    pub(super) rails: &'a [ObjectId],
    /// Whether the selection left an object undecided, which may be a rail.
    pub(super) undecided: bool,
    /// The largest gap allowed between consecutive rails.
    pub(super) gap: f64,
    /// What a stretch is in a message: `flight`, `run`.
    pub(super) noun: &'a str,
}

impl Continuity<'_> {
    /// Whether the handrail along `side` continues across the landing
    /// `named` between the stretches measured `below` and `above` it.
    pub(super) fn side(
        &self,
        (below, above): (&HandrailEvidence, &HandrailEvidence),
        side: RailSide,
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
        let sure = if self.undecided {
            Tri::Maybe
        } else {
            Tri::Sure
        };
        match (last, first) {
            (None, None) => Side::Nothing,
            (Some(rail), None) | (None, Some(rail)) => Side::Broken(
                sure,
                format!(
                    "runs along one {} only and stops at {named} ({rail})",
                    self.noun
                ),
                vec![],
                vec![rail],
            ),
            (Some(from), Some(to)) => {
                let (joined, cited) = self.joined(&from, &to);
                match (joined, self.undecided) {
                    (Tri::Sure, _) => Side::Joined,
                    (Tri::No, false) => Side::Broken(
                        Tri::Sure,
                        format!(
                            "stops at {named}: {from} and {to} are not joined by selected rails \
                             within {} of each other",
                            metres(self.gap)
                        ),
                        cited,
                        vec![from, to],
                    ),
                    _ => Side::Broken(
                        Tri::Maybe,
                        format!(
                            "may stop at {named}: whether {from} and {to} are joined by rails \
                             within {} of each other is not decided",
                            metres(self.gap)
                        ),
                        cited,
                        vec![from, to],
                    ),
                }
            }
        }
    }

    /// Whether a chain of selected rails, each within the gap of the next,
    /// joins `from` to `to`: surely through measured pairs, surely not when
    /// no pair that may lie within it leads there.
    pub(super) fn joined(&self, from: &ObjectId, to: &ObjectId) -> (Tri, Vec<Evidence>) {
        if from == to {
            return (Tri::Sure, vec![]);
        }
        let mut nodes: Vec<ObjectId> = self.rails.to_vec();
        for end in [from, to] {
            if !nodes.contains(end) {
                nodes.push(end.clone());
            }
        }
        let start = nodes.iter().position(|node| node == from).unwrap_or(0);
        let goal = nodes.iter().position(|node| node == to).unwrap_or(0);
        let mut edges: BTreeMap<(usize, usize), Tri> = BTreeMap::new();
        let mut cited = Vec::new();
        let mut reach = |least: Tri, cited: &mut Vec<Evidence>| -> bool {
            let mut seen = vec![false; nodes.len()];
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(at) = stack.pop() {
                if at == goal {
                    return true;
                }
                for (next, visited) in seen.iter_mut().enumerate() {
                    if *visited {
                        continue;
                    }
                    let key = (at.min(next), at.max(next));
                    let found = *edges.entry(key).or_insert_with(|| {
                        touching(
                            self.proximity,
                            &nodes[key.0],
                            &nodes[key.1],
                            self.gap,
                            cited,
                        )
                    });
                    if found >= least {
                        *visited = true;
                        stack.push(next);
                    }
                }
            }
            false
        };
        if reach(Tri::Sure, &mut cited) {
            return (Tri::Sure, cited);
        }
        let possible = reach(Tri::Maybe, &mut cited);
        (if possible { Tri::Maybe } else { Tri::No }, cited)
    }
}

/// Whether two rails lie within `gap` of each other in space.
pub(super) fn touching(
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

/// The handrail along each side of a ramp across every landing between
/// consecutive runs, each within `gap` of the next piece: a side surely
/// broken is a finding, one whose gap straddles `gap` not evaluated.
pub(super) fn across_runs(
    (stairs, proximity): (
        &WalkingSurfaceServiceHandle,
        Option<&ProximityServiceHandle>,
    ),
    check: &HandrailCheck<'_>,
    rails: &Selected,
    object: &ObjectId,
    (runs, gap): (&[SlopedRun], f64),
) -> Checks {
    let (candidates, undecided) = match rails {
        Ok(rails) => rails,
        Err((_, message)) => return vec![(Check::Undecided(message.clone()), vec![], vec![])],
    };
    let total = runs.len();
    let label = |index: usize| format!("run {} of {total}", index + 1);
    let measure = |index: usize| {
        let label = label(index);
        handrails::measure(
            stairs,
            check,
            candidates,
            &handrails::Along {
                object,
                stretch: WalkingStretch::Run(index),
                label: &label,
                #[cfg(feature = "parity-reference")]
                width: runs[index].width(),
                #[cfg(feature = "parity-reference")]
                risers: None,
            },
        )
    };
    let continuity = Continuity {
        proximity,
        rails: candidates,
        undecided: *undecided,
        gap,
        noun: "run",
    };
    let mut checks = Vec::new();
    for index in 1..total {
        let named = format!(
            "the landing between {} and {}",
            label(index - 1),
            label(index)
        );
        let (arrives, starts) = (runs[index - 1].top(), runs[index].bottom());
        if arrives.lower_metres() - MEETING > starts.upper_metres()
            || starts.lower_metres() - MEETING > arrives.upper_metres()
        {
            checks.push((
                Check::Undecided(format!(
                    "{} arrives at {} and {} starts from {}, so the handrail across {named} is \
                     not judged",
                    label(index - 1),
                    shown(arrives.lower_metres(), arrives.upper_metres()),
                    label(index),
                    shown(starts.lower_metres(), starts.upper_metres())
                )),
                vec![],
                vec![],
            ));
            continue;
        }
        let (below, above) = match (measure(index - 1), measure(index)) {
            (Ok(below), Ok(above)) => (below, above),
            (Err(message), _) | (_, Err(message)) => {
                checks.push((Check::Undecided(message), vec![], vec![]));
                continue;
            }
        };
        let evidence = [below.evidence().clone(), above.evidence().clone()];
        for side in [RailSide::Left, RailSide::Right] {
            let words = match side {
                RailSide::Left => "left",
                RailSide::Right => "right",
            };
            let (verdict, message, mut cited, rails) =
                match continuity.side((&below, &above), side, &named) {
                    Side::Nothing | Side::Joined => continue,
                    Side::Broken(verdict, message, cited, rails) => {
                        (verdict, message, cited, rails)
                    }
                };
            cited.extend(evidence.iter().cloned());
            let message = format!("the handrail along the {words} side {message}");
            checks.push(match verdict {
                Tri::Sure => (Check::Fail(message), cited, rails),
                _ => (Check::Undecided(message), cited, rails),
            });
        }
    }
    checks
}
