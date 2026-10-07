//! The free-floor search both free-floor capabilities run: obstacles, door
//! swings, elevation band, merged spaces and the path from the entrances,
//! answering for each space whether the shape is found, proven absent or
//! open. The capabilities' templates (`free_floor/template.rs`) judge the
//! answer through the measured list `free_floor_fit`; `free_placements`
//! lists it for expressions.

mod measured;
#[cfg(feature = "parity-reference")]
pub(crate) mod reference;
pub(crate) mod template;

pub(crate) use measured::{FitMeasures, PlacementMeasures};

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    BoxClearance, CylinderClearance, ElevationBand, EntranceReach, FreeSpaceError,
    FreeSpaceServiceHandle, NotEvaluatedReason, ParameterDescriptor, ParameterType,
    PlacementDomain, PlacementOrientation, PlacementOutcome, PlacementRequest, PlacementShape,
    RuleContext, SupportedPlacement, SweptDoor,
};
use axioval_ir::contract::ParameterValue;
use axioval_ir::measured::MeasuredSelection;
use axioval_ir::{Evidence, Object, ObjectId};

use crate::door_swing::Swings;
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

/// The shape a free-floor rule declares, and its height, in the
/// capability's order and words.
///
/// # Errors
///
/// An invalid declaration, worded as the capability refused it.
pub(crate) fn declared_shape(
    rule: &axioval_engine::CompiledRule,
    rectangle: bool,
) -> Result<(PlacementShape, f64), Unavailable> {
    let number = |name: &str| match rule.parameters.get(name)? {
        ParameterValue::Number { value } => Some(*value),
        _ => None,
    };
    if !rectangle {
        let (Some(diameter), Some(height)) = (number("diameter_metres"), number("height_metres"))
        else {
            return Err(invalid("free-floor circle dimensions are invalid"));
        };
        let shape = CylinderClearance::try_new(diameter / 2.0, height)
            .map_err(|_| invalid("free-floor circle dimensions must be positive and finite"))?;
        return Ok((PlacementShape::Cylinder(shape), height));
    }
    let (Some(width), Some(length), Some(height)) = (
        number("width_metres"),
        number("length_metres"),
        number("height_metres"),
    ) else {
        return Err(invalid("free-floor rectangle dimensions are invalid"));
    };
    let shape = BoxClearance::try_new(width, length, height)
        .map_err(|_| invalid("free-floor rectangle dimensions must be positive and finite"))?;
    let orientation = match rule.parameters.get("orientation") {
        Some(ParameterValue::String { value }) if value == "any" => PlacementOrientation::Any,
        Some(ParameterValue::String { value }) => {
            return Err(invalid(format!(
                "free-floor rectangle orientation `{value}` is not supported; \
                 only `any` has a frame source"
            )));
        }
        _ => {
            return Err(invalid(
                "free-floor rectangle needs an explicit `orientation`",
            ));
        }
    };
    Ok((PlacementShape::Box { shape, orientation }, height))
}

/// What a free-floor rule declares besides its shape and its selections.
pub(crate) struct Declared {
    /// The band obstacles count in; the shape's height without one.
    pub(crate) band: Option<ElevationBand>,
    /// The path from each space to the spaces searched with it.
    pub(crate) merge: Option<Traversal>,
    /// The width and tolerance of the path from the entrances that must
    /// reach the shape.
    pub(crate) entrance: Option<(f64, f64)>,
}

impl Declared {
    /// Reads the options; `height` is the shape's, the band's default top.
    ///
    /// # Errors
    ///
    /// An invalid declaration, worded as the capability refused it.
    pub(crate) fn parse(parameters: &Parameters<'_>, height: f64) -> Result<Self, Unavailable> {
        let band = band(
            parameters.number("band_from_metres")?,
            parameters.number("band_to_metres")?,
            height,
        )?;
        let merge = match parameters.strings("merge_path")? {
            Some(path) => Some(Traversal::path(path)?),
            None => None,
        };
        let access = AccessDeclaration::parse(parameters)?;
        let width = parameters.number("entrance_path_width")?;
        let tolerance = parameters.number("entrance_tolerance_metres")?;
        let entrance = entrance(access.is_some(), width, tolerance)?;
        // Read as the capability read them, so a parameter of another type
        // is refused in its order.
        parameters.selector("obstacles")?;
        parameters.selector("subtract_door_swings")?;
        Ok(Self {
            band,
            merge,
            entrance,
        })
    }
}

