//! Accessible route: route spaces must connect start points to destinations
//! for a mobility profile.
//!
//! The rule's selector picks the destinations (accessible rooms, say);
//! `start_selector` the start points (entrances). `route_selector` picks the
//! spaces a route may cross, `portal_selector` the doors and openings it may
//! pass, and `lift_selector`, `ramp_selector` and `stair_selector` the
//! vertical connectors it may climb. A start or destination the portal
//! selector picks is an entrance; any other is a walkable surface.
//!
//! One walkability snapshot is taken for the profile: a body `width_metres`
//! wide under `clear_height_metres` of headroom. The capability then judges
//! each passage on top of the snapshot's width bounds
//! ([`PassageAdmission`]): a portal must be at least `door_width_metres`
//! wide, a ramp `ramp_width_metres` and a stair `stair_width_metres`; a
//! stair is refused outright while `forbid_stairs` holds (the default). A
//! width is read from `clear_width_property` where it is stated, a portal's
//! from the geometry's bound otherwise, and a ramp's or stair's from its
//! measured flight or runs (the walking-surface service `stair-geometry`
//! and `ramp-geometry` measure with); a stated portal width also goes into
//! the request, so the geometry can admit the body through a door.
//!
//! `obstruction_depth_metres` tolerates obstacles within that distance of
//! a route space's boundary (a skirting), and `surface_gap_metres` joins
//! route spaces at most that far apart; both go into the request. A block
//! inside one route space names where it lies and whether the space is too
//! narrow there, obstructed, or too low.
//!
//! With `subtract_door_swings`, the sectors the selected doors' leaves sweep
//! are obstacles on the surfaces they stand on: a body walks through its
//! own door's swing, never past another's. A door whose swing may count but
//! is unknown leaves every destination not evaluated.
//!
//! With `passing_width_metres`, `passing_length_metres` and
//! `passing_spacing_metres`, a proven route must also offer a free box that
//! size at most every `passing_spacing_metres` along it (see
//! [`crate::passing_spaces`]).
//!
//! Three-valued throughout: a destination some start reaches definitely
//! passes, one every start is proven cut off from is a finding relating the
//! elements that block it, and anything else is not evaluated. Objects whose
//! selection is undecided can only add routes or remove blocks, so they
//! leave a verdict standing only where they cannot change it.

use std::collections::{BTreeMap, BTreeSet};

use axioval_engine::{
    CapabilityEvaluation, CompiledRule, LengthInterval, MeasuredInterval, NotEvaluatedReason,
    ParameterDescriptor, ParameterType, PassageAdmission, RuleCapability, RuleContext, SlopedRun,
    StretchLimit, SweptDoor, TreadFlightRequest, VerifiedWalkablePassage, VerticalConnector,
    VerticalConnectorKind, WalkabilityError, WalkabilityRegionId, WalkabilityRequest,
    WalkabilityRouteOutcome, WalkabilityServiceHandle, WalkabilitySnapshot, WalkableStretch,
    WalkingSurfaceServiceHandle,
};
use axioval_ir::contract::Selector;
use axioval_ir::{Evidence, Object, ObjectId, PropertyValue, QuantityDimension};

use crate::door_swing::Swings;
use crate::passing_spaces::{self, Ground, PassingSpaces, Spacing};
use crate::plan_area::shown;
use crate::selection::{Selection, select_objects, selector_matches};
use crate::support::{Parameters, PropertyRef, Unavailable, display, finding, invalid, resolve};

/// Requires a route for a mobility profile from the start points to each
/// selected destination.
pub struct AccessibleRoute;

struct Declaration<'a> {
    route: &'a Selector,
    starts: &'a Selector,
    portals: Option<&'a Selector>,
    connectors: [(VerticalConnectorKind, Option<&'a Selector>); 3],
    obstacles: Option<&'a Selector>,
    swings: Option<&'a Selector>,
    width: f64,
    clear_height: Option<f64>,
    door_width: Option<f64>,
    ramp_width: Option<f64>,
    stair_width: Option<f64>,
    forbid_stairs: bool,
    clear_width: Option<PropertyRef<'a>>,
    passing: Option<PassingSpaces>,
    obstruction_depth: f64,
    surface_gap: f64,
}

fn length(parameters: &Parameters<'_>, name: &str) -> Result<Option<f64>, Unavailable> {
    match parameters.number(name)? {
        Some(value) if value <= 0.0 => Err(invalid(format!("`{name}` must be positive"))),
        other => Ok(other),
    }
}

/// A tolerance: zero when absent, never negative.
fn tolerance(parameters: &Parameters<'_>, name: &str) -> Result<f64, Unavailable> {
    match parameters.number(name)? {
        Some(value) if !value.is_finite() || value < 0.0 => {
            Err(invalid(format!("`{name}` must not be negative")))
        }
        other => Ok(other.unwrap_or(0.0)),
    }
}

fn declaration(rule: &CompiledRule) -> Result<Declaration<'_>, Unavailable> {
    let parameters = Parameters(rule);
    let clear_height = length(&parameters, "clear_height_metres")?;
    Ok(Declaration {
        route: parameters.required_selector("route_selector")?,
        starts: parameters.required_selector("start_selector")?,
        portals: parameters.selector("portal_selector")?,
        connectors: [
            (
                VerticalConnectorKind::Lift,
                parameters.selector("lift_selector")?,
            ),
            (
                VerticalConnectorKind::Ramp,
                parameters.selector("ramp_selector")?,
            ),
            (
                VerticalConnectorKind::Stair,
                parameters.selector("stair_selector")?,
            ),
        ],
        obstacles: parameters.selector("obstacle_selector")?,
        swings: parameters.selector("subtract_door_swings")?,
        width: length(&parameters, "width_metres")?
            .ok_or_else(|| invalid("parameter `width_metres` is required"))?,
        clear_height,
        door_width: length(&parameters, "door_width_metres")?,
        ramp_width: length(&parameters, "ramp_width_metres")?,
        stair_width: length(&parameters, "stair_width_metres")?,
        forbid_stairs: parameters.boolean("forbid_stairs")?.unwrap_or(true),
        clear_width: parameters.property("clear_width_property")?,
        passing: PassingSpaces::parse(&parameters, clear_height)?,
        obstruction_depth: tolerance(&parameters, "obstruction_depth_metres")?,
        surface_gap: tolerance(&parameters, "surface_gap_metres")?,
    })
}

