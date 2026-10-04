//! Shared policy of the free-floor capabilities: obstacles, door swings,
//! elevation band, merged spaces, the path from the entrances and the
//! three-valued placement judgement.

use std::collections::BTreeSet;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ElevationBand, EntranceReach, FreeSpaceError,
    FreeSpaceServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PlacementDomain, PlacementOutcome, PlacementRequest, PlacementShape, RuleContext,
    SupportedPlacement, SweptDoor,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, Severity};

use crate::door_swing::Swings;
use crate::level_spacing::metres;
use crate::selection::select_objects;
use crate::space_access::{AccessDeclaration, AccessIndex, AccessType};
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// The optional parameters both free-floor capabilities take.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("obstacles", ParameterType::Selector),
        ParameterDescriptor::optional("band_from_metres", ParameterType::Number),
        ParameterDescriptor::optional("band_to_metres", ParameterType::Number),
        ParameterDescriptor::optional("merge_path", ParameterType::StringList),
        ParameterDescriptor::optional("subtract_door_swings", ParameterType::Selector),
        ParameterDescriptor::optional("entrance_path_width", ParameterType::Number),
        ParameterDescriptor::optional("entrance_tolerance_metres", ParameterType::Number),
        ParameterDescriptor::optional("access_path", ParameterType::StringList),
        ParameterDescriptor::optional("door_selector", ParameterType::Selector),
        ParameterDescriptor::optional("opening_selector", ParameterType::Selector),
        ParameterDescriptor::optional("space_selector", ParameterType::Selector),
    ]
}

/// The default distance an entrance may be from the path beyond its half
/// width, as in `local-circulation`.
const DEFAULT_ENTRANCE_TOLERANCE: f64 = 0.05;

/// A path from the space's entrances that must reach the shape.
struct EntrancePath<'a> {
    access: AccessDeclaration<'a>,
    width: f64,
    tolerance: f64,
}

/// What a free-floor rule declares besides its shape.
pub(crate) struct Options<'a> {
    /// The obstacle selection; every other object without one.
    obstacles: Option<&'a Selector>,
    /// The band obstacles count in; the shape's height without one.
    band: Option<ElevationBand>,
    /// The path from each space to the spaces searched with it.
    merge: Option<Traversal>,
    /// The doors whose swings are obstacles.
    swings: Option<&'a Selector>,
    /// The path from the entrances the shape must be reached by.
    entrance: Option<EntrancePath<'a>>,
}

impl<'a> Options<'a> {
    /// Reads the options; `height` is the shape's, the band's default top.
    pub(crate) fn parse(rule: &'a CompiledRule, height: f64) -> Result<Self, Unavailable> {
        let parameters = Parameters(rule);
        let from = parameters.number("band_from_metres")?;
        let to = parameters.number("band_to_metres")?;
        let band = if from.is_none() && to.is_none() {
            None
        } else {
            let (from, to) = (from.unwrap_or(0.0), to.unwrap_or(height));
            Some(ElevationBand::try_new(from, to).map_err(|_| {
                invalid(format!(
                    "the elevation band from {from} m to {to} m above the floor is invalid: \
                     it must start at or above the floor and end above its start"
                ))
            })?)
        };
        let merge = match parameters.strings("merge_path")? {
            Some(path) => Some(Traversal::path(path)?),
            None => None,
        };
        let access = AccessDeclaration::parse(&parameters)?;
        let width = parameters.number("entrance_path_width")?;
        let tolerance = parameters.number("entrance_tolerance_metres")?;
        let entrance = match (access, width) {
            (Some(access), Some(width)) if width.is_finite() && width > 0.0 => {
                let tolerance = tolerance.unwrap_or(DEFAULT_ENTRANCE_TOLERANCE);
                if !(tolerance.is_finite() && tolerance >= 0.0) {
                    return Err(invalid("`entrance_tolerance_metres` must not be negative"));
                }
                Some(EntrancePath {
                    access,
                    width,
                    tolerance,
                })
            }
            (_, Some(_)) => {
                return Err(invalid(
                    "`entrance_path_width` must be positive and needs `access_path` to find \
                     the entrances",
                ));
            }
            (Some(_), None) => {
                return Err(invalid(
                    "`access_path` finds the entrances `entrance_path_width` reaches from; \
                     declare both",
                ));
            }
            (None, None) if tolerance.is_some() => {
                return Err(invalid(
                    "`entrance_tolerance_metres` needs `entrance_path_width`",
                ));
            }
            (None, None) => None,
        };
        Ok(Self {
            obstacles: parameters.selector("obstacles")?,
            band,
            merge,
            swings: parameters.selector("subtract_door_swings")?,
            entrance,
        })
    }
}

