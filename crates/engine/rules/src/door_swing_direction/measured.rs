//! The spaces a door opens onto as measured members, probed as
//! `door-swing` probes them: each with whether the door swings into it and
//! whether it surely swings away from it.

use axioval_engine::{
    FreeSpaceServiceHandle, MeasuredMember, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, ObjectFrameServiceHandle, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Object, ObjectId};

use super::hinged_leaves;
use crate::door_swing::{self, Relation};
use crate::measured_kinds::{objects_of_kinds, resolution_error};
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
        let leaves = hinged_leaves(frames, door)?;
        let everything: Vec<&Object> = context.project.objects().collect();
        let (mut reached, _) = Traversal::path(steps)?.related(context, &door.id, &everything)?;
        if call.argument("kinds").is_some() {
            let kinds = objects_of_kinds(context, call, "kinds", &door.id)
                .map_err(|error| (NotEvaluatedReason::BackendUnavailable, error.to_string()))?;
            reached.retain(|space| kinds.contains(space));
        }
        Ok(reached
            .iter()
            .map(|space| {
                let truth = |value: bool| MemberValue::Truth {
                    value,
                    locator: format!("{SWING_SPACES}:{}:{space}", door.id),
                };
                let undecided = |why: String| MemberValue::Undecided { why };
                let (into, away) = match door_swing::relation(free, &leaves, space) {
                    Ok((relation, _)) => (
                        truth(relation.swings_into()),
                        match relation {
                            Relation::Away => truth(true),
                            Relation::Into | Relation::BothWays => truth(false),
                            Relation::Apart => undecided(format!(
                                "neither side of the door lies in {space} at its probes"
                            )),
                        },
                    ),
                    Err((_, why)) => (undecided(why.clone()), undecided(why)),
                };
                MeasuredMember {
                    certain: true,
                    exact: true,
                    fields: [("into", into), ("away", away)].into_iter().collect(),
                }
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
