//! The `corridor-end-openings` implementation the template replaced, kept
//! as the template's parity reference.
//!
//! Each selected space (a corridor, by type) reaches its openings through
//! `opening_path` (for example `axioval:derived.adjacent-space:backward`),
//! filtered by `opening_selector` (windows). The plan-span service finds the
//! ends of the paths through the space's footprint and the wall each runs
//! into, and measures every opening against each such wall: its plan gap to
//! the wall segment and the length of the segment it faces.
//!
//! An opening sits in an end wall when it lies within `wall_depth` of it
//! (half a metre by default: the depth of a thick wall behind its face) and
//! faces more than `facing` of it (a tenth of a metre by default, so a
//! window in a side wall beside the corner, which faces none of it, does
//! not). It surely sits there when the whole gap interval lies within the
//! depth and the whole facing interval beyond the minimum; surely not when
//! either lies wholly on the other side. Anything else is undecided, and so
//! is every opening of a space one of whose ends runs into a wall the
//! service could not name: the opening might sit in it.
//!
//! The ends themselves come from an approximate skeleton, so their evidence
//! is approximate; the gap and facing lengths are measured on the
//! footprints. An opening whose selection is undecided is found only as
//! not evaluated, and only where it would sit in an end wall.

use std::collections::BTreeMap;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, CorridorEndRequest, CorridorEnds, EndWall,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PlanSpanError, PlanSpanServiceHandle,
    RuleCapability, RuleContext, WallContact,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Object, ObjectId};

use crate::plan_area::shown;
use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Finds openings in the wall a selected corridor ends at.
pub struct CorridorEndOpenings;

struct Declaration<'a> {
    openings: Traversal,
    opening_selector: &'a Selector,
    wall_depth: f64,
    facing: f64,
}

fn length(parameters: &Parameters<'_>, name: &str, default: f64) -> Result<f64, Unavailable> {
    match parameters.number(name)? {
        None => Ok(default),
        Some(value) if value.is_finite() && value >= 0.0 => Ok(value),
        Some(_) => Err(invalid(format!(
            "`{name}` must be a non-negative length in metres"
        ))),
    }
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    Ok(Declaration {
        openings: Traversal::path(
            parameters
                .strings("opening_path")?
                .ok_or_else(|| invalid("parameter `opening_path` is required"))?,
        )?,
        opening_selector: parameters.required_selector("opening_selector")?,
        wall_depth: length(&parameters, "wall_depth", 0.5)?,
        facing: length(&parameters, "facing", 0.1)?,
    })
}

impl RuleCapability for CorridorEndOpenings {
    fn id(&self) -> &'static str {
        "axioval:capability.corridor-end-openings"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        vec![
            ParameterDescriptor::required("opening_path", ParameterType::StringList),
            ParameterDescriptor::required("opening_selector", ParameterType::Selector),
            ParameterDescriptor::optional("wall_depth", ParameterType::Number),
            ParameterDescriptor::optional("facing", ParameterType::Number),
        ]
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("corridor-end-openings: {message}"),
                );
            }
        };
        let (universe, undecided) = candidates(context, declared.opening_selector);
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        for space in spaces {
            match check(context, rule, &declared, &universe, &undecided, space) {
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(space.id.clone(), reason, message);
                }
                Ok(outcomes) => {
                    for outcome in outcomes {
                        match outcome {
                            Outcome::Found(found) => evaluation.push_finding(*found),
                            Outcome::Unknown(opening, reason, message) => {
                                evaluation.push_object_not_evaluated(opening, reason, message);
                            }
                        }
                    }
                }
            }
        }
        evaluation
    }
}

/// The objects `opening_selector` picks, with those it cannot decide and why.
fn candidates<'a>(
    context: &RuleContext<'a>,
    selector: &Selector,
) -> (Vec<&'a Object>, BTreeMap<ObjectId, String>) {
    let mut universe = Vec::new();
    let mut undecided = BTreeMap::new();
    for object in context.project.objects() {
        match selector_matches(context, selector, object, &mut Vec::new()) {
            Selection::Match => universe.push(object),
            Selection::NoMatch => {}
            Selection::NotEvaluated(_, message) => {
                undecided.insert(object.id.clone(), message);
                universe.push(object);
            }
        }
    }
    (universe, undecided)
}

enum Outcome {
    Found(Box<axioval_ir::Finding>),
    Unknown(ObjectId, NotEvaluatedReason, String),
}

/// Where an opening stands against one end wall.
enum Standing {
    In,
    Out,
    Unknown(String),
}