impl RuleCapability for AccessibleRoute {
    fn id(&self) -> &'static str {
        "axioval:capability.accessible-route"
    }

    fn parameters(&self) -> Vec<ParameterDescriptor> {
        let mut parameters = vec![
            ParameterDescriptor::required("route_selector", ParameterType::Selector),
            ParameterDescriptor::required("start_selector", ParameterType::Selector),
            ParameterDescriptor::optional("portal_selector", ParameterType::Selector),
            ParameterDescriptor::optional("lift_selector", ParameterType::Selector),
            ParameterDescriptor::optional("ramp_selector", ParameterType::Selector),
            ParameterDescriptor::optional("stair_selector", ParameterType::Selector),
            ParameterDescriptor::optional("obstacle_selector", ParameterType::Selector),
            ParameterDescriptor::optional("subtract_door_swings", ParameterType::Selector),
            ParameterDescriptor::required("width_metres", ParameterType::Number),
            ParameterDescriptor::optional("clear_height_metres", ParameterType::Number),
            ParameterDescriptor::optional("door_width_metres", ParameterType::Number),
            ParameterDescriptor::optional("ramp_width_metres", ParameterType::Number),
            ParameterDescriptor::optional("stair_width_metres", ParameterType::Number),
            ParameterDescriptor::optional("forbid_stairs", ParameterType::Boolean),
            ParameterDescriptor::optional("clear_width_property", ParameterType::PropertyReference),
            ParameterDescriptor::optional("obstruction_depth_metres", ParameterType::Number),
            ParameterDescriptor::optional("surface_gap_metres", ParameterType::Number),
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
                    format!("accessible-route: {message}"),
                );
            }
        };
        let (destinations, mut evaluation) = select_objects(context, &rule.selector);
        let refuse =
            |evaluation: &mut CapabilityEvaluation, reason: NotEvaluatedReason, message: String| {
                for destination in &destinations {
                    evaluation.push_object_not_evaluated(
                        destination.id.clone(),
                        reason.clone(),
                        message.clone(),
                    );
                }
            };
        if destinations.is_empty() {
            return evaluation;
        }
        let Some(service) = context.services.get::<WalkabilityServiceHandle>() else {
            refuse(
                &mut evaluation,
                NotEvaluatedReason::MissingService,
                "walkability service is not registered".into(),
            );
            return evaluation;
        };
        let scene = match Scene::select(context, &declared, &destinations) {
            Ok(scene) => scene,
            Err((reason, message)) => {
                refuse(&mut evaluation, reason, message);
                return evaluation;
            }
        };
        let request = match scene.request(&declared) {
            Ok(request) => request,
            Err((reason, message)) => {
                refuse(&mut evaluation, reason, message);
                return evaluation;
            }
        };
        let snapshot = match service.snapshot(&request) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                refuse(
                    &mut evaluation,
                    error_reason(&error),
                    format!("no walkability snapshot: {error}"),
                );
                return evaluation;
            }
        };
        let judge = Judge {
            context,
            declared: &declared,
            scene: &scene,
            snapshot: &snapshot,
            regions: snapshot
                .regions()
                .iter()
                .map(|region| (region.id(), region.objects()))
                .collect(),
        };
        for destination in destinations {
            match judge.destination(&destination.id) {
                Verdict::Reachable => {}
                Verdict::Blocked(blocked) => {
                    evaluation.push_finding(judge.finding(rule, &destination.id, blocked));
                }
                Verdict::Crowded(missed) => {
                    evaluation.push_finding(judge.crowded(rule, &destination.id, missed));
                }
                Verdict::Undecided(reason, message) => {
                    evaluation.push_object_not_evaluated(destination.id.clone(), reason, message);
                }
            }
        }
        evaluation
    }
}

