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
        })
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
            clear_width,
            evidence,
        })
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
        passages.sort_by(|a, b| (&a.a, &a.b, &a.portal).cmp(&(&b.a, &b.b, &b.portal)));
        if passages.windows(2).any(|window| {
            (&window[0].a, &window[0].b, &window[0].portal)
                == (&window[1].a, &window[1].b, &window[1].portal)
        }) {
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
        let starts = self
            .object_regions
            .get(from)
            .ok_or(WalkabilityError::ObjectUnavailable)?;
        let goals = self
            .object_regions
            .get(to)
            .ok_or(WalkabilityError::ObjectUnavailable)?;
        if let Some(path) = self.path(starts, goals, false) {
            return Ok(WalkabilityRouteOutcome::Reachable(path));
        }
        if self.path(starts, goals, true).is_some() {
            Ok(WalkabilityRouteOutcome::Indeterminate)
        } else {
            Ok(WalkabilityRouteOutcome::Unreachable)
        }
    }
    fn path(
        &self,
        starts: &[WalkabilityRegionId],
        goals: &[WalkabilityRegionId],
        possible: bool,
    ) -> Option<Vec<WalkabilityRegionId>> {
        let mut graph: BTreeMap<WalkabilityRegionId, Vec<WalkabilityRegionId>> = BTreeMap::new();
        for edge in &self.passages {
            let usable = if possible {
                edge.clear_width.upper_metres() >= self.request.minimum_width
            } else {
                edge.clear_width.lower_metres() >= self.request.minimum_width
            };
            if usable {
                graph
                    .entry(edge.a.clone())
                    .or_default()
                    .push(edge.b.clone());
                graph
                    .entry(edge.b.clone())
                    .or_default()
                    .push(edge.a.clone());
            }
        }
        for neighbors in graph.values_mut() {
            neighbors.sort();
            neighbors.dedup();
        }
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
