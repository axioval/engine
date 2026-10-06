//! The `door-swing` implementation the template replaced, kept as the
//! template's parity reference.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, FreeSpaceServiceHandle, NotEvaluatedReason,
    ObjectFrameServiceHandle, ParameterDescriptor, ParameterType, RuleCapability, RuleContext,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use super::hinged_leaves;
use crate::door_swing::{self, Relation};
use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Requires each selected door to swing into, or not into, the spaces it
/// opens onto.
///
/// A door's spaces are what `space_path` reaches from it. Its swing comes
/// from its leaves as the object-frame service states them (with IFC, the
/// operation type, the panels and the placement). Which side of the door a
/// space lies on is asked of the free-space service at two small probes per
/// hinged leaf, halfway through its sweep and three quarters of its width
/// out from the hinge: one on the side it opens into, one behind it.
///
/// - `swing_into`: among the reached spaces this selector picks, the door
///   must swing into at least one. It is a finding when every picked space
///   surely lies behind the door and none on its swing side.
/// - `swing_not_into`: the door must swing into none of the reached spaces
///   this selector picks.
///
/// A double-acting leaf swings into the spaces on both sides. A door
/// without a hinged leaf (sliding, rolling up) swings into no space and is
/// not evaluated, like a door whose leaves cannot be read. A space whose
/// selection is undecided, a probe the service cannot answer, and a space
/// neither probe lies in decide only what they cannot change.
pub struct DoorSwing;

const ID: &str = "axioval:capability.door-swing";

struct Config<'a> {
    spaces: Traversal,
    into: Option<&'a Selector>,
    not_into: Option<&'a Selector>,
}

impl<'a> Config<'a> {
    fn parse(rule: &'a CompiledRule) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let path = parameters
            .strings("space_path")?
            .ok_or_else(|| invalid("parameter `space_path` is required"))?;
        let into = parameters.selector("swing_into")?;
        let not_into = parameters.selector("swing_not_into")?;
        if into.is_none() && not_into.is_none() {
            return Err(invalid("declare `swing_into`, `swing_not_into` or both"));
        }
        Ok(Self {
            spaces: Traversal::path(path)?,
            into,
            not_into,
        })
    }
}

impl RuleCapability for DoorSwing {
    fn id(&self) -> &'static str {
        ID
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("space_path", ParameterType::StringList),
            ParameterDescriptor::optional("swing_into", ParameterType::Selector),
            ParameterDescriptor::optional("swing_not_into", ParameterType::Selector),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let config = match Config::parse(rule) {
            Ok(config) => config,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("door-swing: {message}"),
                );
            }
        };
        let (Some(frames), Some(free)) = (
            context.services.get::<ObjectFrameServiceHandle>(),
            context.services.get::<FreeSpaceServiceHandle>(),
        ) else {
            return CapabilityEvaluation::not_evaluated(
                NotEvaluatedReason::MissingService,
                "door-swing needs the object-frame and free-space services",
            );
        };
        let (doors, mut evaluation) = select_objects(context, &rule.selector);
        let everything: Vec<&Object> = context.project.objects().collect();
        for door in doors {
            let judged = check(context, &config, frames, free, &everything, door);
            let Judged {
                findings,
                doubts,
                evidence,
            } = match judged {
                Ok(judged) => judged,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(
                        door.id.clone(),
                        reason,
                        format!("door-swing: {message}"),
                    );
                    continue;
                }
            };
            for (message, related) in findings {
                evaluation.push_finding(finding(
                    rule,
                    &door.id,
                    message,
                    evidence.clone(),
                    related,
                ));
            }
            if let Some((reason, message)) = doubts.into_iter().next() {
                evaluation.push_object_not_evaluated(
                    door.id.clone(),
                    reason,
                    format!("door-swing: {message}"),
                );
            }
        }
        evaluation
    }
}

/// What checking one door found.
#[derive(Default)]
struct Judged {
    findings: Vec<(String, Vec<ObjectId>)>,
    doubts: Vec<Unavailable>,
    evidence: Vec<Evidence>,
}

fn check(
    context: &RuleContext<'_>,
    config: &Config<'_>,
    frames: &ObjectFrameServiceHandle,
    free: &FreeSpaceServiceHandle,
    everything: &[&Object],
    door: &Object,
) -> Result<Judged, Unavailable> {
    let leaves = hinged_leaves(frames, &door.id)?;
    let (reached, cited) = config.spaces.related(context, &door.id, everything)?;
    let mut judged = Judged {
        evidence: cited,
        ..Judged::default()
    };
    judged.evidence.push(leaves.evidence().clone());
    let mut relations: BTreeMap<ObjectId, Result<Relation, Unavailable>> = BTreeMap::new();
    let mut relation = |space: &ObjectId, evidence: &mut Vec<Evidence>| {
        relations
            .entry(space.clone())
            .or_insert_with(|| {
                door_swing::relation(free, &leaves, space).map(|(relation, proof)| {
                    evidence.extend(proof);
                    relation
                })
            })
            .clone()
    };
    let picked = |selector: &Selector, space: &ObjectId, evidence: &mut Vec<Evidence>| {
        context
            .project
            .object(space)
            .map_or(Selection::NoMatch, |object| {
                selector_matches(context, selector, object, evidence)
            })
    };
    if let Some(selector) = config.not_into {
        for space in &reached {
            let selection = picked(selector, space, &mut judged.evidence);
            if matches!(selection, Selection::NoMatch) {
                continue;
            }
            match (relation(space, &mut judged.evidence), selection) {
                (Ok(found), Selection::Match) if found.swings_into() => judged.findings.push((
                    format!("swings into {space}, which `swing_not_into` forbids"),
                    vec![space.clone()],
                )),
                (Ok(found), _) if found.swings_into() => judged.doubts.push((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!("it swings into {space}, which `swing_not_into` may pick"),
                )),
                (Ok(_), _) => {}
                (Err(unavailable), _) => judged.doubts.push(unavailable),
            }
        }
    }
    if let Some(selector) = config.into {
        let mut sure_away = Vec::new();
        let mut open = None;
        let mut satisfied = false;
        for space in &reached {
            let selection = picked(selector, space, &mut judged.evidence);
            if matches!(selection, Selection::NoMatch) {
                continue;
            }
            match (relation(space, &mut judged.evidence), &selection) {
                (Ok(found), Selection::Match) if found.swings_into() => satisfied = true,
                (Ok(Relation::Away), Selection::Match) => sure_away.push(space.clone()),
                (Ok(Relation::Away), _) => {}
                (Ok(_), _) => {
                    open.get_or_insert((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!("whether it swings into {space} is undecided"),
                    ));
                }
                (Err(unavailable), _) => {
                    open.get_or_insert(unavailable);
                }
            }
        }
        match (satisfied, open) {
            (false, Some(unavailable)) => judged.doubts.push(unavailable),
            (false, None) if !sure_away.is_empty() => {
                let names: Vec<String> = sure_away.iter().map(ToString::to_string).collect();
                judged.findings.push((
                    format!(
                        "swings away from {}, which `swing_into` requires it to swing into",
                        names.join(", ")
                    ),
                    sure_away,
                ));
            }
            _ => {}
        }
    }
    Ok(judged)
}