/// The obstacle candidates, split by how sure the selection is.
enum Obstacles {
    /// Every object but the searched spaces.
    Everything(Vec<ObjectId>),
    Selected {
        sure: BTreeSet<ObjectId>,
        maybe: BTreeSet<ObjectId>,
    },
}

impl Obstacles {
    fn select(context: &RuleContext<'_>, selector: Option<&Selector>) -> Result<Self, Unavailable> {
        let Some(selector) = selector else {
            return Ok(Self::Everything(
                context
                    .project
                    .objects()
                    .map(|object| object.id.clone())
                    .collect(),
            ));
        };
        let (objects, outcomes) = select_objects(context, selector);
        let mut maybe = BTreeSet::new();
        for outcome in outcomes.not_evaluated_outcomes() {
            match outcome.object_id() {
                Some(object) => {
                    maybe.insert(object.clone());
                }
                None => {
                    return Err((
                        outcome.reason().clone(),
                        format!("obstacle selection is undecided: {}", outcome.message()),
                    ));
                }
            }
        }
        Ok(Self::Selected {
            sure: objects.iter().map(|object| object.id.clone()).collect(),
            maybe,
        })
    }

    /// Sure obstacles and possible ones for a search of `spaces`, which are
    /// floor, never obstacles.
    fn around(&self, spaces: &BTreeSet<&ObjectId>) -> (Vec<ObjectId>, Vec<ObjectId>) {
        let keep = |id: &&ObjectId| !spaces.contains(*id);
        match self {
            Self::Everything(all) => (all.iter().filter(keep).cloned().collect(), Vec::new()),
            Self::Selected { sure, maybe } => (
                sure.iter().filter(keep).cloned().collect(),
                maybe.iter().filter(keep).cloned().collect(),
            ),
        }
    }
}

/// Judges each selected space: a proven absence of any placement is a
/// finding with `message`, a witness passes, anything else is not evaluated.
///
/// Obstacles the selection cannot decide are sent as candidates, so a
/// witness stands; a proof of absence with them is asked again without
/// them, and stands only if it holds there too. Door swings follow the
/// same rule, and a door whose leaves are unknown can only spoil a
/// witness: its swing may cover it.
pub(crate) fn evaluate(
    context: &RuleContext<'_>,
    rule: &CompiledRule,
    selected: &[&Object],
    mut evaluation: CapabilityEvaluation,
    shape: &PlacementShape,
    options: &Options<'_>,
    message: &str,
) -> CapabilityEvaluation {
    let Some(service) = context.services.get::<FreeSpaceServiceHandle>() else {
        return unavailable(
            selected,
            &NotEvaluatedReason::MissingService,
            "free-space service is not registered",
            evaluation,
        );
    };
    let obstacles = match Obstacles::select(context, options.obstacles) {
        Ok(obstacles) => obstacles,
        Err((reason, message)) => return unavailable(selected, &reason, &message, evaluation),
    };
    let swings = match Swings::select(context, options.swings) {
        Ok(swings) => swings,
        Err((reason, message)) => return unavailable(selected, &reason, &message, evaluation),
    };
    let everything: Vec<&Object> = context.project.objects().collect();
    let index = options
        .entrance
        .as_ref()
        .map(|entrance| entrance.access.index(context));
    let ground = Ground {
        context,
        service,
        obstacles: &obstacles,
        swings: &swings,
        everything: &everything,
        index: index.as_ref(),
        shape,
        options,
    };
    for space in selected {
        match ground.judge(space) {
            Ok(None) => {}
            Ok(Some(absent)) => evaluation.push_finding(Finding {
                explanation: None,
                id: None,
                decision: None,
                rule_id: rule.id.clone(),
                scope: axioval_ir::Scope::Object(space.id.clone()),
                severity: severity(rule),
                related: absent.related,
                message: match absent.unreached {
                    Some(width) => format!(
                        "{message}: the shape fits only where no path {} wide from an \
                         entrance reaches it",
                        metres(width)
                    ),
                    None => message.into(),
                },
                evidence: absent.evidence,
                location: None,
                categories: Vec::new(),
            }),
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(space.id.clone(), reason, message);
            }
        }
    }
    evaluation
}