fn error_reason(error: &WalkabilityError) -> NotEvaluatedReason {
    match error {
        WalkabilityError::Unavailable(_) | WalkabilityError::ObjectUnavailable => {
            NotEvaluatedReason::BackendUnavailable
        }
        WalkabilityError::IncompleteEvidence => NotEvaluatedReason::IncompleteEvidence,
        WalkabilityError::InvalidMinimumWidth
        | WalkabilityError::ConflictingConnector
        | WalkabilityError::InvalidStatedClearWidth => NotEvaluatedReason::InvalidDeclaration,
        _ => NotEvaluatedReason::InvalidEvidence,
    }
}

/// The objects a selector picks, and those it cannot decide with why.
#[derive(Default)]
struct Picked {
    decided: BTreeSet<ObjectId>,
    undecided: BTreeMap<ObjectId, String>,
}

impl Picked {
    fn select(context: &RuleContext<'_>, selector: Option<&Selector>) -> Self {
        let mut picked = Self::default();
        let Some(selector) = selector else {
            return picked;
        };
        for object in context.project.objects() {
            match selector_matches(context, selector, object, &mut Vec::new()) {
                Selection::Match => {
                    picked.decided.insert(object.id.clone());
                }
                Selection::NoMatch => {}
                Selection::NotEvaluated(_, message) => {
                    picked.undecided.insert(object.id.clone(), message);
                }
            }
        }
        picked
    }

    fn all(&self) -> impl Iterator<Item = &ObjectId> {
        self.decided.iter().chain(self.undecided.keys())
    }

    fn contains(&self, id: &ObjectId) -> bool {
        self.decided.contains(id) || self.undecided.contains_key(id)
    }
}

/// A clear width as the source states it.
enum Stated {
    Known(f64, Vec<Evidence>),
    Absent,
    Unknown(String),
}

/// A connector's width as its measured flight or runs show it.
enum Measured {
    Width(MeasuredInterval, Evidence),
    Unknown(String),
}

/// Everything the rule selected, and the widths it read or measured.
struct Scene {
    route: Picked,
    starts: Picked,
    portals: Picked,
    connectors: BTreeMap<ObjectId, (VerticalConnectorKind, Option<String>)>,
    obstacles: Vec<ObjectId>,
    swept: Vec<SweptDoor>,
    destinations: BTreeSet<ObjectId>,
    widths: BTreeMap<ObjectId, Stated>,
    measured: BTreeMap<ObjectId, Measured>,
}

