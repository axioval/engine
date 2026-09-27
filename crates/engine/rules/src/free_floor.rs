//! Shared policy of the free-floor capabilities: obstacles, elevation band,
//! merged spaces and the three-valued placement judgement.

use std::collections::BTreeSet;

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, ElevationBand, FreeSpaceError, FreeSpaceServiceHandle,
    NotEvaluatedReason, ParameterDescriptor, ParameterType, PlacementDomain, PlacementOutcome,
    PlacementRequest, PlacementShape, RuleContext, SupportedPlacement,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Finding, Object, ObjectId, Severity};

use crate::selection::select_objects;
use crate::support::{Parameters, Traversal, Unavailable, invalid};

/// The optional parameters both free-floor capabilities take.
pub(crate) fn parameters() -> Vec<ParameterDescriptor> {
    vec![
        ParameterDescriptor::optional("obstacles", ParameterType::Selector),
        ParameterDescriptor::optional("band_from_metres", ParameterType::Number),
        ParameterDescriptor::optional("band_to_metres", ParameterType::Number),
        ParameterDescriptor::optional("merge_path", ParameterType::StringList),
    ]
}

/// What a free-floor rule declares besides its shape.
pub(crate) struct Options<'a> {
    /// The obstacle selection; every other object without one.
    obstacles: Option<&'a Selector>,
    /// The band obstacles count in; the shape's height without one.
    band: Option<ElevationBand>,
    /// The path from each space to the spaces searched with it.
    merge: Option<Traversal<'a>>,
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
        Ok(Self {
            obstacles: parameters.selector("obstacles")?,
            band,
            merge,
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
/// them, and stands only if it holds there too.
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
    let everything: Vec<&Object> = context.project.objects().collect();
    for space in selected {
        match judge(
            context,
            service,
            &obstacles,
            &everything,
            space,
            shape,
            options,
        ) {
            Ok(None) => {}
            Ok(Some((merged, evidence))) => evaluation.push_finding(Finding {
                rule_id: rule.id.clone(),
                scope: axioval_ir::Scope::Object(space.id.clone()),
                severity: severity(rule),
                related: merged,
                message: message.into(),
                evidence,
            }),
            Err((reason, message)) => {
                evaluation.push_object_not_evaluated(space.id.clone(), reason, message);
            }
        }
    }
    evaluation
}

/// The merged spaces and evidence of a proven absence, or `None` for a
/// witness.
type Judged = Result<Option<(Vec<ObjectId>, Vec<Evidence>)>, Unavailable>;

fn judge(
    context: &RuleContext<'_>,
    service: &FreeSpaceServiceHandle,
    obstacles: &Obstacles,
    everything: &[&Object],
    space: &Object,
    shape: &PlacementShape,
    options: &Options<'_>,
) -> Judged {
    let (merged, mut evidence) = match &options.merge {
        Some(path) => path.related(context, &space.id, everything)?,
        None => (Vec::new(), Vec::new()),
    };
    let spaces: BTreeSet<&ObjectId> = std::iter::once(&space.id).chain(&merged).collect();
    let (sure, maybe) = obstacles.around(&spaces);
    let ask = |candidates: Vec<ObjectId>| -> Result<PlacementOutcome, Unavailable> {
        // A merged search spans several floors, so no single support holds
        // its base; the search is still bounded by the spaces' footprints.
        let domain = if merged.is_empty() {
            PlacementDomain::Supported(
                SupportedPlacement::try_new(space.id.clone(), 0.0).map_err(|e| error(&e))?,
            )
        } else {
            PlacementDomain::Unconstrained
        };
        let mut request =
            PlacementRequest::new_in_domain(space.id.clone(), shape.clone(), candidates, domain)
                .map_err(|e| error(&e))?
                .with_merged_scopes(merged.clone())
                .map_err(|e| error(&e))?;
        if let Some(band) = options.band {
            request = request.with_band(band);
        }
        service.find_placement(&request).map_err(|e| error(&e))
    };
    let mut candidates = sure.clone();
    candidates.extend(maybe.iter().cloned());
    let proof = match ask(candidates)? {
        PlacementOutcome::Found(_) => return Ok(None),
        PlacementOutcome::NoPlacement(proof) => proof,
    };
    let proof = if maybe.is_empty() {
        proof
    } else {
        match ask(sure)? {
            PlacementOutcome::NoPlacement(proof) => proof,
            PlacementOutcome::Found(_) => {
                let names: Vec<String> = maybe.iter().map(ToString::to_string).collect();
                return Err((
                    NotEvaluatedReason::IncompleteEvidence,
                    format!(
                        "the shape fits only if objects the obstacle selection cannot decide \
                         are not obstacles: {}",
                        names.join(", ")
                    ),
                ));
            }
        }
    };
    evidence.push(proof.evidence().clone());
    Ok(Some((merged, evidence)))
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
