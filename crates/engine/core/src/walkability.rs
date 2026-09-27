//! Source-neutral walkable-region topology and service contracts.
use crate::{LengthInterval, ServiceRegistry, ServiceRegistryError};
use axioval_ir::{Evidence, ObjectId};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use thiserror::Error;
#[derive(Clone, Debug, Error, PartialEq)]
pub enum WalkabilityError {
    #[error("minimum width must be finite and positive")]
    InvalidMinimumWidth,
    #[error("walkability region identifier is blank")]
    InvalidRegionId,
    #[error("passage joins a region to itself")]
    SelfPassage,
    #[error("passage evidence is not exact and reviewable")]
    InexactPassage,
    #[error("walkability evidence is incomplete")]
    IncompleteEvidence,
    #[error("duplicate walkability region")]
    DuplicateRegion,
    #[error("passage names an unknown region")]
    UnknownRegion,
    #[error("duplicate walkable passage")]
    DuplicatePassage,
    #[error("region maps an object outside the request universe")]
    UnexpectedMappedObject,
    #[error("portal passage violates the request portal policy")]
    ForbiddenPortalPassage,
    #[error("walkability object is not mapped to a region")]
    ObjectUnavailable,
    #[error("backend returned another request")]
    ResponseRequestMismatch,
    #[error("vertical connector is declared twice with different kinds")]
    ConflictingConnector,
    #[error("passage is both a portal and a vertical connector")]
    PortalConnectorPassage,
    #[error("connector passage violates the request connector declaration")]
    ForbiddenConnectorPassage,
    /// A stated clear width is not finite and positive, names an object
    /// that is not a requested entrance, or is stated twice.
    #[error("stated clear width is invalid or names no requested entrance")]
    InvalidStatedClearWidth,
    /// The backend refused: evidence it would need is missing, approximate
    /// or outside what it can measure. Never a negative verdict.
    #[error("walkability unavailable: {0}")]
    Unavailable(String),
}
/// What kind of vertical connector joins walkable regions on different levels.
///
/// The kind is the rule's (or its source's) classification, carried in the
/// request, never inferred from geometry. A rule forbids a kind by routing
/// with [`WalkabilitySnapshot::route_between_avoiding`], so a route that
/// needs a stair is unreachable for it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VerticalConnectorKind {
    Lift,
    Ramp,
    Stair,
}
impl VerticalConnectorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lift => "lift",
            Self::Ramp => "ramp",
            Self::Stair => "stair",
        }
    }
}
/// A selected object that joins levels, and its kind.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct VerticalConnector {
    object: ObjectId,
    kind: VerticalConnectorKind,
}
impl VerticalConnector {
    pub fn new(object: ObjectId, kind: VerticalConnectorKind) -> Self {
        Self { object, kind }
    }
    pub fn object(&self) -> &ObjectId {
        &self.object
    }
    pub fn kind(&self) -> VerticalConnectorKind {
        self.kind
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct WalkabilityRequest {
    surfaces: Vec<ObjectId>,
    entrances: Vec<ObjectId>,
    obstacles: Vec<ObjectId>,
    minimum_width: f64,
    elevation_band: Option<LengthInterval>,
    traverse_verified_portals: bool,
    include_motion_envelopes: bool,
    connectors: Vec<VerticalConnector>,
    stated_clear_widths: BTreeMap<ObjectId, f64>,
}
impl WalkabilityRequest {
    pub fn try_new(
        mut surfaces: Vec<ObjectId>,
        mut entrances: Vec<ObjectId>,
        mut obstacles: Vec<ObjectId>,
        minimum_width: f64,
        elevation_band: Option<LengthInterval>,
        traverse_verified_portals: bool,
        include_motion_envelopes: bool,
    ) -> Result<Self, WalkabilityError> {
        if !minimum_width.is_finite() || minimum_width <= 0.0 {
            return Err(WalkabilityError::InvalidMinimumWidth);
        }
        surfaces.sort();
        surfaces.dedup();
        entrances.sort();
        entrances.dedup();
        obstacles.sort();
        obstacles.dedup();
        Ok(Self {
            surfaces,
            entrances,
            obstacles,
            minimum_width,
            elevation_band,
            traverse_verified_portals,
            include_motion_envelopes,
            connectors: Vec::new(),
            stated_clear_widths: BTreeMap::new(),
        })
    }
    /// States, in metres, the clear width a requested entrance's leaf and
    /// lining leave, as the rule reads it from its source (a door's stated
    /// clear width, say). A backend bounds the entrance's crossing by it
    /// from above and may use it to admit the body; it never widens what
    /// the geometry shows.
    ///
    /// # Errors
    ///
    /// [`WalkabilityError::InvalidStatedClearWidth`] when a width is not
    /// finite and positive, an object is not a requested entrance, or one
    /// entrance is stated twice.
    pub fn with_stated_clear_widths(
        mut self,
        widths: impl IntoIterator<Item = (ObjectId, f64)>,
    ) -> Result<Self, WalkabilityError> {
        let mut stated = BTreeMap::new();
        for (object, metres) in widths {
            if !metres.is_finite()
                || metres <= 0.0
                || self.entrances.binary_search(&object).is_err()
                || stated.insert(object, metres).is_some()
            {
                return Err(WalkabilityError::InvalidStatedClearWidth);
            }
        }
        self.stated_clear_widths = stated;
        Ok(self)
    }
    /// The clear width stated for `entrance`, if any.
    pub fn stated_clear_width(&self, entrance: &ObjectId) -> Option<f64> {
        self.stated_clear_widths.get(entrance).copied()
    }
    /// Every stated clear width, ordered by entrance.
    pub fn stated_clear_widths(&self) -> &BTreeMap<ObjectId, f64> {
        &self.stated_clear_widths
    }
    /// Declares the vertical connectors the backend may join levels through.
    ///
    /// # Errors
    ///
    /// [`WalkabilityError::ConflictingConnector`] when one object is given
    /// two kinds.
    pub fn with_connectors(
        mut self,
        mut connectors: Vec<VerticalConnector>,
    ) -> Result<Self, WalkabilityError> {
        connectors.sort();
        connectors.dedup();
        if connectors
            .windows(2)
            .any(|pair| pair[0].object == pair[1].object)
        {
            return Err(WalkabilityError::ConflictingConnector);
        }
        self.connectors = connectors;
        Ok(self)
    }
    pub fn connectors(&self) -> &[VerticalConnector] {
        &self.connectors
    }
    pub fn surfaces(&self) -> &[ObjectId] {
        &self.surfaces
    }
    pub fn entrances(&self) -> &[ObjectId] {
        &self.entrances
    }
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
    pub fn minimum_width_metres(&self) -> f64 {
        self.minimum_width
    }
    pub fn elevation_band(&self) -> Option<LengthInterval> {
        self.elevation_band
    }
    pub fn traverses_verified_portals(&self) -> bool {
        self.traverse_verified_portals
    }
    pub fn includes_motion_envelopes(&self) -> bool {
        self.include_motion_envelopes
    }
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WalkabilityRegionId(String);
impl WalkabilityRegionId {
    pub fn new(value: impl Into<String>) -> Result<Self, WalkabilityError> {
        let value = value.into();
        if value.trim().is_empty() {
            Err(WalkabilityError::InvalidRegionId)
        } else {
            Ok(Self(value))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct WalkabilityRegion {
    id: WalkabilityRegionId,
    objects: Vec<ObjectId>,
}
impl WalkabilityRegion {
    pub fn new(id: WalkabilityRegionId, mut objects: Vec<ObjectId>) -> Self {
        objects.sort();
        objects.dedup();
        Self { id, objects }
    }
    pub fn id(&self) -> &WalkabilityRegionId {
        &self.id
    }
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedWalkablePassage {
    a: WalkabilityRegionId,
    b: WalkabilityRegionId,
    portal: Option<ObjectId>,
    connector: Option<VerticalConnector>,
    clear_width: LengthInterval,
    evidence: Evidence,
}
impl VerifiedWalkablePassage {
    pub fn try_new(
        mut a: WalkabilityRegionId,
        mut b: WalkabilityRegionId,
        portal: Option<ObjectId>,
        clear_width: LengthInterval,
        evidence: Evidence,
    ) -> Result<Self, WalkabilityError> {
        if a == b {
            return Err(WalkabilityError::SelfPassage);
        }
        if !evidence.exact || evidence.locator.trim().is_empty() {
            return Err(WalkabilityError::InexactPassage);
        }
        if b < a {
            std::mem::swap(&mut a, &mut b);
        }
        Ok(Self {
            a,
            b,
            portal,
            connector: None,
            clear_width,
            evidence,
        })
    }
    /// Marks this passage as a climb through a vertical connector.
    ///
    /// # Errors
    ///
    /// [`WalkabilityError::PortalConnectorPassage`] when the passage already
    /// crosses a portal.
    pub fn with_connector(
        mut self,
        connector: VerticalConnector,
    ) -> Result<Self, WalkabilityError> {
        if self.portal.is_some() {
            return Err(WalkabilityError::PortalConnectorPassage);
        }
        self.connector = Some(connector);
        Ok(self)
    }
    pub fn connector(&self) -> Option<&VerticalConnector> {
        self.connector.as_ref()
    }
    pub fn endpoints(&self) -> (&WalkabilityRegionId, &WalkabilityRegionId) {
        (&self.a, &self.b)
    }
    pub fn portal(&self) -> Option<&ObjectId> {
        self.portal.as_ref()
    }
    pub fn clear_width(&self) -> LengthInterval {
        self.clear_width
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct WalkabilitySnapshot {
    request: WalkabilityRequest,
    regions: Vec<WalkabilityRegion>,
    passages: Vec<VerifiedWalkablePassage>,
    object_regions: BTreeMap<ObjectId, Vec<WalkabilityRegionId>>,
    evidence: Evidence,
}
impl WalkabilitySnapshot {
    pub fn try_new(
        request: WalkabilityRequest,
        mut regions: Vec<WalkabilityRegion>,
        mut passages: Vec<VerifiedWalkablePassage>,
        evidence: Evidence,
    ) -> Result<Self, WalkabilityError> {
        if !evidence.exact || evidence.locator.trim().is_empty() {
            return Err(WalkabilityError::IncompleteEvidence);
        }
        regions.sort_by(|a, b| a.id.cmp(&b.id));
        if regions.windows(2).any(|w| w[0].id == w[1].id) {
            return Err(WalkabilityError::DuplicateRegion);
        }
        let ids: BTreeSet<_> = regions.iter().map(|r| r.id.clone()).collect();
        if passages
            .iter()
            .any(|p| !ids.contains(&p.a) || !ids.contains(&p.b))
        {
            return Err(WalkabilityError::UnknownRegion);
        }
        let universe: BTreeSet<_> = request
            .surfaces()
            .iter()
            .chain(request.entrances())
            .chain(request.obstacles())
            .chain(request.connectors().iter().map(VerticalConnector::object))
            .cloned()
            .collect();
        if regions
            .iter()
            .flat_map(|region| region.objects.iter())
            .any(|object| !universe.contains(object))
        {
            return Err(WalkabilityError::UnexpectedMappedObject);
        }
        if passages.iter().any(|passage| {
            passage.portal.as_ref().is_some_and(|portal| {
                !request.traverse_verified_portals
                    || request.entrances.binary_search(portal).is_err()
            })
        }) {
            return Err(WalkabilityError::ForbiddenPortalPassage);
        }
        if passages.iter().any(|passage| {
            passage
                .connector
                .as_ref()
                .is_some_and(|connector| request.connectors.binary_search(connector).is_err())
        }) {
            return Err(WalkabilityError::ForbiddenConnectorPassage);
        }
        let key = |p: &VerifiedWalkablePassage| {
            (
                p.a.clone(),
                p.b.clone(),
                p.portal.clone(),
                p.connector.clone(),
            )
        };
        passages.sort_by_key(key);
        if passages
            .windows(2)
            .any(|window| key(&window[0]) == key(&window[1]))
        {
            return Err(WalkabilityError::DuplicatePassage);
        }
        let mut object_regions: BTreeMap<ObjectId, Vec<WalkabilityRegionId>> = BTreeMap::new();
        for region in &regions {
            for object in &region.objects {
                object_regions
                    .entry(object.clone())
                    .or_default()
                    .push(region.id.clone());
            }
        }
        for mapped in object_regions.values_mut() {
            mapped.sort();
            mapped.dedup();
        }
        Ok(Self {
            request,
            regions,
            passages,
            object_regions,
            evidence,
        })
    }
    pub fn request(&self) -> &WalkabilityRequest {
        &self.request
    }
    pub fn regions(&self) -> &[WalkabilityRegion] {
        &self.regions
    }
    pub fn passages(&self) -> &[VerifiedWalkablePassage] {
        &self.passages
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
    pub fn route_between(
        &self,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Result<WalkabilityRouteOutcome, WalkabilityError> {
        self.route_between_avoiding(from, to, &[])
    }
    /// Routes as [`Self::route_between`], but never through a passage whose
    /// vertical connector is of a `forbidden` kind. Forbidding
    /// [`VerticalConnectorKind::Stair`] makes a stairs-only connection
    /// unreachable.
    ///
    /// # Errors
    ///
    /// [`WalkabilityError::ObjectUnavailable`] when an endpoint is mapped to
    /// no region.
    pub fn route_between_avoiding(
        &self,
        from: &ObjectId,
        to: &ObjectId,
        forbidden: &[VerticalConnectorKind],
    ) -> Result<WalkabilityRouteOutcome, WalkabilityError> {
        self.route_between_admitting(from, to, |passage| {
            if passage
                .connector
                .as_ref()
                .is_some_and(|connector| forbidden.contains(&connector.kind))
            {
                PassageAdmission::Refused
            } else {
                PassageAdmission::Admitted
            }
        })
    }
    /// Routes as [`Self::route_between`], with a rule's own judgement of
    /// each passage on top of the width: the definite graph keeps only
    /// passages it [admits](PassageAdmission::Admitted), the possible graph
    /// drops only those it [refuses](PassageAdmission::Refused). A rule
    /// that cannot decide a passage (a door whose required width it cannot
    /// read) can therefore only turn a verdict `Indeterminate`.
    ///
    /// # Errors
    ///
    /// [`WalkabilityError::ObjectUnavailable`] when an endpoint is mapped to
    /// no region.
    pub fn route_between_admitting(
        &self,
        from: &ObjectId,
        to: &ObjectId,
        admit: impl Fn(&VerifiedWalkablePassage) -> PassageAdmission,
    ) -> Result<WalkabilityRouteOutcome, WalkabilityError> {
        let (starts, goals) = self.endpoints(from, to)?;
        if let Some(path) = self.path(starts, goals, false, &admit) {
            return Ok(WalkabilityRouteOutcome::Reachable(path));
        }
        if self.path(starts, goals, true, &admit).is_some() {
            Ok(WalkabilityRouteOutcome::Indeterminate)
        } else {
            Ok(WalkabilityRouteOutcome::Unreachable)
        }
    }
    /// The passages that block every route from `from` to `to`: those
    /// leaving the regions the possible graph reaches from `from` (too
    /// narrow, or refused by `admit`) towards a region from which `to` can
    /// be reached, widths and admission ignored, without re-entering them.
    /// Empty when a possible route exists, and when nothing joins the two
    /// at all.
    ///
    /// Any route, once it last leaves the reached regions, crosses a
    /// returned passage: they form a cut, ordered as the snapshot orders
    /// its passages. A block that only guards some other region is left
    /// out.
    ///
    /// # Errors
    ///
    /// [`WalkabilityError::ObjectUnavailable`] when an endpoint is mapped to
    /// no region.
    pub fn blocking_passages(
        &self,
        from: &ObjectId,
        to: &ObjectId,
        admit: impl Fn(&VerifiedWalkablePassage) -> PassageAdmission,
    ) -> Result<Vec<&VerifiedWalkablePassage>, WalkabilityError> {
        let (starts, goals) = self.endpoints(from, to)?;
        let reached = self.reach(starts, |edge| self.usable(edge, true, &admit));
        if goals.iter().any(|goal| reached.contains(goal)) {
            return Ok(Vec::new());
        }
        // Regions the goal reaches, ignoring widths and admission, without
        // passing through the reached side: a block behind another
        // reached region is not this goal's.
        let graph = self.graph(|_| true);
        let mut leads_to_goal: BTreeSet<WalkabilityRegionId> = goals.iter().cloned().collect();
        let mut queue: VecDeque<WalkabilityRegionId> = leads_to_goal.iter().cloned().collect();
        while let Some(node) = queue.pop_front() {
            for next in graph.get(&node).into_iter().flatten() {
                if !reached.contains(next) && leads_to_goal.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
        Ok(self
            .passages
            .iter()
            .filter(|edge| {
                let (a, b) = (reached.contains(&edge.a), reached.contains(&edge.b));
                (a && !b && leads_to_goal.contains(&edge.b))
                    || (b && !a && leads_to_goal.contains(&edge.a))
            })
            .collect())
    }
    fn endpoints(
        &self,
        from: &ObjectId,
        to: &ObjectId,
    ) -> Result<(&[WalkabilityRegionId], &[WalkabilityRegionId]), WalkabilityError> {
        let starts = self
            .object_regions
            .get(from)
            .ok_or(WalkabilityError::ObjectUnavailable)?;
        let goals = self
            .object_regions
            .get(to)
            .ok_or(WalkabilityError::ObjectUnavailable)?;
        Ok((starts, goals))
    }
    /// Whether `edge` belongs to the possible or the definite graph.
    fn usable(
        &self,
        edge: &VerifiedWalkablePassage,
        possible: bool,
        admit: &impl Fn(&VerifiedWalkablePassage) -> PassageAdmission,
    ) -> bool {
        let minimum = self.request.minimum_width;
        if possible {
            edge.clear_width.upper_metres() >= minimum && admit(edge) != PassageAdmission::Refused
        } else {
            edge.clear_width.lower_metres() >= minimum && admit(edge) == PassageAdmission::Admitted
        }
    }
    /// Every region reachable from `starts` through passages `keep` keeps.
    fn reach(
        &self,
        starts: &[WalkabilityRegionId],
        keep: impl Fn(&VerifiedWalkablePassage) -> bool,
    ) -> BTreeSet<WalkabilityRegionId> {
        let graph = self.graph(keep);
        let mut seen: BTreeSet<WalkabilityRegionId> = starts.iter().cloned().collect();
        let mut queue: VecDeque<WalkabilityRegionId> = seen.iter().cloned().collect();
        while let Some(node) = queue.pop_front() {
            for next in graph.get(&node).into_iter().flatten() {
                if seen.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
        seen
    }
    fn graph(
        &self,
        keep: impl Fn(&VerifiedWalkablePassage) -> bool,
    ) -> BTreeMap<WalkabilityRegionId, Vec<WalkabilityRegionId>> {
        let mut graph: BTreeMap<WalkabilityRegionId, Vec<WalkabilityRegionId>> = BTreeMap::new();
        for edge in self.passages.iter().filter(|edge| keep(edge)) {
            graph
                .entry(edge.a.clone())
                .or_default()
                .push(edge.b.clone());
            graph
                .entry(edge.b.clone())
                .or_default()
                .push(edge.a.clone());
        }
        for neighbors in graph.values_mut() {
            neighbors.sort();
            neighbors.dedup();
        }
        graph
    }
    fn path(
        &self,
        starts: &[WalkabilityRegionId],
        goals: &[WalkabilityRegionId],
        possible: bool,
        admit: &impl Fn(&VerifiedWalkablePassage) -> PassageAdmission,
    ) -> Option<Vec<WalkabilityRegionId>> {
        let graph = self.graph(|edge| self.usable(edge, possible, admit));
        let goal_set: BTreeSet<_> = goals.iter().cloned().collect();
        let mut queue = VecDeque::new();
        let mut parent: BTreeMap<WalkabilityRegionId, Option<WalkabilityRegionId>> =
            BTreeMap::new();
        for start in starts {
            if parent.insert(start.clone(), None).is_none() {
                queue.push_back(start.clone());
            }
        }
        while let Some(node) = queue.pop_front() {
            if goal_set.contains(&node) {
                let mut path = vec![node.clone()];
                let mut cursor = node;
                while let Some(Some(prev)) = parent.get(&cursor) {
                    path.push(prev.clone());
                    cursor = prev.clone();
                }
                path.reverse();
                return Some(path);
            }
            for next in graph.get(&node).into_iter().flatten() {
                if !parent.contains_key(next) {
                    parent.insert(next.clone(), Some(node.clone()));
                    queue.push_back(next.clone());
                }
            }
        }
        None
    }
}
/// A rule's judgement of one passage, on top of its width (see
/// [`WalkabilitySnapshot::route_between_admitting`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassageAdmission {
    /// The rule accepts the passage.
    Admitted,
    /// The rule cannot decide: the passage stays possible, never definite.
    Undecided,
    /// The rule rejects the passage: it is in neither graph.
    Refused,
}
#[derive(Clone, Debug, PartialEq)]
pub enum WalkabilityRouteOutcome {
    Reachable(Vec<WalkabilityRegionId>),
    Unreachable,
    Indeterminate,
}
pub trait WalkabilityService: Send + Sync {
    fn snapshot(
        &self,
        request: &WalkabilityRequest,
    ) -> Result<WalkabilitySnapshot, WalkabilityError>;
}
#[derive(Clone)]
pub struct WalkabilityServiceHandle(Arc<dyn WalkabilityService>);
impl WalkabilityServiceHandle {
    pub fn new(service: Arc<dyn WalkabilityService>) -> Self {
        Self(service)
    }
    pub fn snapshot(
        &self,
        request: &WalkabilityRequest,
    ) -> Result<WalkabilitySnapshot, WalkabilityError> {
        let snapshot = self.0.snapshot(request)?;
        if snapshot.request() != request {
            return Err(WalkabilityError::ResponseRequestMismatch);
        }
        Ok(snapshot)
    }
    pub fn register(self, services: &mut ServiceRegistry) -> Result<(), ServiceRegistryError> {
        services.register(self)
    }
}
