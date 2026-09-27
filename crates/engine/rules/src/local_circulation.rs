//! Local circulation: within each selected space, a path `width_metres`
//! wide from the entrances must reach the space's components.
//!
//! The entrances are the doors and openings `access_path` relates to the
//! space (with `axioval:derived.adjacent-space`, those opening onto it); the
//! components are those `component_selector` picks whose `space_path` leads
//! to the space. The free-space service maps the space's circulation for the
//! width (see [`CirculationMap`]): pieces of the free area eroded by half the
//! width from inside and from outside, their skeleton, and which pieces come
//! within half the width plus `tolerance_metres` of each entrance and
//! component.
//!
//! - A component is **reached** when a piece near it is also near a surely
//!   selected entrance, and **cut off** when no possible piece near it is
//!   near any entrance, sure or possible. With `component_mode` `link` the
//!   components must instead be linked with one another, entrances aside.
//! - Each **path end** (a skeleton end of a piece an entrance reaches) must
//!   offer a free area `end_width_metres` across by `end_length_metres`
//!   along the path, `clear_height_metres` high, with its centre within
//!   `end_reach_metres` of the end, unless the end is short (its branch,
//!   from where it leaves the rest of the path to the wall it ends at, is
//!   shorter than `short_end_metres`) or narrow (the free width there is
//!   less than `narrow_end_metres`). The branch is measured along the
//!   skeleton, whose node positions are approximate: its length is widened
//!   by two sample spacings each way.
//! - With the passing-space parameters, the path from each entrance to each
//!   component it reaches, along the skeleton, must offer a free box that
//!   size at most every `passing_spacing_metres` (see
//!   [`crate::passing_spaces`]).
//!
//! With `merge_path`, the spaces it reaches are mapped together with the
//! selected one as one walkable area: their entrances and components count
//! too. `band_from_metres` starts the obstacle band above the floor, so a
//! skirting below it leaves the path free. `end_exempt_selector` exempts a
//! path end within `end_exempt_reach_metres` of a selected object from
//! needing a free area. With `component_mode` `link_sets`, every component
//! must instead be linked with one of the `partner_selector` objects of its
//! space.
//!
//! With `subtract_door_swings`, the sectors the selected doors' leaves sweep
//! are obstacles too, for the path, its end areas and its passing spaces;
//! an entrance of the space is walked through, so its own swing never is.
//!
//! Three-valued throughout: a proof is a finding, a witness passes, and
//! anything between is not evaluated. An obstacle the selection cannot
//! decide, and a door whose swing may count but is unknown, leave every
//! space not evaluated.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use axioval_engine::{
    BoxClearance, CapabilityEvaluation, CirculationMap, CirculationNodeKind, CirculationRequest,
    CompiledRule, ConvexPlanRegion, ElevationBand, FrameOffsetPlacement, FreeSpaceError,
    FreeSpaceServiceHandle, MetricDirection, MetricFrame, MetricPoint, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, PlacementDomain, PlacementOrientation, PlacementOutcome,
    PlacementRequest, PlacementShape, ProximityServiceHandle, RegionDistanceRequest,
    RuleCapability, RuleContext, SignedDistanceInterval, SweptDoor,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::door_swing::Swings;
use crate::level_spacing::metres;
use crate::passing_spaces::{self, PassingSpaces, Spacing};
use crate::selection::select_objects;
use crate::space_access::{AccessDeclaration, AccessIndex, AccessType};
use crate::support::{Parameters, Traversal, Unavailable, finding, invalid};

/// Keeps an end's anchor on the floor.
const FLOOR_MARGIN: f64 = 1.0e-3;

/// A traced path is simplified to within this many path widths before its
/// passing spaces are searched.
const SIMPLIFY_WIDTHS: f64 = 0.25;

/// The default distance a subject may be from the path beyond its half
/// width.
const DEFAULT_TOLERANCE: f64 = 0.05;

/// Requires a path of a width from a space's entrances to its components.
pub struct LocalCirculation;

/// What the components must be to the path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Reached from an entrance.
    Touch,
    /// Linked with each other.
    Link,
    /// Each linked with one of the partners.
    LinkSets,
}

/// The free area each path end needs.
struct EndArea {
    width: f64,
    length: f64,
    reach: f64,
}

struct Declaration<'a> {
    components: &'a Selector,
    spaces: Traversal<'a>,
    access: AccessDeclaration<'a>,
    obstacles: Option<&'a Selector>,
    swings: Option<&'a Selector>,
    merge: Option<Traversal<'a>>,
    band_from: Option<f64>,
    exempt: Option<(&'a Selector, f64)>,
    partners: Option<&'a Selector>,
    width: f64,
    height: f64,
    tolerance: f64,
    mode: Mode,
    end: Option<EndArea>,
    short_end: Option<f64>,
    narrow_end: Option<f64>,
    passing: Option<PassingSpaces>,
}

fn positive(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.number(name)? {
        Some(value) if !(value.is_finite() && value > 0.0) => {
            Err(invalid(format!("`{name}` must be positive")))
        }
        other => Ok(other),
    }
}

