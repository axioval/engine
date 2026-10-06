//! The spaces a door opens onto as measured members, probed as
//! `door-swing` probes them: each with whether a selection picks it,
//! whether the door swings into it and whether it surely swings away from
//! it.
//!
//! A door's leaves are read once per run, and each space it may open onto
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
        let leaves = MeasuredMemo::of(context.services, LeavesKey(door.id.clone()), || {
            hinged_leaves(frames, door).map(Arc::new)
        })?;
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
                })
            })
            .collect())
    }
}

impl MeasuredProvider for SwingMeasures {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[SWING_SPACES]
    }

    fn measure(
        &self,
        _: &MeasuredCall,
        _: &ObjectId,
        _: &RuleContext<'_>,
    ) -> Result<Measurement, PropertyResolutionError> {
        Err(PropertyResolutionError::InvalidRequest)
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