/// The band obstacles count in, `None` where neither end is declared.
fn band(
    from: Option<f64>,
    to: Option<f64>,
    height: f64,
) -> Result<Option<ElevationBand>, Unavailable> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    let (from, to) = (from.unwrap_or(0.0), to.unwrap_or(height));
    ElevationBand::try_new(from, to).map(Some).map_err(|_| {
        invalid(format!(
            "the elevation band from {from} m to {to} m above the floor is invalid: \
             it must start at or above the floor and end above its start"
        ))
    })
}

/// The entrance path's width and tolerance, where declared.
fn entrance(
    access: bool,
    width: Option<f64>,
    tolerance: Option<f64>,
) -> Result<Option<(f64, f64)>, Unavailable> {
    match (access, width) {
        (true, Some(width)) if width.is_finite() && width > 0.0 => {
            let tolerance = tolerance.unwrap_or(DEFAULT_ENTRANCE_TOLERANCE);
            if !(tolerance.is_finite() && tolerance >= 0.0) {
                return Err(invalid("`entrance_tolerance_metres` must not be negative"));
            }
            Ok(Some((width, tolerance)))
        }
        (_, Some(_)) => Err(invalid(
            "`entrance_path_width` must be positive and needs `access_path` to find \
             the entrances",
        )),
        (true, None) => Err(invalid(
            "`access_path` finds the entrances `entrance_path_width` reaches from; \
             declare both",
        )),
        (false, None) if tolerance.is_some() => Err(invalid(
            "`entrance_tolerance_metres` needs `entrance_path_width`",
        )),
        (false, None) => Ok(None),
    }
}

/// Checks the rule parameters `free_floor_fit` names, as the rule states
/// them, in the capabilities' order and words: the shape (`shape`, the
/// list's own word, says which), then the options.
///
/// # Errors
///
/// An invalid declaration.
pub(crate) fn check_arguments(
    stated: &BTreeMap<String, ParameterValue>,
) -> Result<(), Unavailable> {
    let rectangle = matches!(
        stated.get("shape"),
        Some(ParameterValue::String { value }) if value == "rectangle"
    );
    let rule = crate::light_area::synthesised(stated.clone());
    let (_, height) = declared_shape(&rule, rectangle)?;
    Declared::parse(&Parameters(&rule), height).map(|_| ())
}

/// One search's setting: the shape, its options, the obstacle candidates
/// and the swept doors, read once per run.
pub(crate) struct Search {
    pub(crate) shape: PlacementShape,
    pub(crate) declared: Declared,
    pub(crate) obstacles: Obstacles,
    pub(crate) swings: Swings,
    /// The entrances of every space, where a path from them must reach the
    /// shape.
    pub(crate) index: Option<AccessIndex>,
}

/// The obstacle candidates, split by how sure the selection is.
pub(crate) enum Obstacles {
    /// Every object but the searched spaces.
    Everything(Vec<ObjectId>),
    Selected {
        sure: BTreeSet<ObjectId>,
        maybe: BTreeSet<ObjectId>,
    },
}

