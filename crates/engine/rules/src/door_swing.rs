//! Door and window swings for capabilities: leaves read through the
//! object-frame service and swing footprints bracketed by convex plan
//! regions.
//!
//! A footprint is the union of what a door's or window's leaves sweep in
//! plan. Each side-hinged sector is bracketed by an inscribed and a
//! circumscribed convex polygon (`SwingSector::plan_bounds`, `SEGMENTS` per
//! quarter turn), so a distance measured to the circumscribed one bounds
//! the true distance from below and one to the inscribed one from above. A
//! window panel tilting on a top or bottom hinge sweeps, in plan, exactly
//! the rectangle its width spans out to its height along its opening
//! direction.

use axioval_engine::{
    BoxClearance, ClearanceShape, ContainmentOutcome, ContainmentRequest, ConvexPlanRegion,
    DoorLeaves, DoorLeavesError, FreeSpaceServiceHandle, LeafMotion, MetricDirection, MetricFrame,
    MetricPoint, NotEvaluatedReason, ObjectFrameServiceHandle, RuleContext, SweptDoor, SwingSector,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, ObjectId};

use crate::component_clearance::free_space_error;
use crate::selection::select_objects;
use crate::support::Unavailable;

/// The doors whose swings a rule subtracts (its `subtract_door_swings`
/// selection), each with the sectors its leaves sweep.
#[derive(Default)]
pub(crate) struct Swings {
    /// Surely selected doors that sweep floor.
    pub(crate) sure: Vec<SweptDoor>,
    /// Doors the selection cannot decide that sweep floor.
    pub(crate) maybe: Vec<SweptDoor>,
    /// Doors that may be selected but whose leaves are unknown, with why.
    pub(crate) unknown: Vec<(ObjectId, String)>,
    /// The leaves' provenance.
    pub(crate) evidence: Vec<Evidence>,
}

impl Swings {
    /// Reads the swings of every door `selector` picks or cannot decide. A
    /// door without a hinged leaf sweeps nothing; a door whose leaves
    /// cannot be read is unknown, never left out.
    ///
    /// # Errors
    ///
    /// A selection undecided for no one object.
    pub(crate) fn select(
        context: &RuleContext<'_>,
        selector: Option<&Selector>,
    ) -> Result<Self, Unavailable> {
        let Some(selector) = selector else {
            return Ok(Self::default());
        };
        let (picked, outcomes) = select_objects(context, selector);
        let mut doors: Vec<(ObjectId, bool)> = picked
            .iter()
            .map(|object| (object.id.clone(), true))
            .collect();
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => doors.push((object.clone(), false)),
                None => {
                    return Err((
                        outcome.reason().clone(),
                        format!(
                            "the door swing selection is undecided: {}",
                            outcome.message()
                        ),
                    ));
                }
            }
        }
        Ok(Self::of_doors(context, doors))
    }

    /// The swings of the doors a measured value's argument bound: those
    /// surely picked, then those it cannot decide, each in the project's
    /// order, as [`Swings::select`] reads a selection.
    pub(crate) fn of_selection(
        context: &RuleContext<'_>,
        selection: &axioval_ir::measured::MeasuredSelection,
    ) -> Self {
        let in_order = |set: &std::collections::BTreeSet<ObjectId>, sure: bool| {
            context
                .project
                .objects()
                .filter(|object| set.contains(&object.id))
                .map(|object| (object.id.clone(), sure))
                .collect::<Vec<_>>()
        };
        let mut doors = in_order(&selection.matched, true);
        doors.extend(in_order(&selection.undecided, false));
        Self::of_doors(context, doors)
    }

    /// The swings of `doors`, each surely selected or not.
    fn of_doors(context: &RuleContext<'_>, doors: Vec<(ObjectId, bool)>) -> Self {
        let mut swings = Self::default();
        let Some(frames) = context.services.get::<ObjectFrameServiceHandle>() else {
            for (door, _) in doors {
                swings
                    .unknown
                    .push((door, "the object-frame service is not registered".into()));
            }
            return swings;
        };
        for (door, sure) in doors {
            let swept = leaves(frames, &door).and_then(|leaves| {
                swings.evidence.push(leaves.evidence().clone());
                SweptDoor::of(&leaves).map_err(|error| (reason(&error), error.to_string()))
            });
            match swept {
                Ok(None) => {}
                Ok(Some(swept)) if sure => swings.sure.push(swept),
                Ok(Some(swept)) => swings.maybe.push(swept),
                Err((_, why)) => swings.unknown.push((door, why)),
            }
        }
        swings
    }

    /// Why the swings are not all decided, if they are not.
    pub(crate) fn undecided(&self) -> Option<String> {
        let mut why: Vec<String> = self
            .maybe
            .iter()
            .map(|door| {
                format!(
                    "whether the swing of {} is subtracted is undecided",
                    door.door()
                )
            })
            .collect();
        why.extend(
            self.unknown
                .iter()
                .map(|(door, why)| format!("{door}: {why}")),
        );
        (!why.is_empty()).then(|| why.join("; "))
    }
}