#[allow(clippy::too_many_lines)]
fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let height = positive(&parameters, "clear_height_metres")?
        .ok_or_else(|| invalid("parameter `clear_height_metres` is required"))?;
    let access = AccessDeclaration::parse(&parameters)?
        .ok_or_else(|| invalid("parameter `access_path` is required: it finds the entrances"))?;
    let spaces = Traversal::path(
        parameters
            .strings("space_path")?
            .ok_or_else(|| invalid("parameter `space_path` is required"))?,
    )?;
    let tolerance = match parameters.number("tolerance_metres")? {
        Some(value) if !(value.is_finite() && value >= 0.0) => {
            return Err(invalid("`tolerance_metres` must not be negative"));
        }
        Some(value) => value,
        None => DEFAULT_TOLERANCE,
    };
    let mode = match parameters.string("component_mode")?.unwrap_or("touch") {
        "touch" => Mode::Touch,
        "link" => Mode::Link,
        "link_sets" => Mode::LinkSets,
        other => {
            return Err(invalid(format!(
                "component mode `{other}` is unsupported (touch, link, link_sets)"
            )));
        }
    };
    let partners = parameters.selector("partner_selector")?;
    if (mode == Mode::LinkSets) != partners.is_some() {
        return Err(invalid(
            "`component_mode` `link_sets` and `partner_selector` go together",
        ));
    }
    let band_from = match parameters.number("band_from_metres")? {
        Some(from) if !(from.is_finite() && from >= 0.0 && from < height) => {
            return Err(invalid(
                "`band_from_metres` must lie from the floor up to below `clear_height_metres`",
            ));
        }
        other => other,
    };
    let merge = match parameters.strings("merge_path")? {
        Some(path) => Some(Traversal::path(path)?),
        None => None,
    };
    let exempt = match (
        parameters.selector("end_exempt_selector")?,
        positive(&parameters, "end_exempt_reach_metres")?,
    ) {
        (Some(selector), Some(reach)) => Some((selector, reach)),
        (None, None) => None,
        _ => {
            return Err(invalid(
                "`end_exempt_selector` and `end_exempt_reach_metres` go together",
            ));
        }
    };
    let end = match (
        positive(&parameters, "end_width_metres")?,
        positive(&parameters, "end_length_metres")?,
        positive(&parameters, "end_reach_metres")?,
    ) {
        (Some(width), Some(length), reach) => Some(EndArea {
            width,
            length,
            reach: reach.unwrap_or(width.max(length) / 2.0),
        }),
        (None, None, None) => None,
        _ => {
            return Err(invalid(
                "`end_width_metres` and `end_length_metres` go together, and \
                 `end_reach_metres` needs them",
            ));
        }
    };
    let short_end = positive(&parameters, "short_end_metres")?;
    let narrow_end = positive(&parameters, "narrow_end_metres")?;
    if end.is_none() && (short_end.is_some() || narrow_end.is_some() || exempt.is_some()) {
        return Err(invalid(
            "`short_end_metres`, `narrow_end_metres` and `end_exempt_selector` exempt ends \
             from the free area `end_width_metres` and `end_length_metres` declare",
        ));
    }
    Ok(Declaration {
        components: parameters.required_selector("component_selector")?,
        spaces,
        access,
        obstacles: parameters.selector("obstacles")?,
        swings: parameters.selector("subtract_door_swings")?,
        merge,
        band_from,
        exempt,
        partners,
        width: positive(&parameters, "width_metres")?
            .ok_or_else(|| invalid("parameter `width_metres` is required"))?,
        height,
        tolerance,
        mode,
        end,
        short_end,
        narrow_end,
        passing: PassingSpaces::parse(&parameters, Some(height))?,
    })
}