/// A proven absence: the merged spaces and entrances it relates, its
/// evidence, and the path width when the shape fits but is not reached.
struct Absent {
    related: Vec<ObjectId>,
    evidence: Vec<Evidence>,
    unreached: Option<f64>,
}

/// A proven absence, or `None` for a witness.
type Judged = Result<Option<Absent>, Unavailable>;

/// Everything a space is judged against.
struct Ground<'a> {
    context: &'a RuleContext<'a>,
    service: &'a FreeSpaceServiceHandle,
    obstacles: &'a Obstacles,
    swings: &'a Swings,
    everything: &'a [&'a Object],
    index: Option<&'a AccessIndex>,
    shape: &'a PlacementShape,
    options: &'a Options<'a>,
}

/// One search: its obstacles, swept doors and entrances (`None` when the
/// shape need not be reached).
struct Ask {
    candidates: Vec<ObjectId>,
    swept: Vec<SweptDoor>,
    entrances: Option<Vec<ObjectId>>,
}

impl Ground<'_> {
    fn ask(
        &self,
        space: &Object,
        merged: &[ObjectId],
        ask: Ask,
    ) -> Result<PlacementOutcome, Unavailable> {
        // A merged search spans several floors, so no single support holds
        // its base; the search is still bounded by the spaces' footprints.
        let domain = if merged.is_empty() {
            PlacementDomain::Supported(
                SupportedPlacement::try_new(space.id.clone(), 0.0).map_err(|e| error(&e))?,
            )
        } else {
            PlacementDomain::Unconstrained
        };
        let mut request = PlacementRequest::new_in_domain(
            space.id.clone(),
            self.shape.clone(),
            ask.candidates,
            domain,
        )
        .map_err(|e| error(&e))?
        .with_merged_scopes(merged.to_vec())
        .map_err(|e| error(&e))?;
        if let Some(band) = self.options.band {
            request = request.with_band(band);
        }
        if let (Some(entrances), Some(path)) = (ask.entrances, &self.options.entrance) {
            request = request.with_entrance_reach(
                EntranceReach::try_new(entrances, path.width, path.tolerance)
                    .map_err(|e| error(&e))?,
            );
        }
        if !ask.swept.is_empty() {
            request = request.with_swept_doors(ask.swept).map_err(|e| error(&e))?;
        }
        self.service.find_placement(&request).map_err(|e| error(&e))
    }

    /// The entrances of `spaces`: sure ones with their evidence, and every
    /// one, sure or possible. `None` without an entrance path.
    fn entrances(
        &self,
        spaces: &[&ObjectId],
    ) -> Option<(Vec<ObjectId>, Vec<ObjectId>, Vec<Evidence>)> {
        let index = self.index?;
        let mut sure = Vec::new();
        let mut all = Vec::new();
        let mut evidence = Vec::new();
        for space in spaces {
            let found = index.entrances(space, AccessType::Any);
            for (door, cited) in found.sure {
                sure.push(door.clone());
                all.push(door);
                evidence.extend(cited);
            }
            all.extend(found.maybe.into_iter().map(|(door, _)| door));
        }
        for list in [&mut sure, &mut all] {
            list.sort();
            list.dedup();
        }
        Some((sure, all, evidence))
    }

    #[allow(clippy::too_many_lines)]
    fn judge(&self, space: &Object) -> Judged {
        let (merged, mut evidence) = match &self.options.merge {
            Some(path) => path.related(self.context, &space.id, self.everything)?,
            None => (Vec::new(), Vec::new()),
        };
        let spaces: BTreeSet<&ObjectId> = std::iter::once(&space.id).chain(&merged).collect();
        let (sure, maybe) = self.obstacles.around(&spaces);
        let swings = self.swings;
        let entrances = self.entrances(&spaces.iter().copied().collect::<Vec<_>>());
        let mut candidates = sure.clone();
        candidates.extend(maybe.iter().cloned());
        let mut swept = swings.sure.clone();
        swept.extend(swings.maybe.iter().cloned());
        let first = Ask {
            candidates,
            swept,
            entrances: entrances.as_ref().map(|(sure, _, _)| sure.clone()),
        };
        let proof = match self.ask(space, &merged, first)? {
            PlacementOutcome::Found(_) if swings.unknown.is_empty() => return Ok(None),
            PlacementOutcome::Found(_) => {
                let names: Vec<String> = swings
                    .unknown
                    .iter()
                    .map(|(door, why)| format!("{door} ({why})"))
                    .collect();
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the shape fits, but the swing of a door whose leaves are unknown may \
                         cover every fit: {}",
                        names.join(", ")
                    ),
                ));
            }
            PlacementOutcome::NoPlacement(proof) => proof,
        };
        let possible_entrances = entrances
            .as_ref()
            .is_some_and(|(sure, all, _)| sure.len() < all.len());
        let proof = if maybe.is_empty() && swings.maybe.is_empty() && !possible_entrances {
            proof
        } else {
            let again = Ask {
                candidates: sure.clone(),
                swept: swings.sure.clone(),
                entrances: entrances.as_ref().map(|(_, all, _)| all.clone()),
            };
            match self.ask(space, &merged, again)? {
                PlacementOutcome::NoPlacement(proof) => proof,
                PlacementOutcome::Found(_) => {
                    let mut names: Vec<String> = maybe
                        .iter()
                        .map(ToString::to_string)
                        .chain(
                            swings
                                .maybe
                                .iter()
                                .map(|door| format!("the swing of {}", door.door())),
                        )
                        .collect();
                    if let Some((sure, all, _)) = &entrances {
                        names.extend(
                            all.iter()
                                .filter(|door| !sure.contains(door))
                                .map(|door| format!("whether {door} is an entrance")),
                        );
                    }
                    return Err((
                        NotEvaluatedReason::IncompleteEvidence,
                        format!(
                            "the shape fits only if what the selections cannot decide goes \
                             its way: {}",
                            names.join(", ")
                        ),
                    ));
                }
            }
        };
        evidence.push(proof.evidence().clone());
        evidence.extend(swings.evidence.iter().cloned());
        let mut related = merged.clone();
        let mut unreached = None;
        if let (Some((_, all, cited)), Some(path)) = (&entrances, &self.options.entrance) {
            related.extend(all.iter().cloned());
            evidence.extend(cited.iter().cloned());
            // Worded apart: the shape fits, but no path reaches it.
            let plain = Ask {
                candidates: sure,
                swept: swings.sure.clone(),
                entrances: None,
            };
            if let Ok(PlacementOutcome::Found(found)) = self.ask(space, &merged, plain) {
                evidence.push(found.evidence().clone());
                unreached = Some(path.width);
            }
        }
        related.sort();
        related.dedup();
        Ok(Some(Absent {
            related,
            evidence,
            unreached,
        }))
    }
}

pub(crate) fn severity(rule: &CompiledRule) -> Severity {
    match rule.severity {
        axioval_ir::contract::Severity::Error => Severity::Error,
        axioval_ir::contract::Severity::Warning => Severity::Warning,
        axioval_ir::contract::Severity::Info => Severity::Info,
    }
}

/// Every selected space not evaluated for one reason.
pub(crate) fn unavailable(
    selected: &[&Object],
    reason: &NotEvaluatedReason,
    message: &str,
    mut evaluation: CapabilityEvaluation,
) -> CapabilityEvaluation {
    for object in selected {
        evaluation.push_object_not_evaluated(object.id.clone(), reason.clone(), message);
    }
    evaluation
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