/// Chords per quarter turn: the radial gap is under 0.12 mm per metre.
pub(crate) const SEGMENTS: usize = 64;

/// Why a door's leaves are unknown, as a not-evaluated reason.
pub(crate) fn reason(error: &DoorLeavesError) -> NotEvaluatedReason {
    match error {
        DoorLeavesError::UncoveredSource(_) | DoorLeavesError::Unsupported => {
            NotEvaluatedReason::MissingService
        }
        DoorLeavesError::UnknownObject(_)
        | DoorLeavesError::NotADoor(_)
        | DoorLeavesError::NotStated(_)
        | DoorLeavesError::Refused(_)
        | DoorLeavesError::Unreadable(_) => NotEvaluatedReason::IncompleteEvidence,
        DoorLeavesError::InvalidLeaves(_)
        | DoorLeavesError::InexactEvidence
        | DoorLeavesError::ResponseRequestMismatch => NotEvaluatedReason::InvalidEvidence,
    }
}

/// The leaves of `door`, or why they are unknown.
pub(crate) fn leaves(
    frames: &ObjectFrameServiceHandle,
    door: &ObjectId,
) -> Result<DoorLeaves, Unavailable> {
    frames.leaves(door).map_err(|error| {
        (
            reason(&error),
            format!("the leaves of {door} are unknown: {error}"),
        )
    })
}

/// A horizontal unit direction is one whose z component is within this of
/// zero.
const LEVEL: f64 = 1.0e-9;

/// The one direction a door's hinged leaves all open towards: the side
/// they swing into. Refused for a door without a hinged leaf, with a
/// double-acting leaf (it swings to both sides), with leaves opening
/// different ways, or opening along a direction that is not horizontal.
pub(crate) fn swing_side(leaves: &DoorLeaves) -> Result<[f64; 3], Unavailable> {
    let door = leaves.door();
    let incomplete = |message: String| (NotEvaluatedReason::IncompleteEvidence, message);
    let mut side: Option<[f64; 3]> = None;
    for leaf in leaves.hinged() {
        if leaf.motion() == LeafMotion::DoubleSwing {
            return Err(incomplete(format!(
                "{door} has a double-acting leaf, which swings to both sides"
            )));
        }
        let opening = leaf.opening().components();
        if opening[2].abs() > LEVEL {
            return Err(incomplete(format!("{door} does not open horizontally")));
        }
        match side {
            None => side = Some(opening),
            Some(known) => {
                let agree = known[0] * opening[0] + known[1] * opening[1] + known[2] * opening[2];
                if agree < 1.0 - LEVEL {
                    return Err(incomplete(format!(
                        "the leaves of {door} open towards different sides"
                    )));
                }
            }
        }
    }
    side.ok_or_else(|| {
        incomplete(format!(
            "{door} has no hinged leaf ({}), so it swings to no side",
            leaves.operation()
        ))
    })
}

/// For a door with exactly one hinged leaf, the horizontal direction from
/// its hinge towards its handle: along the closed leaf.
pub(crate) fn handle(leaves: &DoorLeaves) -> Result<[f64; 3], Unavailable> {
    let door = leaves.door();
    let mut hinged = leaves.hinged();
    match (hinged.next(), hinged.next()) {
        (Some(leaf), None) => {
            let closed = leaf.swing().map_or_else(
                || unreachable!("a hinged leaf has a sector"),
                |sector| sector.closed().components(),
            );
            if closed[2].abs() > LEVEL {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("the leaf of {door} is not horizontal"),
                ));
            }
            Ok(closed)
        }
        (None, _) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{door} has no hinged leaf, so it has no handle side"),
        )),
        (Some(_), Some(_)) => Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("{door} has several hinged leaves, so its handle side is not one edge"),
        )),
    }
}

/// The plan footprint a door's or window's leaves sweep.
pub(crate) struct Footprint {
    /// Per swing or tilt, an inscribed and a circumscribed region.
    pub(crate) parts: Vec<(ConvexPlanRegion, ConvexPlanRegion)>,
    pub(crate) evidence: Evidence,
}