impl RuleCapability for LocalCirculation {
    fn id(&self) -> &'static str {
        "axioval:capability.local-circulation"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("component_selector", ParameterType::Selector),
            ParameterDescriptor::required("space_path", ParameterType::StringList),
            ParameterDescriptor::required("access_path", ParameterType::StringList),
            ParameterDescriptor::optional("door_selector", ParameterType::Selector),
            ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
            ParameterDescriptor::optional("space_selector", ParameterType::Selector),
            ParameterDescriptor::optional("obstacles", ParameterType::Selector),
            ParameterDescriptor::optional("subtract_door_swings", ParameterType::Selector),
            ParameterDescriptor::required("width_metres", ParameterType::Number),
            ParameterDescriptor::required("clear_height_metres", ParameterType::Number),
            ParameterDescriptor::optional("tolerance_metres", ParameterType::Number),
            ParameterDescriptor::optional("component_mode", ParameterType::String),
            ParameterDescriptor::optional("end_width_metres", ParameterType::Number),
            ParameterDescriptor::optional("end_length_metres", ParameterType::Number),
            ParameterDescriptor::optional("end_reach_metres", ParameterType::Number),
            ParameterDescriptor::optional("short_end_metres", ParameterType::Number),
            ParameterDescriptor::optional("narrow_end_metres", ParameterType::Number),
            ParameterDescriptor::optional("merge_path", ParameterType::StringList),
            ParameterDescriptor::optional("band_from_metres", ParameterType::Number),
            ParameterDescriptor::optional("end_exempt_selector", ParameterType::Selector),
            ParameterDescriptor::optional("end_exempt_reach_metres", ParameterType::Number),
            ParameterDescriptor::optional("partner_selector", ParameterType::Selector),
        ];
        parameters.extend(passing_spaces::parameters());
        parameters
    }

    fn evaluate(&self, context: &RuleContext<'_>, rule: &CompiledRule) -> CapabilityEvaluation {
        let declared = match declaration(rule) {
            Ok(declared) => declared,
            Err((reason, message)) => {
                return CapabilityEvaluation::not_evaluated(
                    reason,
                    format!("local-circulation: {message}"),
                );
            }
        };
        let (spaces, mut evaluation) = select_objects(context, &rule.selector);
        if spaces.is_empty() {
            return evaluation;
        }
        let refuse = |evaluation: &mut CapabilityEvaluation, (reason, message): Unavailable| {
            for space in &spaces {
                evaluation.push_object_not_evaluated(
                    space.id.clone(),
                    reason.clone(),
                    message.clone(),
                );
            }
        };
        let Some(free_space) = context.services.get::<FreeSpaceServiceHandle>() else {
            refuse(
                &mut evaluation,
                (
                    NotEvaluatedReason::MissingService,
                    "free-space service is not registered".into(),
                ),
            );
            return evaluation;
        };
        let obstacles = match select_obstacles(context, declared.obstacles) {
            Ok(obstacles) => obstacles,
            Err(unavailable) => {
                refuse(&mut evaluation, unavailable);
                return evaluation;
            }
        };
        let swept = match Swings::select(context, declared.swings) {
            Ok(swings) => match swings.undecided() {
                None => swings.sure,
                Some(why) => {
                    refuse(
                        &mut evaluation,
                        incomplete(format!("the door swings are not all known: {why}")),
                    );
                    return evaluation;
                }
            },
            Err(unavailable) => {
                refuse(&mut evaluation, unavailable);
                return evaluation;
            }
        };
        let everything: Vec<&Object> = context.project.objects().collect();
        let members = match components(context, &declared, &everything, &mut evaluation) {
            Ok(members) => members,
            Err(unavailable) => {
                refuse(&mut evaluation, unavailable);
                return evaluation;
            }
        };
        let others = |selector: Option<&Selector>, what: &str| {
            selector.map_or_else(
                || Ok(Others::default()),
                |selector| Others::select(context, &declared, selector, &everything, what),
            )
        };
        let (partners, exempt) = match (
            others(declared.partners, "partner"),
            others(
                declared.exempt.map(|(selector, _)| selector),
                "exempting object",
            ),
        ) {
            (Ok(partners), Ok(exempt)) => (partners, exempt),
            (Err(unavailable), _) | (_, Err(unavailable)) => {
                refuse(&mut evaluation, unavailable);
                return evaluation;
            }
        };
        let index = declared.access.index(context);
        let judge = Judge {
            context,
            rule,
            declared: &declared,
            free_space,
            index: &index,
            obstacles: &obstacles,
            swept: &swept,
            everything: &everything,
            members: &members,
            partners: &partners,
            exempt: &exempt,
        };
        for space in &spaces {
            judge.space(&space.id, &mut evaluation);
        }
        evaluation
    }
}

/// Every obstacle the selection picks, or every object without one.
fn select_obstacles(
    context: &RuleContext<'_>,
    selector: Option<&Selector>,
) -> Result<Vec<ObjectId>, Unavailable> {
    let Some(selector) = selector else {
        return Ok(context
            .project
            .objects()
            .map(|object| object.id.clone())
            .collect());
    };
    let (objects, outcomes) = select_objects(context, selector);
    if let Some(outcome) = outcomes.not_evaluated_outcomes().first() {
        return Err((
            NotEvaluatedReason::IncompleteEvidence,
            format!("the obstacle selection is undecided: {}", outcome.message()),
        ));
    }
    Ok(objects.iter().map(|object| object.id.clone()).collect())
}

/// The selected components of each space. A component whose selection or
/// spaces are undecided is not evaluated itself.
fn components(
    context: &RuleContext<'_>,
    declared: &Declaration<'_>,
    spaces: &[&Object],
    evaluation: &mut CapabilityEvaluation,
) -> Result<BTreeMap<ObjectId, Vec<ObjectId>>, Unavailable> {
    let (picked, outcomes) = select_objects(context, declared.components);
    for outcome in outcomes.not_evaluated_outcomes() {
        match outcome.object_id() {
            Some(object) => evaluation.push_object_not_evaluated(
                object.clone(),
                outcome.reason().clone(),
                format!(
                    "whether it is a component is undecided: {}",
                    outcome.message()
                ),
            ),
            None => {
                return Err((
                    outcome.reason().clone(),
                    format!(
                        "the component selection is undecided: {}",
                        outcome.message()
                    ),
                ));
            }
        }
    }
    let mut members: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    for component in picked {
        match declared.spaces.related(context, &component.id, spaces) {
            Ok((reached, _)) => {
                for space in reached {
                    members.entry(space).or_default().push(component.id.clone());
                }
            }
            Err((reason, message)) => evaluation.push_object_not_evaluated(
                component.id.clone(),
                reason,
                format!("its spaces cannot be read: {message}"),
            ),
        }
    }
    Ok(members)
}

/// Objects of another selection (partners, exempting objects) by space:
/// surely selected ones, and those the selection cannot decide. Objects
/// whose spaces cannot be read may stand anywhere.
#[derive(Default)]
struct Others {
    sure: BTreeMap<ObjectId, Vec<ObjectId>>,
    maybe: BTreeMap<ObjectId, Vec<ObjectId>>,
    anywhere: Vec<String>,
}

