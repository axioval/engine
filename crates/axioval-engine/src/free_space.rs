//! Source-neutral free-area and clearance host-service contracts.
//!
//! Geometry algorithms and native shapes remain in Axiolid or another trusted
//! backend. This module carries canonical metric requests and reviewable evidence.

use crate::{MetricPoint, MobilityProfile, ThresholdVerdict};
use axioval_ir::{Evidence, ObjectId};
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FreeSpaceError {
    #[error("free-area interval is invalid")]
    InvalidAreaInterval,
    #[error("metric direction is zero or non-finite")]
    InvalidMetricDirection,
    #[error("metric frame axes are not mutually perpendicular")]
    InvalidMetricFrame,
    #[error("clearance shape dimensions must be positive and finite")]
    InvalidClearanceShape,
    #[error("clearance evidence is incomplete")]
    IncompleteClearanceEvidence,
    #[error("obstruction evidence has no blocking objects")]
    EmptyObstructionEvidence,
    #[error("obstruction evidence names an object outside the request candidate set")]
    UnexpectedObstacleEvidence,
    #[error("obstruction provenance is not exact and reviewable")]
    InexactObstructionEvidence,
    #[error("free-area evidence is not exact and reviewable")]
    InexactAreaEvidence,
    #[error("placement evidence is not exact and reviewable")]
    InexactPlacementEvidence,
    #[error("placement frame is not grounded in the requested scope")]
    PlacementScopeMismatch,
    #[error("free-space backend returned evidence for another request")]
    ResponseRequestMismatch,
    #[error("free-space geometry is unavailable for `{0}`")]
    MissingGeometry(Box<ObjectId>),
    #[error("free-space query unavailable: {0}")]
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaInterval {
    lower_square_metres: f64,
    upper_square_metres: f64,
}
impl AreaInterval {
    pub fn try_new(lower: f64, upper: f64) -> Result<Self, FreeSpaceError> {
        if !valid_non_negative(lower) || !valid_non_negative(upper) || lower > upper {
            return Err(FreeSpaceError::InvalidAreaInterval);
        }
        Ok(Self {
            lower_square_metres: lower,
            upper_square_metres: upper,
        })
    }
    pub fn exact(square_metres: f64) -> Result<Self, FreeSpaceError> {
        Self::try_new(square_metres, square_metres)
    }
    pub fn lower_square_metres(&self) -> f64 {
        self.lower_square_metres
    }
    pub fn upper_square_metres(&self) -> f64 {
        self.upper_square_metres
    }
    pub fn compare_minimum(&self, minimum: f64) -> Result<ThresholdVerdict, FreeSpaceError> {
        if !valid_non_negative(minimum) {
            return Err(FreeSpaceError::InvalidAreaInterval);
        }
        if self.lower_square_metres >= minimum {
            Ok(ThresholdVerdict::Satisfied)
        } else if self.upper_square_metres < minimum {
            Ok(ThresholdVerdict::Violated)
        } else {
            Ok(ThresholdVerdict::Indeterminate)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricDirection([f64; 3]);
impl MetricDirection {
    pub fn try_new(vector: [f64; 3]) -> Result<Self, FreeSpaceError> {
        if !vector.iter().all(|v| v.is_finite()) {
            return Err(FreeSpaceError::InvalidMetricDirection);
        }
        let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm <= f64::EPSILON {
            return Err(FreeSpaceError::InvalidMetricDirection);
        }
        Ok(Self([vector[0] / norm, vector[1] / norm, vector[2] / norm]))
    }
    pub fn components(&self) -> [f64; 3] {
        self.0
    }
    fn dot(self, other: Self) -> f64 {
        self.0[0] * other.0[0] + self.0[1] * other.0[1] + self.0[2] * other.0[2]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricFrame {
    origin: MetricPoint,
    right: MetricDirection,
    forward: MetricDirection,
    up: MetricDirection,
}
impl MetricFrame {
    pub fn try_new(
        origin: MetricPoint,
        right: MetricDirection,
        forward: MetricDirection,
        up: MetricDirection,
    ) -> Result<Self, FreeSpaceError> {
        const ORTHOGONAL_TOLERANCE: f64 = 1.0e-9;
        let [rx, ry, rz] = right.components();
        let [fx, fy, fz] = forward.components();
        let [ux, uy, uz] = up.components();
        let handedness =
            (ry * fz - rz * fy) * ux + (rz * fx - rx * fz) * uy + (rx * fy - ry * fx) * uz;
        if right.dot(forward).abs() > ORTHOGONAL_TOLERANCE
            || right.dot(up).abs() > ORTHOGONAL_TOLERANCE
            || forward.dot(up).abs() > ORTHOGONAL_TOLERANCE
            || handedness < 1.0 - ORTHOGONAL_TOLERANCE
        {
            return Err(FreeSpaceError::InvalidMetricFrame);
        }
        Ok(Self {
            origin,
            right,
            forward,
            up,
        })
    }
    pub fn origin(&self) -> &MetricPoint {
        &self.origin
    }
    pub fn right(&self) -> MetricDirection {
        self.right
    }
    pub fn forward(&self) -> MetricDirection {
        self.forward
    }
    pub fn up(&self) -> MetricDirection {
        self.up
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxClearance {
    width: f64,
    depth: f64,
    height: f64,
}
impl BoxClearance {
    pub fn try_new(width: f64, depth: f64, height: f64) -> Result<Self, FreeSpaceError> {
        if !valid_positive(width) || !valid_positive(depth) || !valid_positive(height) {
            return Err(FreeSpaceError::InvalidClearanceShape);
        }
        Ok(Self {
            width,
            depth,
            height,
        })
    }
    pub fn width_metres(&self) -> f64 {
        self.width
    }
    pub fn depth_metres(&self) -> f64 {
        self.depth
    }
    pub fn height_metres(&self) -> f64 {
        self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CylinderClearance {
    radius: f64,
    height: f64,
}
impl CylinderClearance {
    pub fn try_new(radius: f64, height: f64) -> Result<Self, FreeSpaceError> {
        if !valid_positive(radius) || !valid_positive(height) {
            return Err(FreeSpaceError::InvalidClearanceShape);
        }
        Ok(Self { radius, height })
    }
    pub fn radius_metres(&self) -> f64 {
        self.radius
    }
    pub fn height_metres(&self) -> f64 {
        self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClearanceShape {
    Box(BoxClearance),
    Cylinder(CylinderClearance),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceRequest {
    frame: MetricFrame,
    shape: ClearanceShape,
    obstacles: Vec<ObjectId>,
}
impl ClearanceRequest {
    pub fn new(frame: MetricFrame, shape: ClearanceShape, mut obstacles: Vec<ObjectId>) -> Self {
        obstacles.sort();
        obstacles.dedup();
        Self {
            frame,
            shape,
            obstacles,
        }
    }
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }
    pub fn shape(&self) -> ClearanceShape {
        self.shape
    }
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FreeAreaRequest {
    scope: ObjectId,
    mobility: MobilityProfile,
    obstacles: Vec<ObjectId>,
}
impl FreeAreaRequest {
    pub fn new(scope: ObjectId, mobility: MobilityProfile, mut obstacles: Vec<ObjectId>) -> Self {
        obstacles.sort();
        obstacles.dedup();
        Self {
            scope,
            mobility,
            obstacles,
        }
    }
    pub fn scope(&self) -> &ObjectId {
        &self.scope
    }
    pub fn mobility(&self) -> MobilityProfile {
        self.mobility
    }
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
}

/// Searches an object-grounded scope for any placement of a clearance shape.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementRequest {
    scope: ObjectId,
    shape: ClearanceShape,
    obstacles: Vec<ObjectId>,
}
impl PlacementRequest {
    pub fn new(scope: ObjectId, shape: ClearanceShape, mut obstacles: Vec<ObjectId>) -> Self {
        obstacles.sort();
        obstacles.dedup();
        Self {
            scope,
            shape,
            obstacles,
        }
    }
    pub fn scope(&self) -> &ObjectId {
        &self.scope
    }
    pub fn shape(&self) -> ClearanceShape {
        self.shape
    }
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompleteClearanceEvidence {
    request: ClearanceRequest,
    evidence: Evidence,
}
impl CompleteClearanceEvidence {
    pub fn try_new(request: ClearanceRequest, evidence: Evidence) -> Result<Self, FreeSpaceError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::IncompleteClearanceEvidence);
        }
        Ok(Self { request, evidence })
    }
    pub fn request(&self) -> &ClearanceRequest {
        &self.request
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObstructionEvidence {
    request: ClearanceRequest,
    blockers: Vec<ObjectId>,
    evidence: Evidence,
}
impl ObstructionEvidence {
    pub fn try_new(
        request: ClearanceRequest,
        mut blockers: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if blockers.is_empty() {
            return Err(FreeSpaceError::EmptyObstructionEvidence);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::InexactObstructionEvidence);
        }
        blockers.sort();
        blockers.dedup();
        if blockers
            .iter()
            .any(|blocker| request.obstacles().binary_search(blocker).is_err())
        {
            return Err(FreeSpaceError::UnexpectedObstacleEvidence);
        }
        Ok(Self {
            request,
            blockers,
            evidence,
        })
    }
    pub fn request(&self) -> &ClearanceRequest {
        &self.request
    }
    pub fn blockers(&self) -> &[ObjectId] {
        &self.blockers
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClearanceOutcome {
    Clear(CompleteClearanceEvidence),
    Obstructed(ObstructionEvidence),
}

/// One exact placement witness. It does not claim exhaustive search coverage.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearancePlacementEvidence {
    request: PlacementRequest,
    frame: MetricFrame,
    evidence: Evidence,
}
impl ClearancePlacementEvidence {
    pub fn try_new(
        request: PlacementRequest,
        frame: MetricFrame,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if frame.origin().subject() != request.scope() {
            return Err(FreeSpaceError::PlacementScopeMismatch);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::InexactPlacementEvidence);
        }
        Ok(Self {
            request,
            frame,
            evidence,
        })
    }
    pub fn request(&self) -> &PlacementRequest {
        &self.request
    }
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Exact, complete evidence that no valid placement exists.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletePlacementEvidence {
    request: PlacementRequest,
    evidence: Evidence,
}
impl CompletePlacementEvidence {
    pub fn try_new(request: PlacementRequest, evidence: Evidence) -> Result<Self, FreeSpaceError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::IncompleteClearanceEvidence);
        }
        Ok(Self { request, evidence })
    }
    pub fn request(&self) -> &PlacementRequest {
        &self.request
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlacementOutcome {
    Found(ClearancePlacementEvidence),
    NoPlacement(CompletePlacementEvidence),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FreeAreaEvidence {
    request: FreeAreaRequest,
    available_area: AreaInterval,
    evidence: Evidence,
}
impl FreeAreaEvidence {
    pub fn try_new(
        request: FreeAreaRequest,
        available_area: AreaInterval,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::InexactAreaEvidence);
        }
        Ok(Self {
            request,
            available_area,
            evidence,
        })
    }
    pub fn request(&self) -> &FreeAreaRequest {
        &self.request
    }
    pub fn available_area(&self) -> &AreaInterval {
        &self.available_area
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

pub trait FreeSpaceService: Send + Sync + 'static {
    fn assess_clearance(
        &self,
        request: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError>;
    fn find_placement(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementOutcome, FreeSpaceError>;
    fn measure_free_area(
        &self,
        request: &FreeAreaRequest,
    ) -> Result<FreeAreaEvidence, FreeSpaceError>;
}

#[derive(Clone)]
pub struct FreeSpaceServiceHandle(Arc<dyn FreeSpaceService>);
impl FreeSpaceServiceHandle {
    pub fn new(service: Arc<dyn FreeSpaceService>) -> Self {
        Self(service)
    }
    pub fn assess_clearance(
        &self,
        request: &ClearanceRequest,
    ) -> Result<ClearanceOutcome, FreeSpaceError> {
        let outcome = self.0.assess_clearance(request)?;
        let actual = match &outcome {
            ClearanceOutcome::Clear(value) => value.request(),
            ClearanceOutcome::Obstructed(value) => value.request(),
        };
        if actual != request {
            return Err(FreeSpaceError::ResponseRequestMismatch);
        }
        Ok(outcome)
    }
    pub fn find_placement(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementOutcome, FreeSpaceError> {
        let outcome = self.0.find_placement(request)?;
        let actual = match &outcome {
            PlacementOutcome::Found(value) => value.request(),
            PlacementOutcome::NoPlacement(value) => value.request(),
        };
        if actual != request {
            return Err(FreeSpaceError::ResponseRequestMismatch);
        }
        Ok(outcome)
    }
    pub fn measure_free_area(
        &self,
        request: &FreeAreaRequest,
    ) -> Result<FreeAreaEvidence, FreeSpaceError> {
        let evidence = self.0.measure_free_area(request)?;
        if evidence.request() != request {
            return Err(FreeSpaceError::ResponseRequestMismatch);
        }
        Ok(evidence)
    }
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}
fn valid_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}
fn reviewable_exact_evidence(evidence: &Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}
