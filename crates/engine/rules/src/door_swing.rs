//! Door swings for capabilities: leaves read through the object-frame
//! service and swing footprints bracketed by convex plan regions.
//!
//! A footprint is the union of the sectors a door's hinged leaves sweep.
//! Each sector is bracketed by an inscribed and a circumscribed convex
//! polygon (`SwingSector::plan_bounds`, `SEGMENTS` per quarter turn), so a
//! distance measured to the circumscribed one bounds the true distance from
//! below and one to the inscribed one from above.

use axioval_engine::{
    BoxClearance, ClearanceShape, ContainmentOutcome, ContainmentRequest, ConvexPlanRegion,
    DoorLeaves, DoorLeavesError, FreeSpaceServiceHandle, LeafMotion, MetricDirection, MetricFrame,
    MetricPoint, NotEvaluatedReason, ObjectFrameServiceHandle, SwingSector,
};
use axioval_ir::{Evidence, ObjectId};

use crate::component_clearance::free_space_error;
use crate::support::Unavailable;

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

/// The plan footprint a door's hinged leaves sweep.
pub(crate) struct Footprint {
    /// Per hinged leaf, an inscribed and a circumscribed region.
    pub(crate) parts: Vec<(ConvexPlanRegion, ConvexPlanRegion)>,
    pub(crate) evidence: Evidence,
}

impl Footprint {
    /// The swing footprint of `leaves`; empty when no leaf is hinged.
    pub(crate) fn of(leaves: &DoorLeaves) -> Result<Self, Unavailable> {
        let door = leaves.door();
        let mut parts = Vec::new();
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