impl Others {
    fn select(
        context: &RuleContext<'_>,
        declared: &Declaration<'_>,
        selector: &Selector,
        everything: &[&Object],
        what: &str,
    ) -> Result<Self, Unavailable> {
        let (picked, outcomes) = select_objects(context, selector);
        let mut objects: Vec<(ObjectId, bool)> = picked
            .iter()
            .map(|object| (object.id.clone(), true))
            .collect();
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => objects.push((object.clone(), false)),
                None => {
                    return Err((
                        outcome.reason().clone(),
                        format!("the {what} selection is undecided: {}", outcome.message()),
                    ));
                }
            }
        }
        let mut others = Self::default();
        for (object, sure) in objects {
            match declared.spaces.related(context, &object, everything) {
                Ok((reached, _)) => {
                    let into = if sure {
                        &mut others.sure
                    } else {
                        &mut others.maybe
                    };
                    for space in reached {
                        into.entry(space).or_default().push(object.clone());
                    }
                }
                Err((_, message)) => others.anywhere.push(format!(
                    "the spaces of {what} {object} cannot be read: {message}"
                )),
            }
        }
        Ok(others)
    }

    /// The sure and possible objects of `spaces`.
    fn of(&self, spaces: &[ObjectId]) -> (Vec<ObjectId>, Vec<ObjectId>) {
        let collect = |map: &BTreeMap<ObjectId, Vec<ObjectId>>| {
            let mut out: Vec<ObjectId> = spaces
                .iter()
                .filter_map(|space| map.get(space))
                .flatten()
                .cloned()
                .collect();
            out.sort();
            out.dedup();
            out
        };
        (collect(&self.sure), collect(&self.maybe))
    }
}

fn error(error: &FreeSpaceError) -> Unavailable {
    let reason = match error {
        FreeSpaceError::MissingGeometry(_) | FreeSpaceError::Unavailable(_) => {
            NotEvaluatedReason::BackendUnavailable
        }
        FreeSpaceError::IncompleteClearanceEvidence => NotEvaluatedReason::IncompleteEvidence,
        _ => NotEvaluatedReason::InvalidEvidence,
    };
    (reason, error.to_string())
}

fn incomplete(message: String) -> Unavailable {
    (NotEvaluatedReason::IncompleteEvidence, message)
}

/// How one object stands: findings, or reasons it is not evaluated.
#[derive(Default)]
struct Verdicts {
    findings: Vec<(String, Vec<Evidence>, Vec<ObjectId>)>,
    unknown: Vec<Unavailable>,
}

impl Verdicts {
    fn push_into(
        self,
        rule: &CompiledRule,
        object: &ObjectId,
        evaluation: &mut CapabilityEvaluation,
    ) {
        if !self.findings.is_empty() {
            for (message, evidence, related) in self.findings {
                evaluation.push_finding(finding(rule, object, message, evidence, related));
            }
            return;
        }
        let Some((reason, _)) = self.unknown.first().cloned() else {
            return;
        };
        let messages: Vec<String> = self
            .unknown
            .into_iter()
            .map(|(_, message)| message)
            .collect();
        evaluation.push_object_not_evaluated(object.clone(), reason, messages.join("; "));
    }
}

struct Judge<'a> {
    context: &'a RuleContext<'a>,
    rule: &'a CompiledRule,
    declared: &'a Declaration<'a>,
    free_space: &'a FreeSpaceServiceHandle,
    index: &'a AccessIndex,
    obstacles: &'a [ObjectId],
    swept: &'a [SweptDoor],
    everything: &'a [&'a Object],
    members: &'a BTreeMap<ObjectId, Vec<ObjectId>>,
    partners: &'a Others,
    exempt: &'a Others,
}

/// The entrances of one space.
struct Doors {
    /// Surely entrances, with their relationship evidence.
    sure: Vec<(ObjectId, Vec<Evidence>)>,
    /// Every entrance, sure or possible.
    all: Vec<ObjectId>,
}

impl Judge<'_> {
    #[allow(clippy::too_many_lines)]
    fn space(&self, space: &ObjectId, evaluation: &mut CapabilityEvaluation) {
        let (merged, merge_evidence) = match &self.declared.merge {
            Some(path) => match path.related(self.context, space, self.everything) {
                Ok(merged) => merged,
                Err((reason, message)) => {
                    evaluation.push_object_not_evaluated(
                        space.clone(),
                        reason,
                        format!("the spaces merged with {space} cannot be read: {message}"),
                    );
                    return;
                }
            },
            None => (Vec::new(), Vec::new()),
        };
        let mut merged: Vec<ObjectId> = merged.into_iter().filter(|id| id != space).collect();
        merged.sort();
        merged.dedup();
        let area: Vec<ObjectId> = std::iter::once(space.clone())
            .chain(merged.iter().cloned())
            .collect();
        let mut components: Vec<ObjectId> = area
            .iter()
            .filter_map(|space| self.members.get(space))
            .flatten()
            .cloned()
            .collect();
        components.sort();
        components.dedup();
        // Nothing to reach and no ends to check: the space has no path to
        // judge.
        if components.is_empty() && self.declared.end.is_none() {
            return;
        }
        let mut doors = Doors {
            sure: Vec::new(),
            all: Vec::new(),
        };
        for part in &area {
            let entrances = self.index.entrances(part, AccessType::Any);
            doors
                .all
                .extend(entrances.sure.iter().map(|(id, _)| id.clone()));
            doors
                .all
                .extend(entrances.maybe.iter().map(|(id, _)| id.clone()));
            for (door, cited) in entrances.sure {
                if !doors.sure.iter().any(|(known, _)| *known == door) {
                    doors.sure.push((door, cited));
                }
            }
        }
        doors.all.sort();
        doors.all.dedup();
        doors.sure.sort_by(|a, b| a.0.cmp(&b.0));
        let (partners, maybe_partners) = self.partners.of(&area);
        let obstacles: Vec<ObjectId> = self
            .obstacles
            .iter()
            .filter(|object| !area.contains(object))
            .cloned()
            .collect();
        let mut subjects = components.clone();
        subjects.extend(partners.iter().cloned());
        subjects.extend(maybe_partners.iter().cloned());
        subjects.retain(|subject| !area.contains(subject));
        let map = CirculationRequest::try_new(
            space.clone(),
            doors.all.clone(),
            subjects,
            obstacles.clone(),
            self.declared.width,
            self.declared.height,
            self.declared.tolerance,
        )
        .and_then(|request| request.with_swept_doors(self.swept.to_vec()))
        .and_then(|request| request.with_merged_scopes(merged.clone()))
        .and_then(|request| match self.declared.band_from {
            Some(from) => request.with_band_from(from),
            None => Ok(request),
        })
        .and_then(|request| self.free_space.map_circulation(&request));
        let map = match map {
            Ok(map) => map,
            Err(failure) => {
                let (reason, message) = error(&failure);
                let message = format!("the circulation of {space} is not mapped: {message}");
                evaluation.push_object_not_evaluated(
                    space.clone(),
                    reason.clone(),
                    message.clone(),
                );
                for component in &components {
                    evaluation.push_object_not_evaluated(
                        component.clone(),
                        reason.clone(),
                        message.clone(),
                    );
                }
                return;
            }
        };
        let context = Context {
            judge: self,
            space,
            merged: &merged,
            merge_evidence: &merge_evidence,
            map: &map,
            doors: &doors,
            obstacles: &obstacles,
            area: &area,
        };
        for component in &components {
            let verdicts = match self.declared.mode {
                Mode::Touch => context.touch(component),
                Mode::Link => context.link(component, &components),
                Mode::LinkSets => context.link_sets(component, &partners, &maybe_partners),
            };
            verdicts.push_into(self.rule, component, evaluation);
        }
        context.ends().push_into(self.rule, space, evaluation);
    }
}

