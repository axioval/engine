//! Door swings for capabilities: leaves read through the object-frame
//! service and swing footprints bracketed by convex plan regions.
//!
//! A footprint is the union of the sectors a door's hinged leaves sweep.
//! Each sector is bracketed by an inscribed and a circumscribed convex
//! polygon (`SwingSector::plan_bounds`, `SEGMENTS` per quarter turn), so a
//! distance measured to the circumscribed one bounds the true distance from
//! below and one to the inscribed one from above.

use axioval_engine::{
    ConvexPlanRegion, DoorLeaves, DoorLeavesError, LeafMotion, NotEvaluatedReason,
    ObjectFrameServiceHandle,
};
use axioval_ir::{Evidence, ObjectId};

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