impl Scene {
    fn select(
        context: &RuleContext<'_>,
        declared: &Declaration<'_>,
        destinations: &[&Object],
    ) -> Result<Self, Unavailable> {
        let obstacles = Picked::select(context, declared.obstacles);
        if let Some((object, why)) = obstacles.undecided.iter().next() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("whether {object} is an obstacle is undecided: {why}"),
            ));
        }
        let swings = Swings::select(context, declared.swings)?;
        if let Some(why) = swings.undecided() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                format!("the door swings are not all known: {why}"),
            ));
        }
        let mut connectors = BTreeMap::new();
        for (kind, selector) in declared.connectors {
            let picked = Picked::select(context, selector);
            for (object, undecided) in picked
                .decided
                .into_iter()
                .map(|object| (object, None))
                .chain(
                    picked
                        .undecided
                        .into_iter()
                        .map(|(object, why)| (object, Some(why))),
                )
            {
                if let Some((other, _)) = connectors.insert(object.clone(), (kind, undecided))
                    && other != kind
                {
                    return Err(invalid(format!(
                        "{object} is selected as both a {} and a {}",
                        other.as_str(),
                        kind.as_str()
                    )));
                }
            }
        }
        let mut scene = Self {
            route: Picked::select(context, Some(declared.route)),
            starts: Picked::select(context, Some(declared.starts)),
            portals: Picked::select(context, declared.portals),
            connectors,
            obstacles: obstacles.decided.into_iter().collect(),
            swept: swings.sure,
            destinations: destinations
                .iter()
                .map(|object| object.id.clone())
                .collect(),
            widths: BTreeMap::new(),
            measured: BTreeMap::new(),
        };
        if scene.starts.decided.is_empty() && scene.starts.undecided.is_empty() {
            return Err((
                NotEvaluatedReason::IncompleteEvidence,
                "the start selector selects no start point".into(),
            ));
        }
        if let Some(property) = declared.clear_width {
            let objects: BTreeMap<&ObjectId, &Object> = context
                .project
                .objects()
                .map(|object| (&object.id, object))
                .collect();
            let measured: Vec<ObjectId> = scene
                .portals
                .all()
                .cloned()
                .chain(scene.connectors.keys().cloned())
                .collect();
            for id in measured {
                let stated = match objects.get(&id) {
                    Some(object) => read_width(context, object, property),
                    None => Stated::Unknown(format!("{id} is not in the project")),
                };
                scene.widths.insert(id, stated);
            }
        }
        scene.measure_connectors(context, declared);
        Ok(scene)
    }

    /// Measures each ramp or stair with a minimum and no stated width, as
    /// `ramp-geometry` and `stair-geometry` measure it; only an exact
    /// absence moves on to the geometry.
    fn measure_connectors(&mut self, context: &RuleContext<'_>, declared: &Declaration<'_>) {
        let stairs = context.services.get::<WalkingSurfaceServiceHandle>();
        for (object, (kind, _)) in &self.connectors {
            let required = match kind {
                VerticalConnectorKind::Lift => None,
                VerticalConnectorKind::Ramp => declared.ramp_width,
                VerticalConnectorKind::Stair if declared.forbid_stairs => None,
                VerticalConnectorKind::Stair => declared.stair_width,
            };
            if required.is_none()
                || matches!(
                    self.widths.get(object),
                    Some(Stated::Known(..) | Stated::Unknown(_))
                )
            {
                continue;
            }
            let measured = match stairs {
                Some(stairs) => measure_width(stairs, object, *kind),
                None => Measured::Unknown("the walking-surface service is not registered".into()),
            };
            self.measured.insert(object.clone(), measured);
        }
    }

    fn request(&self, declared: &Declaration<'_>) -> Result<WalkabilityRequest, Unavailable> {
        let entrances: Vec<ObjectId> = self.portals.all().cloned().collect();
        let surfaces: Vec<ObjectId> = self
            .route
            .all()
            .chain(self.starts.all())
            .chain(&self.destinations)
            .filter(|object| !self.portals.contains(object))
            .cloned()
            .collect();
        let band = declared
            .clear_height
            .map(|height| LengthInterval::try_new(0.0, height))
            .transpose()
            .map_err(|error| invalid(format!("clear height: {error}")))?;
        let stated: Vec<(ObjectId, f64)> = self
            .widths
            .iter()
            .filter(|(object, _)| self.portals.contains(object))
            .filter_map(|(object, stated)| match stated {
                Stated::Known(metres, _) => Some((object.clone(), *metres)),
                _ => None,
            })
            .collect();
        WalkabilityRequest::try_new(
            surfaces,
            entrances,
            self.obstacles.clone(),
            declared.width,
            band,
            true,
            false,
        )
        .and_then(|request| {
            request.with_connectors(
                self.connectors
                    .iter()
                    .map(|(object, (kind, _))| VerticalConnector::new(object.clone(), *kind))
                    .collect(),
            )
        })
        .and_then(|request| request.with_stated_clear_widths(stated))
        .and_then(|request| request.with_swept_doors(self.swept.clone()))
        .and_then(|request| request.with_obstruction_depth(declared.obstruction_depth))
        .and_then(|request| request.with_surface_gap(declared.surface_gap))
        .map_err(|error| invalid(format!("walkability request: {error}")))
    }
}

/// A ramp's or stair's width from its measured runs or flight: the
/// narrowest run governs a ramp, and a flight's width needs every tread's
/// sides.
fn measure_width(
    stairs: &WalkingSurfaceServiceHandle,
    object: &ObjectId,
    kind: VerticalConnectorKind,
) -> Measured {
    let measured = match kind {
        VerticalConnectorKind::Stair => stairs
            .measure_tread_flight(&TreadFlightRequest::new(object.clone()))
            .map(|flight| {
                flight.width().map_or_else(
                    || {
                        Measured::Unknown(
                            "a tread fills no rectangle along the walking direction".into(),
                        )
                    },
                    |width| Measured::Width(width, flight.evidence().clone()),
                )
            }),
        VerticalConnectorKind::Ramp => stairs.measure_sloped_runs(object).map(|surface| {
            let widths: Option<Vec<MeasuredInterval>> =
                surface.runs().iter().map(SlopedRun::width).collect();
            let narrowest = widths.and_then(|widths| {
                let lower = widths
                    .iter()
                    .map(MeasuredInterval::lower)
                    .reduce(f64::min)?;
                let upper = widths
                    .iter()
                    .map(MeasuredInterval::upper)
                    .reduce(f64::min)?;
                MeasuredInterval::try_new(lower, upper).ok()
            });
            narrowest.map_or_else(
                || Measured::Unknown("a run fills no rectangle along its slope".into()),
                |width| Measured::Width(width, surface.evidence().clone()),
            )
        }),
        VerticalConnectorKind::Lift => return Measured::Unknown("a lift has no width".into()),
    };
    measured.unwrap_or_else(|error| Measured::Unknown(error.to_string()))
}

/// A clear width read from `property`: a length, stated or absent.
fn read_width(context: &RuleContext<'_>, object: &Object, property: PropertyRef<'_>) -> Stated {
    match resolve(context, object, property) {
        Ok(resolved) => match resolved.value() {
            None => Stated::Absent,
            Some(PropertyValue::Quantity {
                value,
                dimension: QuantityDimension::Length,
            }) if value.is_finite() && *value > 0.0 => Stated::Known(*value, resolved.evidence()),
            Some(other) => Stated::Unknown(format!(
                "{property} of {} is {}, not a positive length",
                object.id,
                display(Some(other))
            )),
        },
        Err((_, message)) => Stated::Unknown(format!(
            "{property} of {} cannot be read: {message}",
            object.id
        )),
    }
}