impl Footprint {
    /// The swing footprint of `leaves`: every side-hinged leaf's swing and
    /// every window panel's tilt; empty when no leaf swings or tilts.
    pub(crate) fn of(leaves: &DoorLeaves) -> Result<Self, Unavailable> {
        let door = leaves.door();
        let mut parts = Vec::new();
        for leaf in leaves.leaves() {
            let Some(tilt) = leaf.tilt() else { continue };
            let (along, open) = (leaf.along().components(), tilt.open().components());
            if along[2].abs() > LEVEL || open[2].abs() > LEVEL {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("a leaf of {door} does not tilt over a plan area"),
                ));
            }
            // The tilt sweeps from its hinge out to its radius along the
            // opening direction, along the whole width of the leaf.
            let [hx, hy, _] = tilt.hinge();
            let (width, reach) = (leaf.width_metres(), tilt.radius_metres());
            let mut ring = vec![
                [hx, hy],
                [hx + width * along[0], hy + width * along[1]],
                [
                    hx + width * along[0] + reach * open[0],
                    hy + width * along[1] + reach * open[1],
                ],
                [hx + reach * open[0], hy + reach * open[1]],
            ];
            if signed_area(&ring) < 0.0 {
                ring.reverse();
            }
            let region = ConvexPlanRegion::try_new(ring).map_err(|error| {
                (
                    NotEvaluatedReason::InvalidEvidence,
                    format!("the tilt of {door} is no convex region: {error}"),
                )
            })?;
            parts.push((region.clone(), region));
        }
        for leaf in leaves.hinged() {
            let Some((inner, outer)) = leaf.swing().and_then(|sector| sector.plan_bounds(SEGMENTS))
            else {
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("a leaf of {door} does not swing in a horizontal plane"),
                ));
            };
            let region = |ring| {
                ConvexPlanRegion::try_new(ring).map_err(|error| {
                    (
                        NotEvaluatedReason::InvalidEvidence,
                        format!("the swing of {door} is no convex region: {error}"),
                    )
                })
            };
            parts.push((region(inner)?, region(outer)?));
        }
        Ok(Self {
            parts,
            evidence: leaves.evidence().clone(),
        })
    }

    /// The plan box around every circumscribed region; `None` when empty.
    pub(crate) fn plan_box(&self) -> Option<([f64; 2], [f64; 2])> {
        self.parts
            .iter()
            .map(|(_, outer)| outer.bounds())
            .reduce(|(low, high), (a, b)| {
                (
                    [low[0].min(a[0]), low[1].min(a[1])],
                    [high[0].max(b[0]), high[1].max(b[1])],
                )
            })
    }

    /// Bounds on the plan distance to `other`'s footprint: infinite when
    /// either sweeps nothing.
    pub(crate) fn distance_to(&self, other: &Self) -> (f64, f64) {
        let mut bounds = (f64::INFINITY, f64::INFINITY);
        for (inner, outer) in &self.parts {
            for (other_inner, other_outer) in &other.parts {
                bounds.0 = bounds.0.min(outer.separation(other_outer).max(0.0));
                bounds.1 = bounds.1.min(inner.separation(other_inner).max(0.0));
            }
        }
        bounds
    }
}

/// Twice the signed area of a plan ring: positive when anticlockwise.
fn signed_area(ring: &[[f64; 2]]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum()
}

/// The plan gap between two boxes: zero when they meet.
pub(crate) fn box_gap(a: ([f64; 2], [f64; 2]), b: ([f64; 2], [f64; 2])) -> f64 {
    let dx = (b.0[0] - a.1[0]).max(a.0[0] - b.1[0]).max(0.0);
    let dy = (b.0[1] - a.1[1]).max(a.0[1] - b.1[1]).max(0.0);
    dx.hypot(dy)
}

/// How a door's swing relates to one space, judged at probe points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relation {
    /// A single-swing leaf opens into the space.
    Into,
    /// The door opens out of the space: the space lies behind it and not
    /// on its swing side.
    Away,
    /// A double-acting leaf opens into the space and out of it.
    BothWays,
    /// Neither side of the door lies in the space at its probes.
    Apart,
}

impl Relation {
    /// Whether a leaf swings into the space.
    pub(crate) fn swings_into(self) -> bool {
        matches!(self, Self::Into | Self::BothWays)
    }
}