/// One mapped space.
struct Context<'a> {
    judge: &'a Judge<'a>,
    space: &'a ObjectId,
    /// The spaces mapped with it, and why.
    merged: &'a [ObjectId],
    merge_evidence: &'a [Evidence],
    map: &'a CirculationMap,
    doors: &'a Doors,
    obstacles: &'a [ObjectId],
    /// The space and the merged spaces.
    area: &'a [ObjectId],
}

impl Context<'_> {
    fn describe_width(&self) -> String {
        metres(self.judge.declared.width)
    }

    /// The pieces near `subject`, proven, and the possible pieces that may
    /// be near it.
    fn near(&self, subject: &ObjectId) -> (Vec<usize>, Vec<usize>) {
        self.map.contact(subject).map_or_else(
            || (Vec::new(), Vec::new()),
            |contact| {
                (
                    contact.reached().iter().map(|(piece, _)| *piece).collect(),
                    contact.possible().to_vec(),
                )
            },
        )
    }

    fn node_of(&self, subject: &ObjectId, piece: usize) -> Option<usize> {
        self.map.contact(subject).and_then(|contact| {
            contact
                .reached()
                .iter()
                .find(|(reached, _)| *reached == piece)
                .and_then(|(_, node)| *node)
        })
    }

    /// Whether an entrance reaches `component`.
    fn touch(&self, component: &ObjectId) -> Verdicts {
        let mut verdicts = Verdicts::default();
        let (pieces, possible) = self.near(component);
        let reaching: Vec<(&ObjectId, &Vec<Evidence>, usize)> = self
            .doors
            .sure
            .iter()
            .filter_map(|(door, evidence)| {
                let (doors, _) = self.near(door);
                pieces
                    .iter()
                    .find(|piece| doors.contains(piece))
                    .map(|piece| (door, evidence, *piece))
            })
            .collect();
        if reaching.is_empty() {
            let apart = self.doors.all.iter().all(|door| {
                let (_, doors) = self.near(door);
                doors.iter().all(|piece| !possible.contains(piece))
            });
            if apart {
                let mut evidence = vec![self.map.evidence().clone()];
                evidence.extend(self.merge_evidence.iter().cloned());
                let mut related = vec![self.space.clone()];
                for part in self.area {
                    let (cited_doors, cited) = self.judge.index.cited(part);
                    evidence.extend(cited);
                    related.extend(cited_doors);
                }
                related.extend(self.merged.iter().cloned());
                related.sort();
                related.dedup();
                verdicts.findings.push((
                    format!(
                        "no entrance of {} reaches it on a path {} wide",
                        self.space,
                        self.describe_width()
                    ),
                    evidence,
                    related,
                ));
            } else {
                verdicts.unknown.push(incomplete(format!(
                    "whether a path {} wide from an entrance of {} reaches it is not proven \
                     either way",
                    self.describe_width(),
                    self.space
                )));
            }
            return verdicts;
        }
        if let Some(passing) = &self.judge.declared.passing {
            for (door, cited, piece) in reaching {
                self.passing(passing, door, cited, component, piece, &mut verdicts);
            }
        }
        verdicts
    }

    /// Whether the path from `door` to `component` in `piece` has its
    /// passing spaces.
    fn passing(
        &self,
        passing: &PassingSpaces,
        door: &ObjectId,
        cited: &[Evidence],
        component: &ObjectId,
        piece: usize,
        verdicts: &mut Verdicts,
    ) {
        let (Some(from), Some(to)) = (self.node_of(door, piece), self.node_of(component, piece))
        else {
            let why = self
                .map
                .unmapped_reason(piece)
                .unwrap_or("the piece has no skeleton node near it");
            verdicts.unknown.push(incomplete(format!(
                "the path from {door} is not traced for its passing spaces: {why}"
            )));
            return;
        };
        let Some(nodes) = trace(self.map, from, to) else {
            verdicts.unknown.push(incomplete(format!(
                "the skeleton does not join {door} to it, so its passing spaces are not traced"
            )));
            return;
        };
        let points: Vec<[f64; 3]> = nodes
            .iter()
            .map(|&node| self.map.nodes()[node].point())
            .collect();
        // Skeleton nodes lie a sample spacing apart; searching a passing
        // space per segment that short would take a search per node.
        let points = simplify(&points, SIMPLIFY_WIDTHS * self.judge.declared.width);
        match passing_spaces::judge_path(
            passing,
            self.judge.free_space,
            (self.space, self.merged),
            (self.obstacles, self.judge.swept),
            &points,
            &format!("the path from {door}"),
        ) {
            Spacing::Met => {}
            Spacing::Missed(message, mut evidence) => {
                evidence.push(self.map.evidence().clone());
                evidence.extend(cited.iter().cloned());
                verdicts
                    .findings
                    .push((message, evidence, vec![self.space.clone(), door.clone()]));
            }
            Spacing::Unknown(reason, message) => verdicts.unknown.push((reason, message)),
        }
    }

    /// Whether `component` is linked with one of the partners: a partner
    /// sharing a piece with it passes; a finding needs its possible pieces
    /// apart from every partner's, sure or possible, and every partner
    /// placed.
    fn link_sets(
        &self,
        component: &ObjectId,
        partners: &[ObjectId],
        maybe_partners: &[ObjectId],
    ) -> Verdicts {
        let mut verdicts = Verdicts::default();
        let (pieces, possible) = self.near(component);
        let linked = partners.iter().any(|partner| {
            partner != component && {
                let (theirs, _) = self.near(partner);
                pieces.iter().any(|piece| theirs.contains(piece))
            }
        });
        if linked {
            return verdicts;
        }
        let apart = partners
            .iter()
            .chain(maybe_partners)
            .filter(|partner| *partner != component)
            .all(|partner| {
                let (_, theirs) = self.near(partner);
                possible.iter().all(|piece| !theirs.contains(piece))
            });
        let anywhere = &self.judge.partners.anywhere;
        if apart && anywhere.is_empty() {
            let mut evidence = vec![self.map.evidence().clone()];
            evidence.extend(self.merge_evidence.iter().cloned());
            let mut related = vec![self.space.clone()];
            related.extend(partners.iter().chain(maybe_partners).cloned());
            related.extend(self.merged.iter().cloned());
            verdicts.findings.push((
                format!(
                    "no path {} wide in {} links it with a partner{}",
                    self.describe_width(),
                    self.space,
                    if partners.is_empty() && maybe_partners.is_empty() {
                        " (the space has none)"
                    } else {
                        ""
                    }
                ),
                evidence,
                related,
            ));
        } else {
            let mut why = format!(
                "whether a path {} wide links it with a partner is not proven either way",
                self.describe_width()
            );
            if !anywhere.is_empty() {
                why.push_str(": ");
                why.push_str(&anywhere.join("; "));
            }
            verdicts.unknown.push(incomplete(why));
        }
        verdicts
    }

    /// Whether `component` is linked with every other component.
    fn link(&self, component: &ObjectId, components: &[ObjectId]) -> Verdicts {
        let mut verdicts = Verdicts::default();
        let (pieces, possible) = self.near(component);
        for other in components.iter().filter(|other| *other != component) {
            let (theirs, their_possible) = self.near(other);
            if pieces.iter().any(|piece| theirs.contains(piece)) {
                continue;
            }
            if possible.iter().all(|piece| !their_possible.contains(piece)) {
                verdicts.findings.push((
                    format!(
                        "no path {} wide in {} links it with {other}",
                        self.describe_width(),
                        self.space
                    ),
                    vec![self.map.evidence().clone()],
                    vec![self.space.clone(), other.clone()],
                ));
            } else {
                verdicts.unknown.push(incomplete(format!(
                    "whether a path {} wide links it with {other} is not proven either way",
                    self.describe_width()
                )));
            }
        }
        verdicts
    }

    /// The free areas at the ends of the paths the entrances reach.
    fn ends(&self) -> Verdicts {
        let mut verdicts = Verdicts::default();
        let Some(area) = &self.judge.declared.end else {
            return verdicts;
        };
        let reached: BTreeSet<usize> = self
            .doors
            .sure
            .iter()
            .flat_map(|(door, _)| self.near(door).0)
            .collect();
        let mut missing = Vec::new();
        let mut proofs = vec![self.map.evidence().clone()];
        for &piece in &reached {
            if let Some(why) = self.map.unmapped_reason(piece) {
                verdicts.unknown.push(incomplete(format!(
                    "where the paths of {} end is unknown: {why}",
                    self.space
                )));
                continue;
            }
            for (node, at) in self.map.nodes().iter().enumerate() {
                if at.piece() != piece || at.kind() != CirculationNodeKind::End {
                    continue;
                }
                match self.end(area, node) {
                    Ok(End::Free | End::Exempt) => {}
                    Ok(End::Missing(proof)) => {
                        let [x, y, _] = at.point();
                        missing.push(format!("({}, {})", metres(x), metres(y)));
                        proofs.push(proof);
                    }
                    Err(unavailable) => verdicts.unknown.push(unavailable),
                }
            }
        }
        if !missing.is_empty() {
            let (cited_doors, cited) = self.judge.index.cited(self.space);
            proofs.extend(cited);
            verdicts.findings.push((
                format!(
                    "no free area {} by {} lies within {} of the path end{} at {}",
                    metres(area.width),
                    metres(area.length),
                    metres(area.reach),
                    if missing.len() == 1 { "" } else { "s" },
                    missing.join(", ")
                ),
                proofs,
                cited_doors,
            ));
        }
        verdicts
    }

    /// Whether the end `node` has its free area or is exempt.
    fn end(&self, area: &EndArea, node: usize) -> Result<End, Unavailable> {
        let declared = self.judge.declared;
        let at = &self.map.nodes()[node];
        // Exemptions: surely, surely not, or undecided.
        let mut exempt = Exemption::No;
        if let Some(narrow) = declared.narrow_end {
            let half = at.half_width();
            exempt = exempt.or(if 2.0 * half.upper_metres() < narrow {
                Exemption::Yes
            } else if 2.0 * half.lower_metres() >= narrow {
                Exemption::No
            } else {
                Exemption::Maybe
            });
        }
        if let Some(short) = declared.short_end {
            let (lower, upper) = branch(self.map, node);
            exempt = exempt.or(if upper < short {
                Exemption::Yes
            } else if lower >= short {
                Exemption::No
            } else {
                Exemption::Maybe
            });
        }
        if exempt == Exemption::Yes {
            return Ok(End::Exempt);
        }
        let [x, y, z] = at.point();
        let (dx, dy) = heading(self.map, node)
            .ok_or_else(|| incomplete("a path end has no direction".into()))?;
        let outcome = self
            .search(area, [x, y, z], [dx, dy])
            .map_err(|failure| error(&failure))?;
        if matches!(outcome, PlacementOutcome::NoPlacement(_)) {
            exempt = exempt.or(self.near_exempt(node));
            if exempt == Exemption::Yes {
                return Ok(End::Exempt);
            }
        }
        match (outcome, exempt) {
            (PlacementOutcome::Found(_), _) => Ok(End::Free),
            (PlacementOutcome::NoPlacement(proof), Exemption::No) => {
                Ok(End::Missing(proof.evidence().clone()))
            }
            (PlacementOutcome::NoPlacement(_), _) => Err(incomplete(format!(
                "the path end at ({}, {}) has no free area, and whether it is short or \
                 narrow enough to need none is not proven either way",
                metres(x),
                metres(y)
            ))),
        }
    }

    /// Whether the end `node` lies within `end_exempt_reach_metres` of an
    /// exempting object's footprint. The end's true position lies within
    /// two sample spacings of the node, so the distance is measured from a
    /// square that wide around it: surely near when even its farthest point
    /// is, surely not when the square is farther.
    fn near_exempt(&self, node: usize) -> Exemption {
        let Some((_, reach)) = self.judge.declared.exempt else {
            return Exemption::No;
        };
        let exempt = self.judge.exempt;
        let (sure, maybe) = exempt.of(self.area);
        if sure.is_empty() && maybe.is_empty() {
            return if exempt.anywhere.is_empty() {
                Exemption::No
            } else {
                Exemption::Maybe
            };
        }
        let Some(proximity) = self.judge.context.services.get::<ProximityServiceHandle>() else {
            return Exemption::Maybe;
        };
        let [x, y, _] = self.map.nodes()[node].point();
        let slack = 2.0 * self.map.spacing_metres();
        let Ok(square) = ConvexPlanRegion::try_new(vec![
            [x - slack, y - slack],
            [x + slack, y - slack],
            [x + slack, y + slack],
            [x - slack, y + slack],
        ]) else {
            return Exemption::Maybe;
        };
        // From the square to the footprint, the end is at most the square's
        // diagonal farther.
        let diagonal = 2.0 * std::f64::consts::SQRT_2 * slack;
        let mut verdict = if exempt.anywhere.is_empty() {
            Exemption::No
        } else {
            Exemption::Maybe
        };
        for (object, surely) in sure
            .iter()
            .map(|object| (object, true))
            .chain(maybe.iter().map(|object| (object, false)))
        {
            let request = RegionDistanceRequest::new(square.clone(), object.clone());
            let here = match proximity.measure_region_distance(&request) {
                Ok(measured) if measured.interval_metres().1 + diagonal <= reach && surely => {
                    Exemption::Yes
                }
                Ok(measured) if measured.interval_metres().0 > reach => Exemption::No,
                _ => Exemption::Maybe,
            };
            verdict = verdict.or(here);
        }
        verdict
    }

    fn search(
        &self,
        area: &EndArea,
        point: [f64; 3],
        [dx, dy]: [f64; 2],
    ) -> Result<PlacementOutcome, FreeSpaceError> {
        let forward = MetricDirection::try_new([dx, dy, 0.0])?;
        let right = MetricDirection::try_new([dy, -dx, 0.0])?;
        let up = MetricDirection::try_new([0.0, 0.0, 1.0])?;
        let origin = MetricPoint::try_new(self.space.clone(), point)
            .map_err(|failure| FreeSpaceError::Unavailable(failure.to_string()))?;
        let anchor = MetricFrame::try_new(origin, right, forward, up)?;
        let offsets = FrameOffsetPlacement::new(
            anchor.clone(),
            SignedDistanceInterval::try_new(-area.reach, area.reach)?,
            SignedDistanceInterval::try_new(-area.reach, area.reach)?,
            SignedDistanceInterval::try_new(-FLOOR_MARGIN, FLOOR_MARGIN)?,
        );
        let shape = PlacementShape::Box {
            shape: BoxClearance::try_new(area.width, area.length, self.judge.declared.height)?,
            orientation: PlacementOrientation::Fixed(anchor),
        };
        let mut request = PlacementRequest::new_in_domain(
            self.space.clone(),
            shape,
            self.obstacles.to_vec(),
            PlacementDomain::FrameOffsets(offsets),
        )?
        .with_swept_doors(self.judge.swept.to_vec())?
        .with_merged_scopes(self.merged.to_vec())?;
        if let Some(from) = self.judge.declared.band_from {
            request = request.with_band(ElevationBand::try_new(from, self.judge.declared.height)?);
        }
        self.judge.free_space.find_placement(&request)
    }
}