/// Why one element stops or may stop a route.
struct Judgement {
    admission: PassageAdmission,
    element: Option<ObjectId>,
    reason: Option<String>,
    evidence: Vec<Evidence>,
    stairs_only: bool,
    /// Other objects the block depends on (the obstacles at a stretch).
    others: Vec<ObjectId>,
}

impl Judgement {
    fn admitted() -> Self {
        Self {
            admission: PassageAdmission::Admitted,
            element: None,
            reason: None,
            evidence: Vec::new(),
            stairs_only: false,
            others: Vec::new(),
        }
    }

    fn worst(self, other: Self) -> Self {
        let rank = |admission: PassageAdmission| match admission {
            PassageAdmission::Admitted => 0,
            PassageAdmission::Undecided => 1,
            PassageAdmission::Refused => 2,
        };
        if rank(other.admission) > rank(self.admission) {
            other
        } else {
            self
        }
    }
}

enum Verdict {
    Reachable,
    Blocked(Vec<Judgement>),
    /// Every proven route lacks passing spaces: per start, the message and
    /// evidence.
    Crowded(Vec<(ObjectId, String, Vec<Evidence>)>),
    Undecided(NotEvaluatedReason, String),
}

struct Judge<'a> {
    context: &'a RuleContext<'a>,
    declared: &'a Declaration<'a>,
    scene: &'a Scene,
    snapshot: &'a WalkabilitySnapshot,
    regions: BTreeMap<&'a WalkabilityRegionId, &'a [ObjectId]>,
}

fn metres(value: f64) -> String {
    shown(value, value)
}