/// How far out a probe stands, as a share of the leaf's width: well beyond
/// the leaf's own frame and any ordinary wall.
const PROBE_REACH: f64 = 0.75;

/// A probe's side in metres: small enough to fall on one side of a wall.
const PROBE_SIZE: f64 = 0.05;

/// Whether the probe `sign` (+1 on the swing side, -1 behind it) of `leaf`
/// lies in `space`: halfway through the sweep, three quarters of the leaf
/// out from the hinge.
fn probe(
    free: &FreeSpaceServiceHandle,
    door: &ObjectId,
    sector: &SwingSector,
    sign: f64,
    space: &ObjectId,
) -> Result<(bool, Evidence), Unavailable> {
    let (closed, open) = (sector.closed().components(), sector.open().components());
    let reach = PROBE_REACH * sector.radius_metres() * std::f64::consts::FRAC_1_SQRT_2;
    let hinge = sector.hinge();
    let at = [0, 1, 2].map(|axis| hinge[axis] + reach * (closed[axis] + sign * open[axis]));
    let invalid = |error: axioval_engine::FreeSpaceError| {
        (
            NotEvaluatedReason::InvalidEvidence,
            format!("a probe beside {door} cannot be placed: {error}"),
        )
    };
    let forward = MetricDirection::try_new([open[0], open[1], 0.0]).map_err(invalid)?;
    let [fx, fy, _] = forward.components();
    let frame = MetricFrame::try_new(
        MetricPoint::try_new(door.clone(), at).map_err(|error| {
            (
                NotEvaluatedReason::InvalidEvidence,
                format!("a probe beside {door} cannot be placed: {error}"),
            )
        })?,
        MetricDirection::try_new([fy, -fx, 0.0]).map_err(invalid)?,
        forward,
        MetricDirection::try_new([0.0, 0.0, 1.0]).map_err(invalid)?,
    )
    .map_err(invalid)?;
    let shape = BoxClearance::try_new(PROBE_SIZE, PROBE_SIZE, PROBE_SIZE).map_err(invalid)?;
    let request = ContainmentRequest::new(frame, ClearanceShape::Box(shape), vec![space.clone()]);
    match free.assess_containment(&request) {
        Ok(ContainmentOutcome::Inside(proof)) => Ok((true, proof.evidence().clone())),
        Ok(ContainmentOutcome::Outside(proof)) => Ok((false, proof.evidence().clone())),
        Err(error) => {
            let (reason, message) = free_space_error(&error);
            Err((
                reason,
                format!("whether {space} lies beside {door} is unknown: {message}"),
            ))
        }
    }
}

/// How `leaves` swing relative to `space`, with the containment evidence.
///
/// Each hinged leaf is probed on both sides, halfway through its sweep and
/// three quarters of its width out: a side lies in the space when a probe
/// of any leaf lies in it, and outside when every leaf's lies outside. A
/// door without a hinged leaf is refused: it swings to no side.
pub(crate) fn relation(
    free: &FreeSpaceServiceHandle,
    leaves: &DoorLeaves,
    space: &ObjectId,
) -> Result<(Relation, Vec<Evidence>), Unavailable> {
    let door = leaves.door();
    let mut evidence = vec![leaves.evidence().clone()];
    let mut sides = [Ok(false), Ok(false)];
    let mut double = false;
    let mut any = false;
    for leaf in leaves.hinged() {
        let Some(sector) = leaf.swing() else { continue };
        any = true;
        double |= sector.is_double_acting();
        for (side, sign) in sides.iter_mut().zip([1.0, -1.0]) {
            if matches!(side, Ok(true)) {
                continue;
            }
            match probe(free, door, sector, sign, space) {
                Ok((inside, proof)) => {
                    evidence.push(proof);
                    if inside {
                        *side = Ok(true);
                    }
                }
                Err(unavailable) => {
                    if side.is_ok() {
                        *side = Err(unavailable);
                    }
                }
            }
        }
    }
    if !any {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "{door} has no hinged leaf ({}), so it swings into no space",
                leaves.operation()
            ),
        ));
    }
    let [swing, back] = sides;
    let relation = match (double, swing, back) {
        (true, Ok(true), _) | (true, _, Ok(true)) => Relation::BothWays,
        (false, Ok(true), _) => Relation::Into,
        (_, Ok(false), Ok(false)) => Relation::Apart,
        (false, Ok(false), Ok(true)) => Relation::Away,
        (_, Err(unavailable), _) | (_, _, Err(unavailable)) => return Err(unavailable),
    };
    Ok((relation, evidence))
}