/// How a path end stands.
enum End {
    Free,
    Exempt,
    /// No free area near it: the proof.
    Missing(Evidence),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Exemption {
    Yes,
    No,
    Maybe,
}

impl Exemption {
    /// Exempt when either exemption applies.
    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            (Self::No, Self::No) => Self::No,
            _ => Self::Maybe,
        }
    }
}

/// Bounds on the length of the branch ending at `end`: along the skeleton
/// to the first node that is no mere path node, plus the free half width
/// at the end (the end node stands that far from the wall it ends at),
/// widened by two sample spacings each way for the approximate node
/// positions.
fn branch(map: &CirculationMap, end: usize) -> (f64, f64) {
    let nodes = map.nodes();
    let plan = |a: usize, b: usize| {
        let ([ax, ay, _], [bx, by, _]) = (nodes[a].point(), nodes[b].point());
        (bx - ax).hypot(by - ay)
    };
    let mut length = 0.0;
    let (mut previous, mut here) = (end, end);
    let mut visited = BTreeSet::from([end]);
    loop {
        let next = map
            .neighbours(here)
            .into_iter()
            .find(|node| *node != previous && !visited.contains(node));
        let Some(next) = next else { break };
        length += plan(here, next);
        visited.insert(next);
        previous = here;
        here = next;
        if nodes[here].kind() != CirculationNodeKind::Path {
            break;
        }
    }
    let slack = 2.0 * map.spacing_metres();
    let half = nodes[end].half_width();
    (
        (length - slack + half.lower_metres()).max(0.0),
        length + slack + half.upper_metres(),
    )
}