impl Judge<'_> {
    /// Whether a route between `from` and `to` may use `passage`.
    fn judge(
        &self,
        passage: &VerifiedWalkablePassage,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Judgement {
        let (a, b) = passage.endpoints();
        let mut judged = Judgement::admitted();
        for object in [a, b]
            .into_iter()
            .flat_map(|region| self.regions.get(region).copied().unwrap_or_default())
        {
            judged = judged.worst(self.transit(object, from, to));
        }
        if let Some(portal) = passage.portal() {
            judged = judged.worst(self.portal(portal, passage));
        }
        if let Some(connector) = passage.connector() {
            judged = judged.worst(self.connector(connector));
        }
        judged
    }

    /// Whether a route may cross `object`'s region.
    fn transit(&self, object: &ObjectId, from: &ObjectId, to: &ObjectId) -> Judgement {
        let scene = self.scene;
        if object == from
            || object == to
            || scene.route.decided.contains(object)
            || scene.portals.decided.contains(object)
        {
            return Judgement::admitted();
        }
        let why = scene
            .route
            .undecided
            .get(object)
            .or_else(|| scene.portals.undecided.get(object));
        match why {
            Some(why) => Judgement {
                admission: PassageAdmission::Undecided,
                element: Some(object.clone()),
                reason: Some(format!(
                    "whether {object} is on the route is undecided: {why}"
                )),
                evidence: Vec::new(),
                stairs_only: false,
                others: Vec::new(),
            },
            None => Judgement {
                admission: PassageAdmission::Refused,
                element: Some(object.clone()),
                reason: Some(format!("{object} is not a route space")),
                evidence: Vec::new(),
                stairs_only: false,
                others: Vec::new(),
            },
        }
    }

    /// Whether a portal is wide enough for `door_width_metres`.
    fn portal(&self, portal: &ObjectId, crossing: &VerifiedWalkablePassage) -> Judgement {
        let Some(required) = self.declared.door_width else {
            return Judgement::admitted();
        };
        let judged = |admission, reason: Option<String>, evidence| Judgement {
            admission,
            element: Some(portal.clone()),
            reason,
            evidence,
            stairs_only: false,
            others: Vec::new(),
        };
        let bound = crossing.clear_width();
        match self.scene.widths.get(portal) {
            Some(Stated::Known(stated, evidence)) if *stated < required => judged(
                PassageAdmission::Refused,
                Some(format!(
                    "{portal} states a clear width of {} m, less than the {} m required",
                    metres(*stated),
                    metres(required)
                )),
                evidence.clone(),
            ),
            _ if bound.upper_metres() < required => judged(
                PassageAdmission::Refused,
                Some(format!(
                    "{portal} is at most {} m wide in the geometry, less than the {} m required",
                    metres(bound.upper_metres()),
                    metres(required)
                )),
                vec![crossing.evidence().clone()],
            ),
            Some(Stated::Known(..)) => Judgement::admitted(),
            _ if bound.lower_metres() >= required => Judgement::admitted(),
            Some(Stated::Unknown(why)) => judged(
                PassageAdmission::Undecided,
                Some(format!("the clear width of {portal} is unknown: {why}")),
                Vec::new(),
            ),
            _ => judged(
                PassageAdmission::Undecided,
                Some(format!(
                    "{portal} states no clear width, and the geometry bounds it only from above"
                )),
                Vec::new(),
            ),
        }
    }

    /// Whether a connector stating no clear width is wide enough by its
    /// measured flight or runs.
    fn measured(&self, object: &ObjectId, kind: VerticalConnectorKind, required: f64) -> Judgement {
        let judged = |admission, reason: String, evidence| Judgement {
            admission,
            element: Some(object.clone()),
            reason: Some(reason),
            evidence,
            stairs_only: false,
            others: Vec::new(),
        };
        let kind = kind.as_str();
        match self.scene.measured.get(object) {
            Some(Measured::Width(width, evidence)) if width.upper() < required => judged(
                PassageAdmission::Refused,
                format!(
                    "{kind} {object} is at most {} m wide in the geometry, less than the {} m \
                     required",
                    metres(width.upper()),
                    metres(required)
                ),
                vec![evidence.clone()],
            ),
            Some(Measured::Width(width, _)) if width.lower() >= required => Judgement::admitted(),
            Some(Measured::Width(width, _)) => judged(
                PassageAdmission::Undecided,
                format!(
                    "{kind} {object} is {} m wide in the geometry, which does not decide the {} m \
                     required",
                    shown(width.lower(), width.upper()),
                    metres(required)
                ),
                Vec::new(),
            ),
            Some(Measured::Unknown(why)) => judged(
                PassageAdmission::Undecided,
                format!(
                    "{kind} {object} states no clear width to compare with {} m, and its width is \
                     not measured: {why}",
                    metres(required)
                ),
                Vec::new(),
            ),
            None => judged(
                PassageAdmission::Undecided,
                format!(
                    "{kind} {object} states no clear width to compare with {} m",
                    metres(required)
                ),
                Vec::new(),
            ),
        }
    }

    /// Whether a connector may be climbed.
    fn connector(&self, connector: &VerticalConnector) -> Judgement {
        let object = connector.object();
        let kind = connector.kind();
        let judged = |admission, reason: String, evidence, stairs_only| Judgement {
            admission,
            element: Some(object.clone()),
            reason: Some(reason),
            evidence,
            stairs_only,
            others: Vec::new(),
        };
        if kind == VerticalConnectorKind::Stair && self.declared.forbid_stairs {
            return judged(
                PassageAdmission::Refused,
                format!("{object} is a stair, which the route may not use"),
                Vec::new(),
                true,
            );
        }
        let required = match kind {
            VerticalConnectorKind::Lift => None,
            VerticalConnectorKind::Ramp => self.declared.ramp_width,
            VerticalConnectorKind::Stair => self.declared.stair_width,
        };
        let mut result = Judgement::admitted();
        if let Some(required) = required {
            result = match self.scene.widths.get(object) {
                Some(Stated::Known(stated, evidence)) if *stated < required => judged(
                    PassageAdmission::Refused,
                    format!(
                        "{} {object} states a clear width of {} m, less than the {} m required",
                        kind.as_str(),
                        metres(*stated),
                        metres(required)
                    ),
                    evidence.clone(),
                    false,
                ),
                Some(Stated::Known(..)) => Judgement::admitted(),
                Some(Stated::Unknown(why)) => judged(
                    PassageAdmission::Undecided,
                    format!("the clear width of {object} is unknown: {why}"),
                    Vec::new(),
                    false,
                ),
                Some(Stated::Absent) | None => self.measured(object, kind, required),
            };
        }
        if let Some((_, Some(why))) = self.scene.connectors.get(object) {
            result = result.worst(judged(
                PassageAdmission::Undecided,
                format!(
                    "whether {object} is a {} is undecided: {why}",
                    kind.as_str()
                ),
                Vec::new(),
                false,
            ));
        }
        result
    }

    /// How an endpoint that is itself a portal stands: its own crossing is
    /// not on the route between its faces, so it is judged here.
    fn endpoint(&self, object: &ObjectId, from: &ObjectId, to: &ObjectId) -> Judgement {
        if !self.scene.portals.contains(object) {
            return Judgement::admitted();
        }
        let crossings: Vec<&VerifiedWalkablePassage> = self
            .snapshot
            .passages()
            .iter()
            .filter(|passage| passage.portal() == Some(object))
            .collect();
        let width = self.declared.width;
        let mut judged = Judgement::admitted();
        for crossing in &crossings {
            let bound = crossing.clear_width();
            let own = if bound.upper_metres() < width {
                Judgement {
                    admission: PassageAdmission::Refused,
                    element: Some(object.clone()),
                    reason: Some(format!(
                        "{object} is at most {} m wide, less than the {} m body",
                        metres(bound.upper_metres()),
                        metres(width)
                    )),
                    evidence: vec![crossing.evidence().clone()],
                    stairs_only: false,
                    others: Vec::new(),
                }
            } else if bound.lower_metres() < width {
                Judgement {
                    admission: PassageAdmission::Undecided,
                    element: Some(object.clone()),
                    reason: Some(format!("the body's passage through {object} is not proven")),
                    evidence: Vec::new(),
                    stairs_only: false,
                    others: Vec::new(),
                }
            } else {
                Judgement::admitted()
            };
            judged = judged.worst(own).worst(self.judge(crossing, from, to));
        }
        if crossings.is_empty() {
            judged = Judgement {
                admission: PassageAdmission::Undecided,
                element: Some(object.clone()),
                reason: Some(format!("{object} has no crossing to pass through")),
                evidence: Vec::new(),
                stairs_only: false,
                others: Vec::new(),
            };
        }
        judged
    }

    #[allow(clippy::too_many_lines)]
    fn destination(&self, destination: &ObjectId) -> Verdict {
        let scene = self.scene;
        let mut blocked: Vec<Judgement> = Vec::new();
        let mut undecided: BTreeSet<String> = BTreeSet::new();
        let mut open = false;
        let mut spacing_reason = NotEvaluatedReason::IncompleteEvidence;
        let mut spacing_open: BTreeSet<String> = BTreeSet::new();
        let mut crowded = Vec::new();
        let starts = scene
            .starts
            .decided
            .iter()
            .map(|start| (start, None))
            .chain(
                scene
                    .starts
                    .undecided
                    .iter()
                    .map(|(start, why)| (start, Some(why))),
            );
        for (start, why) in starts {
            let ends = self
                .endpoint(start, start, destination)
                .worst(self.endpoint(destination, start, destination));
            if ends.admission == PassageAdmission::Refused {
                blocked.push(ends);
                continue;
            }
            let admit = |passage: &VerifiedWalkablePassage| {
                self.judge(passage, start, destination).admission
            };
            let outcome = if start == destination {
                Ok(WalkabilityRouteOutcome::Reachable(Vec::new()))
            } else {
                self.snapshot
                    .route_between_admitting(start, destination, admit)
            };
            match outcome {
                Ok(WalkabilityRouteOutcome::Reachable(_))
                    if why.is_none() && ends.admission == PassageAdmission::Admitted =>
                {
                    match self.spacing(start, destination) {
                        Spacing::Met => return Verdict::Reachable,
                        Spacing::Missed(message, evidence) => {
                            crowded.push((start.clone(), message, evidence));
                        }
                        Spacing::Unknown(reason, message) => {
                            spacing_reason = reason;
                            spacing_open.insert(message);
                        }
                    }
                }
                Ok(
                    WalkabilityRouteOutcome::Reachable(_) | WalkabilityRouteOutcome::Indeterminate,
                ) => {
                    open = true;
                    if let Some(why) = why {
                        undecided.insert(format!("whether {start} is a start is undecided: {why}"));
                    }
                    if let Some(reason) = ends.reason {
                        undecided.insert(reason);
                    }
                    for passage in self.snapshot.passages() {
                        let judged = self.judge(passage, start, destination);
                        if judged.admission == PassageAdmission::Undecided
                            && let Some(reason) = judged.reason
                        {
                            undecided.insert(reason);
                        }
                    }
                }
                Ok(WalkabilityRouteOutcome::Unreachable) => {
                    match self.snapshot.blocking_passages(start, destination, admit) {
                        Ok(passages) => {
                            for passage in passages {
                                blocked.push(self.blocking(passage, start, destination));
                            }
                        }
                        Err(error) => {
                            return Verdict::Undecided(error_reason(&error), error.to_string());
                        }
                    }
                }
                Err(error) => {
                    return Verdict::Undecided(
                        error_reason(&error),
                        format!("no route from {start} to {destination}: {error}"),
                    );
                }
            }
        }
        if open {
            let mut message = format!(
                "no route from a start to {destination} is proven for a body {} m wide, \
                 and none is ruled out",
                metres(self.declared.width)
            );
            undecided.extend(spacing_open);
            if !undecided.is_empty() {
                message.push_str(": ");
                message.push_str(&undecided.into_iter().collect::<Vec<_>>().join("; "));
            }
            return Verdict::Undecided(NotEvaluatedReason::IncompleteEvidence, message);
        }
        if !spacing_open.is_empty() {
            return Verdict::Undecided(
                spacing_reason,
                format!(
                    "a route to {destination} is proven, but not its passing spaces: {}",
                    spacing_open.into_iter().collect::<Vec<_>>().join("; ")
                ),
            );
        }
        if !crowded.is_empty() {
            return Verdict::Crowded(crowded);
        }
        Verdict::Blocked(blocked)
    }

    /// Whether the proven route from `start` has its passing spaces.
    fn spacing(&self, start: &ObjectId, destination: &ObjectId) -> Spacing {
        let Some(passing) = &self.declared.passing else {
            return Spacing::Met;
        };
        if start == destination {
            return Spacing::Met;
        }
        let services = match passing_spaces::Services::of(self.context) {
            Ok(services) => services,
            Err((reason, message)) => return Spacing::Unknown(reason, message),
        };
        let scene = self.scene;
        let spaces: BTreeSet<ObjectId> = scene
            .route
            .decided
            .iter()
            .chain([start, destination])
            .filter(|object| !scene.portals.contains(object))
            .cloned()
            .collect();
        let ground = Ground {
            spaces: &spaces,
            portals: &scene.portals.decided,
            obstacles: &scene.obstacles,
            swept: &scene.swept,
            body: self.declared.width,
        };
        passing_spaces::judge(passing, &services, &ground, start, destination)
    }

    /// Why `passage` blocks: its own judgement, or its width.
    fn blocking(
        &self,
        passage: &VerifiedWalkablePassage,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Judgement {
        let judged = self.judge(passage, from, to);
        if judged.admission == PassageAdmission::Refused {
            return judged;
        }
        if let Some(stretch) = passage.stretch() {
            return Judgement {
                admission: PassageAdmission::Refused,
                element: Some(stretch.surface().clone()),
                reason: Some(self.stretch_reason(stretch)),
                evidence: vec![passage.evidence().clone()],
                stairs_only: false,
                others: stretch.obstacles().to_vec(),
            };
        }
        let element = passage
            .portal()
            .or_else(|| passage.connector().map(VerticalConnector::object))
            .cloned();
        Judgement {
            admission: PassageAdmission::Refused,
            reason: Some(format!(
                "{} is at most {} m wide, less than the {} m body",
                element
                    .as_ref()
                    .map_or_else(|| "a passage".to_owned(), ToString::to_string),
                metres(passage.clear_width().upper_metres()),
                metres(self.declared.width)
            )),
            element,
            evidence: vec![passage.evidence().clone()],
            stairs_only: false,
            others: Vec::new(),
        }
    }

    /// Where and why a stretch of one route space stops the body.
    fn stretch_reason(&self, stretch: &WalkableStretch) -> String {
        let surface = stretch.surface();
        let [x, y, z] = stretch.at();
        let at = format!("({}, {}, {})", metres(x), metres(y), metres(z));
        let listed = || {
            stretch
                .obstacles()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let body = metres(self.declared.width);
        match stretch.limit() {
            StretchLimit::Narrow => {
                format!("{surface} is too narrow near {at} for a body {body} m wide")
            }
            StretchLimit::Obstructed if stretch.obstacles().is_empty() => {
                format!("{surface} is obstructed near {at} for a body {body} m wide")
            }
            StretchLimit::Obstructed => format!(
                "{surface} is obstructed near {at} for a body {body} m wide by {}",
                listed()
            ),
            StretchLimit::Low => {
                let under = if stretch.obstacles().is_empty() {
                    "overhead obstacles".to_owned()
                } else {
                    listed()
                };
                let headroom = stretch.headroom().map_or_else(
                    || "is below the clear height".to_owned(),
                    |headroom| {
                        format!(
                            "is {} m",
                            shown(headroom.lower_metres(), headroom.upper_metres())
                        )
                    },
                );
                format!(
                    "{surface} is too low near {at} for a body {body} m wide: the headroom \
                     under {under} {headroom}"
                )
            }
        }
    }

    /// The finding for a destination whose every proven route lacks
    /// passing spaces.
    fn crowded(
        &self,
        rule: &CompiledRule,
        destination: &ObjectId,
        missed: Vec<(ObjectId, String, Vec<Evidence>)>,
    ) -> axioval_ir::Finding {
        let mut evidence = vec![self.snapshot.evidence().clone()];
        let mut related = Vec::new();
        let mut messages = Vec::new();
        for (start, message, cited) in missed {
            related.push(start);
            messages.push(message);
            evidence.extend(cited);
        }
        finding(
            rule,
            destination,
            format!(
                "no route to {destination} has its passing spaces: {}",
                messages.join("; ")
            ),
            evidence,
            related,
        )
    }

    fn finding(
        &self,
        rule: &CompiledRule,
        destination: &ObjectId,
        blocked: Vec<Judgement>,
    ) -> axioval_ir::Finding {
        let mut evidence = vec![self.snapshot.evidence().clone()];
        let mut related = BTreeSet::new();
        let mut reasons = BTreeSet::new();
        let stairs_only = !blocked.is_empty() && blocked.iter().all(|judged| judged.stairs_only);
        for judged in blocked {
            evidence.extend(judged.evidence);
            related.extend(judged.element);
            related.extend(judged.others);
            reasons.extend(judged.reason);
        }
        let message = if reasons.is_empty() {
            format!("no route space connects {destination} to a start")
        } else if stairs_only {
            format!(
                "{destination} is connected to the starts by stairs only: {}",
                related
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!(
                "no accessible route reaches {destination} for a body {} m wide: {}",
                metres(self.declared.width),
                reasons.into_iter().collect::<Vec<_>>().join("; ")
            )
        };
        finding(
            rule,
            destination,
            message,
            evidence,
            related.into_iter().collect(),
        )
    }
}
