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
    #[error("placement offset interval is non-finite or reversed")]
    InvalidOffsetInterval,
    #[error("placement support gap must be finite and non-negative")]
    InvalidSupportGap,
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
    #[error("support evidence is not exact and reviewable")]
    InexactSupportEvidence,
    #[error("supported placement has no complete support evidence")]
    MissingSupportEvidence,
    #[error("support evidence does not match the requested support or found frame")]
    SupportEvidenceMismatch,
    #[error("placement frame is not grounded in the requested scope")]
    PlacementScopeMismatch,
    #[error("placement witness falls outside its requested search domain")]
    PlacementDomainMismatch,
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

/// Inclusive signed offset bounds in canonical metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignedDistanceInterval {
    lower_metres: f64,
    upper_metres: f64,
}
impl SignedDistanceInterval {
    pub fn try_new(lower_metres: f64, upper_metres: f64) -> Result<Self, FreeSpaceError> {
        if !lower_metres.is_finite() || !upper_metres.is_finite() || lower_metres > upper_metres {
            return Err(FreeSpaceError::InvalidOffsetInterval);
        }
        Ok(Self {
            lower_metres,
            upper_metres,
        })
    }
    pub fn exact(metres: f64) -> Result<Self, FreeSpaceError> {
        Self::try_new(metres, metres)
    }
    pub fn lower_metres(&self) -> f64 {
        self.lower_metres
    }
    pub fn upper_metres(&self) -> f64 {
        self.upper_metres
    }
    fn contains(self, value: f64) -> bool {
        value >= self.lower_metres && value <= self.upper_metres
    }
}

/// Requires the entire placement base to lie on an object's support surface.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportedPlacement {
    support: ObjectId,
    maximum_gap_metres: f64,
}
impl SupportedPlacement {
    pub fn try_new(support: ObjectId, maximum_gap_metres: f64) -> Result<Self, FreeSpaceError> {
        if !valid_non_negative(maximum_gap_metres) {
            return Err(FreeSpaceError::InvalidSupportGap);
        }
        Ok(Self {
            support,
            maximum_gap_metres,
        })
    }
    pub fn support(&self) -> &ObjectId {
        &self.support
    }
    pub fn maximum_gap_metres(&self) -> f64 {
        self.maximum_gap_metres
    }
}

/// Restricts candidate-frame origins to offsets in an anchor frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameOffsetPlacement {
    anchor: MetricFrame,
    right: SignedDistanceInterval,
    forward: SignedDistanceInterval,
    up: SignedDistanceInterval,
}
impl FrameOffsetPlacement {
    pub fn new(
        anchor: MetricFrame,
        right: SignedDistanceInterval,
        forward: SignedDistanceInterval,
        up: SignedDistanceInterval,
    ) -> Self {
        Self {
            anchor,
            right,
            forward,
            up,
        }
    }
    pub fn anchor(&self) -> &MetricFrame {
        &self.anchor
    }
    pub fn right(&self) -> SignedDistanceInterval {
        self.right
    }
    pub fn forward(&self) -> SignedDistanceInterval {
        self.forward
    }
    pub fn up(&self) -> SignedDistanceInterval {
        self.up
    }
    fn contains_frame(&self, frame: &MetricFrame) -> bool {
        let aligned = self.anchor.right() == frame.right()
            && self.anchor.forward() == frame.forward()
            && self.anchor.up() == frame.up();
        if !aligned {
            return false;
        }
        let anchor = self.anchor.origin().coordinates_metres();
        let found = frame.origin().coordinates_metres();
        let delta = [
            found[0] - anchor[0],
            found[1] - anchor[1],
            found[2] - anchor[2],
        ];
        let project = |axis: MetricDirection| {
            axis.components()
                .into_iter()
                .zip(delta)
                .map(|(a, b)| a * b)
                .sum()
        };
        self.right.contains(project(self.anchor.right()))
            && self.forward.contains(project(self.anchor.forward()))
            && self.up.contains(project(self.anchor.up()))
    }
}

/// Geometric predicate limiting where a backend may search for placements.
#[derive(Clone, Debug, PartialEq)]
pub enum PlacementDomain {
    Unconstrained,
    Supported(SupportedPlacement),
    FrameOffsets(FrameOffsetPlacement),
    SupportedFrameOffsets {
        support: SupportedPlacement,
        offsets: FrameOffsetPlacement,
    },
}

fn requested_support(domain: &PlacementDomain) -> Option<&SupportedPlacement> {
    match domain {
        PlacementDomain::Supported(support)
        | PlacementDomain::SupportedFrameOffsets { support, .. } => Some(support),
        _ => None,
    }
}