/// The direction the path runs into the end `end`: from the first node
/// back along its branch at least the end's half width (and four sample
/// spacings) away, so that neighbouring nodes a rounding apart do not set
/// it.
fn heading(map: &CirculationMap, end: usize) -> Option<(f64, f64)> {
    let nodes = map.nodes();
    let [x, y, _] = nodes[end].point();
    let wanted = nodes[end]
        .half_width()
        .upper_metres()
        .max(4.0 * map.spacing_metres());
    let (mut previous, mut here) = (end, end);
    let mut visited = BTreeSet::from([end]);
    loop {
        let next = map
            .neighbours(here)
            .into_iter()
            .find(|node| *node != previous && !visited.contains(node))?;
        visited.insert(next);
        previous = here;
        here = next;
        let [hx, hy, _] = nodes[here].point();
        let (dx, dy) = (x - hx, y - hy);
        let length = dx.hypot(dy);
        let stop = length >= wanted || nodes[here].kind() != CirculationNodeKind::Path;
        if stop && length > 0.0 && length.is_finite() {
            return Some((dx / length, dy / length));
        }
        if stop {
            return None;
        }
    }
}

/// The polyline `points` with every point dropped that lies within
/// `tolerance` in plan of the chord kept around it (Douglas and Peucker).
fn simplify(points: &[[f64; 3]], tolerance: f64) -> Vec<[f64; 3]> {
    fn keep(points: &[[f64; 3]], tolerance: f64, out: &mut Vec<[f64; 3]>) {
        let (first, last) = (points[0], points[points.len() - 1]);
        let (dx, dy) = (last[0] - first[0], last[1] - first[1]);
        let length = dx.hypot(dy);
        let distance = |p: &[f64; 3]| {
            if length > 0.0 {
                ((p[0] - first[0]) * dy - (p[1] - first[1]) * dx).abs() / length
            } else {
                (p[0] - first[0]).hypot(p[1] - first[1])
            }
        };
        let farthest = (1..points.len() - 1)
            .map(|i| (distance(&points[i]), i))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        match farthest {
            Some((far, i)) if far > tolerance => {
                keep(&points[..=i], tolerance, out);
                keep(&points[i..], tolerance, out);
            }
            _ => out.push(last),
        }
    }
    let Some(first) = points.first() else {
        return Vec::new();
    };
    let mut out = vec![*first];
    if points.len() > 1 {
        keep(points, tolerance, &mut out);
    }
    out
}

/// The skeleton nodes from `from` to `to`, fewest edges first.
fn trace(map: &CirculationMap, from: usize, to: usize) -> Option<Vec<usize>> {
    let mut previous: BTreeMap<usize, usize> = BTreeMap::new();
    let mut queue = VecDeque::from([from]);
    let mut seen = BTreeSet::from([from]);
    while let Some(here) = queue.pop_front() {
        if here == to {
            let mut path = vec![to];
            let mut at = to;
            while let Some(&before) = previous.get(&at) {
                path.push(before);
                at = before;
            }
            path.reverse();
            return Some(path);
        }
        for next in map.neighbours(here) {
            if seen.insert(next) {
                previous.insert(next, here);
                queue.push_back(next);
            }
        }
    }
    None
}
