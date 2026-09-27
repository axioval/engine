//! Walls beside a component's sides (internal): the one reader of
//! `PlanSpanServiceHandle::measure_side_distances` for every capability.
//!
//! Sides are those of the footprint's least-area rectangle. A side's
//! nearest wall is an interval: bounded from below by every wall that may
//! lie beside it (possibly present, or a wall the selection cannot decide),
//! and from above only by walls surely selected and surely present. The
//! side a component stands against is the one whose nearest wall is surely
//! nearer than every other side's; a tie is never broken.

use std::collections::BTreeSet;

use axioval_engine::{
    PlanSpanError, PlanSpanServiceHandle, RectangleSide, RuleContext, SideDistanceRequest,
    SideDistances, SidePresence,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, NotEvaluatedReason, ObjectId};

use crate::level_spacing::metres;
use crate::selection::select_objects;
use crate::support::Unavailable;

/// The wall selection, split by how sure it is.
pub(crate) struct Walls {
    sure: BTreeSet<ObjectId>,
    maybe: BTreeSet<ObjectId>,
    failed: Option<Unavailable>,
}

impl Walls {
    pub(crate) fn select(context: &RuleContext<'_>, selector: &Selector) -> Self {
        let (objects, outcomes) = select_objects(context, selector);
        let mut maybe = BTreeSet::new();
        let mut failed = None;
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => {
                    maybe.insert(object.clone());
                }
                None => {
                    failed.get_or_insert_with(|| {
                        (
                            outcome.reason().clone(),
                            format!("wall selection is undecided: {}", outcome.message()),
                        )
                    });
                }
            }
        }
        Self {
            sure: objects.iter().map(|object| object.id.clone()).collect(),
            maybe,
            failed,
        }
    }

    fn is_sure(&self, wall: &ObjectId) -> bool {
        self.sure.contains(wall)
    }

    /// Every wall beside `object`'s sides within `reach` of its centre
    /// lines, each strip narrowed by `inset`.
    pub(crate) fn measure(
        &self,
        spans: &PlanSpanServiceHandle,
        object: &ObjectId,
        reach: f64,
        inset: f64,
    ) -> Result<SideDistances, Unavailable> {
        if let Some(failed) = &self.failed {
            return Err(failed.clone());
        }
        let candidates = self
            .sure
            .iter()
            .chain(&self.maybe)
            .filter(|wall| *wall != object)
            .cloned();
        let request = SideDistanceRequest::try_new(object.clone(), candidates, reach, inset)
            .map_err(|error| unavailable(object, &error))?;
        spans
            .measure_side_distances(&request)
            .map_err(|error| unavailable(object, &error))
    }

    /// The nearest wall beside `side`, from the side's centre line.
    pub(crate) fn nearest(&self, measured: &SideDistances, side: RectangleSide) -> Nearest {
        let mut nearest = Nearest {
            lower: None,
            sure: None,
        };
        for distance in measured.beside(side) {
            let length = distance.distance();
            let (lower, upper) = (length.lower_metres(), length.upper_metres());
            nearest.lower = Some(nearest.lower.map_or(lower, |known: f64| known.min(lower)));
            if distance.presence() == SidePresence::Sure
                && self.is_sure(distance.candidate())
                && nearest
                    .sure
                    .as_ref()
                    .is_none_or(|(_, _, known)| upper < *known)
            {
                nearest.sure = Some((distance.candidate().clone(), lower, upper));
            }
        }
        nearest
    }

    /// The side `object` stands against: the one whose nearest wall, from
    /// the side itself, is surely nearer than any other side's.
    pub(crate) fn back(&self, measured: &SideDistances) -> Result<Back, Unavailable> {
        let rectangle = measured.rectangle();
        let halves = rectangle.half_extents_metres();
        let gaps: Vec<(RectangleSide, Nearest)> = RectangleSide::ALL
            .into_iter()
            .map(|side| {
                // From the side itself: less the half extent along its axis.
                let (half_low, half_high) = halves[side.axis()];
                let nearest = self.nearest(measured, side);
                let gap = Nearest {
                    lower: nearest.lower.map(|lower| lower - half_high),
                    sure: nearest
                        .sure
                        .map(|(wall, lower, upper)| (wall, lower - half_high, upper - half_low)),
                };
                (side, gap)
            })
            .collect();
        let object = rectangle.object();
        if gaps.iter().all(|(_, gap)| gap.lower.is_none()) {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "front not decided: no wall lies within {} of {object}'s centre lines",
                    metres(measured.request().reach_metres())
                ),
            ));
        }
        for (side, gap) in &gaps {
            let Some((wall, _, upper)) = &gap.sure else {
                continue;
            };
            let nearer = gaps
                .iter()
                .filter(|(other, _)| other != side)
                .all(|(_, other)| other.lower.is_none_or(|lower| *upper < lower));
            if nearer {
                return Ok(Back {
                    side: *side,
                    evidence: vec![
                        measured.evidence().clone(),
                        rectangle.evidence().clone(),
                        Evidence {
                            source: object.source.clone(),
                            locator: format!(
                                "axioval:derived.front:{object}:against={wall}:side={}",
                                side.name()
                            ),
                            exact: false,
                        },
                    ],
                });
            }
        }
        Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!(
                "front not decided: no side of {object} is surely nearer a wall than the others"
            ),
        ))
    }
}

/// The nearest wall beside a side: `lower` bounds it over every wall that
/// may lie there, and `sure` is the sure wall surely there with the least
/// upper bound, with its `(lower, upper)` distance.
pub(crate) struct Nearest {
    pub(crate) lower: Option<f64>,
    pub(crate) sure: Option<(ObjectId, f64, f64)>,
}

/// The side a component stands against and the evidence naming the wall.
pub(crate) struct Back {
    pub(crate) side: RectangleSide,
    pub(crate) evidence: Vec<Evidence>,
}

pub(crate) fn unavailable(object: &ObjectId, error: &PlanSpanError) -> Unavailable {
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
        format!("the walls beside {object} cannot be measured: {error}"),
    )
}