/// Searches an object-grounded scope for any placement of a clearance shape.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementRequest {
    scope: ObjectId,
    shape: ClearanceShape,
    obstacles: Vec<ObjectId>,
    domain: PlacementDomain,
}
impl PlacementRequest {
    pub fn new(scope: ObjectId, shape: ClearanceShape, mut obstacles: Vec<ObjectId>) -> Self {
        obstacles.sort();
        obstacles.dedup();
        Self {
            scope,
            shape,
            obstacles,
            domain: PlacementDomain::Unconstrained,
        }
    }
    pub fn new_in_domain(
        scope: ObjectId,
        shape: ClearanceShape,
        mut obstacles: Vec<ObjectId>,
        domain: PlacementDomain,
    ) -> Result<Self, FreeSpaceError> {
        let offsets = match &domain {
            PlacementDomain::FrameOffsets(offsets)
            | PlacementDomain::SupportedFrameOffsets { offsets, .. } => Some(offsets),
            _ => None,
        };
        if offsets.is_some_and(|offsets| offsets.anchor().origin().subject() != &scope) {
            return Err(FreeSpaceError::PlacementScopeMismatch);
        }
        obstacles.sort();
        obstacles.dedup();
        Ok(Self {
            scope,
            shape,
            obstacles,
            domain,
        })
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
    pub fn domain(&self) -> &PlacementDomain {
        &self.domain
    }
}

/// Exact proof that the entire candidate base is supported at a found frame.
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteSupportEvidence {
    support: ObjectId,
    frame: MetricFrame,
    maximum_gap_metres: f64,
    evidence: Evidence,
}

impl CompleteSupportEvidence {
    pub fn try_new(
        support: ObjectId,
        frame: MetricFrame,
        maximum_gap_metres: f64,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if !valid_non_negative(maximum_gap_metres) {
            return Err(FreeSpaceError::InvalidSupportGap);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::InexactSupportEvidence);
        }
        Ok(Self {
            support,
            frame,
            maximum_gap_metres,
            evidence,
        })
    }
    pub fn support(&self) -> &ObjectId {
        &self.support
    }
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }
    pub fn maximum_gap_metres(&self) -> f64 {
        self.maximum_gap_metres
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
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
    support_evidence: Option<Box<CompleteSupportEvidence>>,
    evidence: Evidence,
}
fn validate_placement_witness(
    request: &PlacementRequest,
    frame: &MetricFrame,
    evidence: &Evidence,
) -> Result<(), FreeSpaceError> {
    if frame.origin().subject() != request.scope() {
        return Err(FreeSpaceError::PlacementScopeMismatch);
    }
    let offsets = match request.domain() {
        PlacementDomain::FrameOffsets(offsets)
        | PlacementDomain::SupportedFrameOffsets { offsets, .. } => Some(offsets),
        _ => None,
    };
    if offsets.is_some_and(|offsets| !offsets.contains_frame(frame)) {
        return Err(FreeSpaceError::PlacementDomainMismatch);
    }
    if !reviewable_exact_evidence(evidence) {
        return Err(FreeSpaceError::InexactPlacementEvidence);
    }
    Ok(())
}

impl ClearancePlacementEvidence {
    pub fn try_new(
        request: PlacementRequest,
        frame: MetricFrame,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if requested_support(request.domain()).is_some() {
            return Err(FreeSpaceError::MissingSupportEvidence);
        }
        validate_placement_witness(&request, &frame, &evidence)?;
        Ok(Self {
            request,
            frame,
            support_evidence: None,
            evidence,
        })
    }
    pub fn try_new_supported(
        request: PlacementRequest,
        frame: MetricFrame,
        support_evidence: CompleteSupportEvidence,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        validate_placement_witness(&request, &frame, &evidence)?;
        let required =
            requested_support(request.domain()).ok_or(FreeSpaceError::SupportEvidenceMismatch)?;
        if support_evidence.support() != required.support()
            || support_evidence.frame() != &frame
            || support_evidence.maximum_gap_metres() > required.maximum_gap_metres()
        {
            return Err(FreeSpaceError::SupportEvidenceMismatch);
        }
        Ok(Self {
            request,
            frame,
            support_evidence: Some(Box::new(support_evidence)),
            evidence,
        })
    }
    pub fn request(&self) -> &PlacementRequest {
        &self.request
    }
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }
    pub fn support_evidence(&self) -> Option<&CompleteSupportEvidence> {
        self.support_evidence.as_deref()
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