fn standing(declared: &Declaration<'_>, contact: &WallContact) -> Standing {
    let (gap, facing) = (contact.gap(), contact.facing());
    if gap.lower_metres() > declared.wall_depth || facing.upper_metres() <= declared.facing {
        return Standing::Out;
    }
    if gap.upper_metres() <= declared.wall_depth && facing.lower_metres() > declared.facing {
        return Standing::In;
    }
    Standing::Unknown(format!(
        "it lies {} m from an end wall and faces {} m of it, which straddles \
         within {} m and more than {} m",
        shown(gap.lower_metres(), gap.upper_metres()),
        shown(facing.lower_metres(), facing.upper_metres()),
        declared.wall_depth,
        declared.facing
    ))
}

fn check(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    declared: &Declaration<'_>,
    universe: &[&Object],
    undecided: &BTreeMap<ObjectId, String>,
    space: &Object,
) -> Result<Vec<Outcome>, Unavailable> {
    let (openings, path_evidence) = declared.openings.related(context, &space.id, universe)?;
    if openings.is_empty() {
        return Ok(Vec::new());
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
    let mut outcomes = Vec::new();
    for (index, opening) in request.subjects().iter().enumerate() {
        let judged = judge(declared, &ends, index);
        match (judged, undecided.get(opening)) {
            (Judged::In(walls), None) => {
                let mut evidence = path_evidence.clone();
                evidence.push(ends.evidence().clone());
                let mut described = Vec::new();
                for (start, end, contact) in walls {
                    evidence.push(contact.gap().evidence().clone());
                    evidence.push(contact.facing().evidence().clone());
                    described.push(format!(
                        "{} m from the wall {} and facing {} m of it",
                        shown(contact.gap().lower_metres(), contact.gap().upper_metres()),
                        wall(start, end),
                        shown(
                            contact.facing().lower_metres(),
                            contact.facing().upper_metres()
                        ),
                    ));
                }
                outcomes.push(Outcome::Found(Box::new(finding(
                    rule,
                    opening,
                    format!(
                        "sits in the end wall of corridor {}: {}",
                        space.id,
                        described.join("; ")
                    ),
                    evidence,
                    vec![space.id.clone()],
                ))));
            }
            (Judged::In(_), Some(why)) => outcomes.push(Outcome::Unknown(
                opening.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "sits in the end wall of corridor {}, but whether it is selected is \
                     undecided: {why}",
                    space.id
                ),
            )),
            (Judged::Out, _) | (Judged::Unknown(_), Some(_)) => {}
            (Judged::Unknown(why), None) => outcomes.push(Outcome::Unknown(
                opening.clone(),
                NotEvaluatedReason::IncompleteEvidence,
                format!(
                    "whether it sits in an end wall of corridor {} is undecided: {}",
                    space.id,
                    why.join("; ")
                ),
            )),
        }
    }
    Ok(outcomes)
}

/// How an opening stands against every end of a corridor.
enum Judged<'a> {
    /// In these end walls.
    In(Vec<([f64; 2], [f64; 2], &'a WallContact)>),
    /// In none.
    Out,
    /// Possibly in one; why.
    Unknown(Vec<String>),
}

fn judge<'a>(declared: &Declaration<'_>, ends: &'a CorridorEnds, index: usize) -> Judged<'a> {
    let mut walls = Vec::new();
    let mut unknown = Vec::new();
    for corridor_end in ends.ends() {
        let [x, y] = corridor_end.point();
        match corridor_end.wall() {
            EndWall::Undecided(why) => unknown.push(format!(
                "the wall the end near ({}, {}) runs into is undecided: {why}",
                shown(x, x),
                shown(y, y)
            )),
            EndWall::Decided {
                start,
                end,
                contacts,
            } => match standing(declared, &contacts[index]) {
                Standing::In => walls.push((*start, *end, &contacts[index])),
                Standing::Out => {}
                Standing::Unknown(why) => unknown.push(format!("{why} ({})", wall(*start, *end))),
            },
        }
    }
    if !walls.is_empty() {
        Judged::In(walls)
    } else if unknown.is_empty() {
        Judged::Out
    } else {
        Judged::Unknown(unknown)
    }
}

fn wall(start: [f64; 2], end: [f64; 2]) -> String {
    format!(
        "({}, {})–({}, {})",
        shown(start[0], start[0]),
        shown(start[1], start[1]),
        shown(end[0], end[0]),
        shown(end[1], end[1])
    )
}

fn unavailable(error: &PlanSpanError) -> Unavailable {
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
        format!("the corridor ends cannot be measured: {error}"),
    )
}
