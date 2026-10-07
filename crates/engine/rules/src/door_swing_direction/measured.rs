//! The spaces a door opens onto as measured members, probed as
//! `door-swing` probes them: each with whether a selection picks it,
//! whether the door swings into it and whether it surely swings away from
//! it.
//!
//! How many hinged leaves a door has (`hinged_leaves`) is the guard a
//! template reads first: an object without them is refused before its
//! spaces are listed. A door's leaves, once read, are kept for the run
//! (an object refused is kept nowhere), and each space it may open onto
//! is probed once per run, however many lists name it.

use std::sync::Arc;

use axioval_engine::{
    DoorLeaves, FreeSpaceServiceHandle, MeasuredMember, MeasuredMemo, MeasuredProvider,
    Measurement, MemberValue, NotEvaluatedReason, ObjectFrameServiceHandle,
    PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, Object, ObjectId};

use super::hinged_leaves;
use crate::door_swing::{self, Relation};
use crate::measured_kinds::{objects_of_kinds, resolution_error, selection_cow};
use crate::support::{Traversal, Unavailable, invalid};

/// The member list measured here.
const SWING_SPACES: &str = "swing_spaces";
/// How many hinged leaves a door has.
const HINGED_LEAVES: &str = "hinged_leaves";

/// Measures the spaces a door swings into and away from.
pub(crate) struct SwingMeasures;

fn missing(service: &str) -> Unavailable {
    (
        NotEvaluatedReason::MissingService,
        format!("the {service} service is not registered"),
    )
}

/// The key of a door's hinged leaves in the run's memo.
#[derive(Hash, PartialEq, Eq)]
struct LeavesKey(ObjectId);

/// The hinged leaves of `door`, kept for the run once read: the guard,
/// read first, reads and keeps them (`kept` false: nothing to look up yet),
/// the list finds them. A refusal is not kept, so an object without leaves
/// costs the run nothing.
fn leaves_of(
    context: &RuleContext<'_>,
    frames: &ObjectFrameServiceHandle,
    door: &ObjectId,
    kept: bool,
) -> Result<Arc<DoorLeaves>, Unavailable> {
    leaves_in(context.services.get::<MeasuredMemo>(), frames, door, kept)
}

/// [`leaves_of`], the run's memo looked up already.
fn leaves_in(
    memo: Option<&MeasuredMemo>,
    frames: &ObjectFrameServiceHandle,
    door: &ObjectId,
    kept: bool,
) -> Result<Arc<DoorLeaves>, Unavailable> {
    if kept
        && let Some(leaves) =
            memo.and_then(|memo| memo.get::<_, Arc<DoorLeaves>>(&LeavesKey(door.clone())))
    {
        return Ok(leaves);
    }
    let leaves = Arc::new(hinged_leaves(frames, door)?);
    if let Some(memo) = memo {
        memo.insert(LeavesKey(door.clone()), Arc::clone(&leaves));
    }
    Ok(leaves)
}

/// The key of where a space lies against a door's swing.
#[derive(Hash, PartialEq, Eq)]
struct RelationKey(ObjectId, ObjectId);

/// Where `space` lies against the swing of `door`'s `leaves`, probed once
/// per run.
fn relation(
    context: &RuleContext<'_>,
    free: &FreeSpaceServiceHandle,
    (door, leaves): (&ObjectId, &DoorLeaves),
    space: &ObjectId,
) -> Result<(Relation, Vec<Evidence>), Unavailable> {
    MeasuredMemo::of(
        context.services,
        RelationKey(door.clone(), space.clone()),
        || door_swing::relation(free, leaves, space),
    )
}

/// How many hinged leaves `object` has, refused for one with none.
fn hinged(
    frames: Option<&ObjectFrameServiceHandle>,
    memo: Option<&MeasuredMemo>,
    object: &ObjectId,
) -> Result<Measurement, PropertyResolutionError> {
    let refused = crate::measured_kinds::refused(HINGED_LEAVES, object);
    let frames = frames.ok_or_else(|| refused(missing("object-frame")))?;
    let leaves = leaves_in(memo, frames, object, false).map_err(&refused)?;
    #[allow(clippy::cast_precision_loss)]
    let count = leaves.hinged().count() as f64;
    Ok(crate::measured_kinds::interval(
        (count, count),
        None,
        leaves.evidence().exact,
        format!("{HINGED_LEAVES}:{object}"),
    ))
}