impl Obstacles {
    /// The obstacles a measured value's argument bound, every object
    /// without one.
    pub(crate) fn of(context: &RuleContext<'_>, selection: Option<&MeasuredSelection>) -> Self {
        match selection {
            None => Self::Everything(
                context
                    .project
                    .objects()
                    .map(|object| object.id.clone())
                    .collect(),
            ),
            Some(selection) => Self::Selected {
                sure: selection.matched.clone(),
                maybe: selection.undecided.clone(),
            },
        }
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

/// What the search answers for one space.
pub(crate) enum Placement {
    /// A placement on exact evidence.
    Found,
    /// No placement can be: a proof standing whatever the selections
    /// cannot decide.
    Absent(Absent),
    /// Neither, and why.
    Open(Unavailable),
}

/// A proven absence: the merged spaces and entrances it relates, its
/// evidence, and whether the shape fits but no path from an entrance
/// reaches it.
pub(crate) struct Absent {
    pub(crate) related: Vec<ObjectId>,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) unreached: bool,
}

/// One search request: its obstacles, swept doors and entrances (`None`
/// when the shape need not be reached).
struct Ask {
    candidates: Vec<ObjectId>,
    swept: Vec<SweptDoor>,
    entrances: Option<Vec<ObjectId>>,
}

impl Search {
    /// Searches `space`'s free floor for the shape.
    ///
    /// Obstacles the selection cannot decide are sent as candidates, so a
    /// witness stands; a proof of absence with them is asked again without
    /// them, and stands only if it holds there too. Door swings follow the
    /// same rule, and a door whose leaves are unknown can only spoil a
    /// witness: its swing may cover it. A path from the entrances needs a
    /// sure entrance for a witness and every entrance for a proof.
    pub(crate) fn place(&self, context: &RuleContext<'_>, space: &Object) -> Placement {
        let Some(service) = context.services.get::<FreeSpaceServiceHandle>() else {
            return Placement::Open((
                NotEvaluatedReason::MissingService,
                "free-space service is not registered".into(),
            ));
        };
        match self.judge(context, service, space) {
            Ok(None) => Placement::Found,
            Ok(Some(absent)) => Placement::Absent(absent),
            Err(open) => Placement::Open(open),
        }
    }

    fn ask(
        &self,
        service: &FreeSpaceServiceHandle,
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
        if let Some(band) = self.declared.band {
            request = request.with_band(band);
        }
        if let (Some(entrances), Some((width, tolerance))) = (ask.entrances, self.declared.entrance)
        {
            request = request.with_entrance_reach(
                EntranceReach::try_new(entrances, width, tolerance).map_err(|e| error(&e))?,
            );
        }
        if !ask.swept.is_empty() {
            request = request.with_swept_doors(ask.swept).map_err(|e| error(&e))?;
        }
        service.find_placement(&request).map_err(|e| error(&e))
    }

    /// The entrances of `spaces`: sure ones with their evidence, and every
    /// one, sure or possible. `None` without an entrance path.
    fn entrances(
        &self,
        spaces: &[&ObjectId],
    ) -> Option<(Vec<ObjectId>, Vec<ObjectId>, Vec<Evidence>)> {
        let index = self.index.as_ref()?;
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

    /// A proven absence, or `None` for a witness.
    #[allow(clippy::too_many_lines)]
    fn judge(
        &self,
        context: &RuleContext<'_>,
        service: &FreeSpaceServiceHandle,
        space: &Object,
    ) -> Result<Option<Absent>, Unavailable> {
        let (merged, mut evidence) = match &self.declared.merge {
            Some(path) => {
                let everything: Vec<&Object> = context.project.objects().collect();
                path.related(context, &space.id, &everything)?
            }
            None => (Vec::new(), Vec::new()),
        };
        let spaces: BTreeSet<&ObjectId> = std::iter::once(&space.id).chain(&merged).collect();
        let (sure, maybe) = self.obstacles.around(&spaces);
        let swings = &self.swings;
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
        let proof = match self.ask(service, space, &merged, first)? {
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
            match self.ask(service, space, &merged, again)? {
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
        let mut unreached = false;
        if let Some((_, all, cited)) = &entrances {
            related.extend(all.iter().cloned());
            evidence.extend(cited.iter().cloned());
            // Worded apart: the shape fits, but no path reaches it.
            let plain = Ask {
                candidates: sure,
                swept: swings.sure.clone(),
                entrances: None,
            };
            if let Ok(PlacementOutcome::Found(found)) = self.ask(service, space, &merged, plain) {
                evidence.push(found.evidence().clone());
                unreached = true;
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
