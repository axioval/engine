//! The openings a corridor reaches, each searched against the corridor's
//! end walls as `corridor-end-openings` searches them: whether it sits in
//! one (`sits`, undecided where an end wall or a contact cannot decide it),
//! the walls it sits in (`walls`), and whether the openings' selection
//! picks it (`picked`, its reason `unpicked` where it cannot decide).

use axioval_engine::{
    CorridorEndRequest, MeasuredMember, MeasuredProvider, Measurement, MemberValue,
    NotEvaluatedReason, PlanSpanServiceHandle, PropertyResolutionError, RuleContext,
};
use axioval_ir::measured::{MeasuredArgument, MeasuredCall};
use axioval_ir::{Evidence, Object, ObjectId};

use super::{Judged, Margins, described, judge, unavailable};
use crate::measured_kinds::{resolution_error, selection_cow};
use crate::support::{Traversal, Unavailable, invalid};

/// The member list measured here.
const CORRIDOR_END_OPENINGS: &str = "corridor_end_openings";

/// Searches the openings a corridor reaches against its end walls.
pub(crate) struct CorridorEndSearch;

impl CorridorEndSearch {
    fn openings(
        call: &MeasuredCall,
        space: &Object,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), Unavailable> {
        let Some(MeasuredArgument::Path(steps)) = call.argument("path") else {
            return Err(invalid("`path` is required"));
        };
        let picked = selection_cow(context, call, "openings")
            .map_err(|error| (NotEvaluatedReason::IncompleteEvidence, error.to_string()))?
            .ok_or_else(|| invalid("`openings` is required"))?;
        let number = |key: &str, default: f64| match call.argument(key) {
            Some(MeasuredArgument::Number(value)) => *value,
            _ => default,
        };
        let margins = Margins {
            wall_depth: number("depth", 0.5),
            facing: number("facing", 0.1),
        };
        let universe: Vec<&Object> = context
            .project
            .objects()
            .filter(|object| {
                picked.matched.contains(&object.id) || picked.undecided.contains(&object.id)
            })
            .collect();
        let (openings, mut cited) =
            Traversal::path(steps)?.related(context, &space.id, &universe)?;
        if openings.is_empty() {
            return Ok((Vec::new(), cited));
        }
        let spans = context
            .services
            .get::<PlanSpanServiceHandle>()
            .ok_or_else(|| {
                (
                    NotEvaluatedReason::MissingService,
                    "plan-span service is not registered".to_owned(),
                )
            })?;
        let request = CorridorEndRequest::try_new(space.id.clone(), openings.iter().cloned())
            .map_err(|error| unavailable(&error))?;
        let ends = spans
            .measure_corridor_ends(&request)
            .map_err(|error| unavailable(&error))?;
        let exact = ends.evidence().exact;
        // The path, the ends and every contact with a decided end wall.
        cited.push(ends.evidence().clone());
        for end in ends.ends() {
            if let axioval_engine::EndWall::Decided { contacts, .. } = end.wall() {
                for contact in contacts {
                    cited.push(contact.gap().evidence().clone());
                    cited.push(contact.facing().evidence().clone());
                }
            }
        }
        let members = request
            .subjects()
            .iter()
            .enumerate()
            .map(|(index, opening)| {
                let objects = |object: &ObjectId| MemberValue::Objects {
                    objects: vec![object.clone()],
                };
                let truth = |value: bool| MemberValue::Truth {
                    value,
                    locator: format!("{CORRIDOR_END_OPENINGS}:{}:{opening}", space.id),
                };
                let (sits, walls) = match judge(margins, &ends, index) {
                    Judged::In(walls) => (truth(true), described(&walls)),
                    Judged::Out => (truth(false), String::new()),
                    Judged::Unknown(why) => (
                        MemberValue::Undecided {
                            why: why.join("; "),
                        },
                        String::new(),
                    ),
                };
                let (chosen, unpicked) = match picked.reasons.get(opening) {
                    _ if picked.matched.contains(opening) => (truth(true), String::new()),
                    Some(why) => (MemberValue::Undecided { why: why.clone() }, why.clone()),
                    None => (
                        MemberValue::Undecided {
                            why: format!("whether {opening} is selected is undecided"),
                        },
                        format!("whether {opening} is selected is undecided"),
                    ),
                };
                MeasuredMember {
                    certain: true,
                    exact,
                    fields: [
                        ("opening", objects(opening)),
                        ("corridor", objects(&space.id)),
                        ("picked", chosen),
                        ("unpicked", MemberValue::Text { text: unpicked }),
                        ("sits", sits),
                        ("walls", MemberValue::Text { text: walls }),
                    ]
                    .into_iter()
                    .collect(),
                }
            })
            .collect();
        Ok((members, cited))
    }
}

impl MeasuredProvider for CorridorEndSearch {
    fn names(&self) -> &'static [&'static str] {
        &[]
    }

    fn member_lists(&self) -> &'static [&'static str] {
        &[CORRIDOR_END_OPENINGS]
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
        self.members_cited(call, object, context)
            .map(|(members, _)| members)
    }

    /// The members, citing the path, the corridor's ends and every contact
    /// with a decided end wall.
    fn members_cited(
        &self,
        call: &MeasuredCall,
        object: &ObjectId,
        context: &RuleContext<'_>,
    ) -> Result<(Vec<MeasuredMember>, Vec<Evidence>), PropertyResolutionError> {
        let space = context.project.object(object).ok_or_else(|| {
            PropertyResolutionError::Unavailable(format!("{object} is not in the project"))
        })?;
        Self::openings(call, space, context).map_err(|(reason, why)| {
            resolution_error((reason, format!("`{}` of {object}: {why}", call.name())))
        })
    }
}