impl SwingMeasures {
    fn spaces(
        call: &MeasuredCall,
        door: &Object,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, Unavailable> {
        let frames = context
            .services
            .get::<ObjectFrameServiceHandle>()
            .ok_or_else(|| missing("object-frame"))?;
        let free = context
            .services
            .get::<FreeSpaceServiceHandle>()
            .ok_or_else(|| missing("free-space"))?;
        let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
            return Err(invalid("`path` is required"));
        };
        let leaves = leaves_of(context, frames, &door.id, true)?;
        let everything: Vec<&Object> = context.project.objects().collect();
        let (mut reached, _) = Traversal::path(steps)?.related(context, &door.id, &everything)?;
        if call.argument("kinds").is_some() {
            let kinds = objects_of_kinds(context, call, "kinds", &door.id)
                .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
            reached.retain(|space| kinds.contains(space));
        }
        let read = |key: &str| {
            selection_cow(context, call, key)
                .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))
        };
        let (towards, not_towards) = (read("towards")?, read("not_towards")?);
        let selections = [&towards, &not_towards];
        Ok(reached
            .iter()
            .filter_map(|space| {
                let truth = |value: bool| MemberValue::Truth {
                    value,
                    locator: format!("{SWING_SPACES}:{}:{space}", door.id),
                };
                let undecided = |why: String| MemberValue::Undecided { why };
                // Only the spaces a selection may pick are probed.
                if selections.iter().any(|selection| selection.is_some())
                    && !selections.iter().any(|selection| {
                        selection.as_ref().is_some_and(|selection| {
                            selection.matched.contains(space) || selection.undecided.contains(space)
                        })
                    })
                {
                    return None;
                }
                let chosen = |selection: &Option<
                    std::borrow::Cow<'_, axioval_ir::measured::MeasuredSelection>,
                >| match selection {
                    None => truth(true),
                    Some(picked) if picked.matched.contains(space) => truth(true),
                    Some(picked) if picked.undecided.contains(space) => {
                        undecided(format!("whether the selection picks {space} is undecided"))
                    }
                    Some(_) => truth(false),
                };
                let mut exact = false;
                let (into, away) = match relation(context, free, (&door.id, &leaves), space) {
                    Ok((relation, evidence)) => {
                        // As exact as the leaves and every probe were.
                        exact = evidence.iter().all(|evidence| evidence.exact);
                        (
                            truth(relation.swings_into()),
                            match relation {
                                Relation::Away => truth(true),
                                Relation::Into | Relation::BothWays => truth(false),
                                Relation::Apart => undecided(format!(
                                    "neither side of the door lies in {space} at its probes"
                                )),
                            },
                        )
                    }
                    Err((_, why)) => (undecided(why.clone()), undecided(why)),
                };
                Some(MeasuredMember {
                    certain: true,
                    exact,
                    fields: [
                        (
                            "space",
                            MemberValue::Objects {
                                objects: vec![space.clone()],
                            },
                        ),
                        ("towards", chosen(&towards)),
                        ("not_towards", chosen(&not_towards)),
                        ("into", into),
                        ("away", away),
                    ]
                    .into_iter()
                    .collect(),
                    evidence: Vec::new(),
                })
            })
            .collect())
    }
}

impl MeasuredProvider for SwingMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[HINGED_LEAVES]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[SWING_SPACES]
    }

    fn measure(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        if call.name() != HINGED_LEAVES {
            return Err(PropertyResolutionError::InvalidRequest);
        }
        let frames = context.services.get::<ObjectFrameServiceHandle>();
        hinged(frames, context.services.get::<MeasuredMemo>(), object)
    }

    /// The doors' hinged leaves, the services looked up once for them all.
    fn measure_batch(
        &self,
        call: &MeasuredCall,
        objects: &[&ObjectId],
        context: &RuleContext<'_>,
    ) -> Vec<Result<Measurement, PropertyResolutionError>> {
        if call.name() != HINGED_LEAVES {
            return objects
                .iter()
                .map(|_| Err(PropertyResolutionError::InvalidRequest))
                .collect();
        }
        let frames = context.services.get::<ObjectFrameServiceHandle>();
        let memo = context.services.get::<MeasuredMemo>();
        objects
            .iter()
            .map(|object| hinged(frames, memo, object))
            .collect()
    }

    fn members(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<Vec<MeasuredMember>, PropertyResolutionError> {
        let door = context.project.object(object).ok_or_else(|| {
            PropertyResolutionError::Unavailable(format!("{object} is not in the project"))
        })?;
        Self::spaces(call, door, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
