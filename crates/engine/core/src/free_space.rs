//! Source-neutral free-area and clearance host-service contracts.
//!
//! Geometry algorithms and native shapes remain in Axiolid or another trusted
//! backend. This module carries canonical metric requests and reviewable evidence.

use crate::circulation::{CirculationMap, CirculationRequest};
use crate::door_leaves::{SweptDoor, tidy_swept};
use crate::services::reviewable_exact_evidence;
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
    #[error("placement witness does not follow the requested orientation")]
    PlacementOrientationMismatch,
    #[error("frame-offset placement of a box needs a fixed orientation along the anchor axes")]
    OrientationDomainConflict,
    #[error("placement elevation band must be finite, start at or above the floor and be ordered")]
    InvalidElevationBand,
    #[error("a merged scope is the search scope itself or one of its obstacles")]
    MergedScopeConflict,
    /// One door is given twice with different swept sectors.
    #[error("a swept door is given twice with different sectors")]
    ConflictingSweptDoors,
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
    /// Whether a found frame keeps the anchor's axes and lies within the
    /// offsets. Placement evidence is validated with exactly this test, so a
    /// backend may filter its witnesses with it.
    pub fn contains_frame(&self, frame: &MetricFrame) -> bool {
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

/// Which rotations of a box placement count as a fit.
///
/// Answers differ by orientation: a box that fits only diagonally has no
/// placement along a fixed frame but has one at some angle. Evidence for a
/// fixed orientation says nothing about other angles.
#[derive(Clone, Debug, PartialEq)]
pub enum PlacementOrientation {
    /// The box width follows the frame's right axis and its depth the forward
    /// axis. Only the axes are binding; the frame origin is not a location.
    Fixed(MetricFrame),
    /// Every rotation about the vertical axis counts.
    Any,
}

/// The elevations, relative to the scope's floor, in which obstacles count.
///
/// Only the part of an obstacle's solid inside the open band blocks a
/// placement. Without a band, the band runs from the floor up by the shape's
/// height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationBand {
    from_metres: f64,
    to_metres: f64,
}
impl ElevationBand {
    pub fn try_new(from_metres: f64, to_metres: f64) -> Result<Self, FreeSpaceError> {
        if !from_metres.is_finite()
            || !to_metres.is_finite()
            || from_metres < 0.0
            || from_metres >= to_metres
        {
            return Err(FreeSpaceError::InvalidElevationBand);
        }
        Ok(Self {
            from_metres,
            to_metres,
        })
    }
    /// Bottom of the band above the floor.
    pub fn from_metres(&self) -> f64 {
        self.from_metres
    }
    /// Top of the band above the floor.
    pub fn to_metres(&self) -> f64 {
        self.to_metres
    }
}

/// A clearance shape together with the rotations a placement may use.
///
/// A box carries an explicit orientation so that a request cannot leave open
/// which question it asks. A cylinder is rotation-invariant and carries none.
#[derive(Clone, Debug, PartialEq)]
pub enum PlacementShape {
    Box {
        shape: BoxClearance,
        orientation: PlacementOrientation,
    },
    Cylinder(CylinderClearance),
}
impl PlacementShape {
    pub fn clearance(&self) -> ClearanceShape {
        match self {
            Self::Box { shape, .. } => ClearanceShape::Box(*shape),
            Self::Cylinder(shape) => ClearanceShape::Cylinder(*shape),
        }
    }
    pub fn orientation(&self) -> Option<&PlacementOrientation> {
        match self {
            Self::Box { orientation, .. } => Some(orientation),
            Self::Cylinder(_) => None,
        }
    }
}

fn same_axes(a: &MetricFrame, b: &MetricFrame) -> bool {
    a.right() == b.right() && a.forward() == b.forward() && a.up() == b.up()
}

/// Searches an object-grounded scope for any placement of a clearance shape.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementRequest {
    scope: ObjectId,
    shape: PlacementShape,
    obstacles: Vec<ObjectId>,
    domain: PlacementDomain,
    band: Option<ElevationBand>,
    merged: Vec<ObjectId>,
    swept: Vec<SweptDoor>,
}
impl PlacementRequest {
    pub fn new(scope: ObjectId, shape: PlacementShape, mut obstacles: Vec<ObjectId>) -> Self {
        obstacles.sort();
        obstacles.dedup();
        Self {
            scope,
            shape,
            obstacles,
            domain: PlacementDomain::Unconstrained,
            band: None,
            merged: Vec::new(),
            swept: Vec::new(),
        }
    }
    pub fn new_in_domain(
        scope: ObjectId,
        shape: PlacementShape,
        mut obstacles: Vec<ObjectId>,
        domain: PlacementDomain,
    ) -> Result<Self, FreeSpaceError> {
        let offsets = match &domain {
            PlacementDomain::FrameOffsets(offsets)
            | PlacementDomain::SupportedFrameOffsets { offsets, .. } => Some(offsets),
            _ => None,
        };
        // The anchor may be grounded on another object, such as a door or a
        // fixture in front of which the shape must fit; the witness is still
        // grounded on the scope.
        // Offset witnesses must align with the anchor, so a box searched there
        // can only be asked about the anchor's own orientation.
        if let (Some(offsets), Some(orientation)) = (offsets, shape.orientation()) {
            match orientation {
                PlacementOrientation::Fixed(frame) if same_axes(frame, offsets.anchor()) => {}
                _ => return Err(FreeSpaceError::OrientationDomainConflict),
            }
        }
        obstacles.sort();
        obstacles.dedup();
        Ok(Self {
            scope,
            shape,
            obstacles,
            domain,
            band: None,
            merged: Vec::new(),
            swept: Vec::new(),
        })
    }
    /// Counts obstacles only inside `band` above the scope's floor.
    #[must_use]
    pub fn with_band(mut self, band: ElevationBand) -> Self {
        self.band = Some(band);
        self
    }
    /// Searches the union of the scope and `merged` scopes, such as the
    /// spaces of one group. The witness stays grounded on the scope. A merged
    /// scope must be neither the scope nor an obstacle.
    pub fn with_merged_scopes(mut self, mut merged: Vec<ObjectId>) -> Result<Self, FreeSpaceError> {
        merged.sort();
        merged.dedup();
        if merged
            .iter()
            .any(|id| id == &self.scope || self.obstacles.binary_search(id).is_ok())
        {
            return Err(FreeSpaceError::MergedScopeConflict);
        }
        self.merged = merged;
        Ok(self)
    }
    /// Counts the sectors `swept` doors sweep as obstacles (see
    /// [`SweptDoor`]): a witness must stay clear of their circumscribed
    /// polygons, a proof of absence holds against their inscribed ones.
    ///
    /// # Errors
    ///
    /// [`FreeSpaceError::ConflictingSweptDoors`] when one door is given
    /// twice with different sectors.
    pub fn with_swept_doors(mut self, swept: Vec<SweptDoor>) -> Result<Self, FreeSpaceError> {
        self.swept = tidy_swept(swept).ok_or(FreeSpaceError::ConflictingSweptDoors)?;
        Ok(self)
    }
    /// The doors whose swept sectors are obstacles, sorted by door.
    pub fn swept_doors(&self) -> &[SweptDoor] {
        &self.swept
    }
    pub fn scope(&self) -> &ObjectId {
        &self.scope
    }
    pub fn shape(&self) -> &PlacementShape {
        &self.shape
    }
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
    pub fn domain(&self) -> &PlacementDomain {
        &self.domain
    }
    /// The band the request states, if any.
    pub fn band(&self) -> Option<ElevationBand> {
        self.band
    }
    /// The band obstacles count in: the stated one, or from the floor up by
    /// the shape's height.
    pub fn effective_band(&self) -> ElevationBand {
        self.band.unwrap_or(ElevationBand {
            from_metres: 0.0,
            to_metres: match &self.shape {
                PlacementShape::Box { shape, .. } => shape.height_metres(),
                PlacementShape::Cylinder(shape) => shape.height_metres(),
            },
        })
    }
    /// Scopes searched together with the scope, sorted.
    pub fn merged_scopes(&self) -> &[ObjectId] {
        &self.merged
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

/// Asks whether a clearance volume's plan footprint lies inside the union of
/// the plan footprints of `scopes`, such as the spaces a component stands in.
///
/// Only the plan is compared: the volume's height is carried so the request
/// names the same volume a clearance request does, not to compare it with
/// the scopes' heights. The scopes are the rule's selection, sorted and
/// deduplicated; with none, nothing covers the footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainmentRequest {
    frame: MetricFrame,
    shape: ClearanceShape,
    scopes: Vec<ObjectId>,
}
impl ContainmentRequest {
    pub fn new(frame: MetricFrame, shape: ClearanceShape, mut scopes: Vec<ObjectId>) -> Self {
        scopes.sort();
        scopes.dedup();
        Self {
            frame,
            shape,
            scopes,
        }
    }
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }
    pub fn shape(&self) -> ClearanceShape {
        self.shape
    }
    pub fn scopes(&self) -> &[ObjectId] {
        &self.scopes
    }
}

/// Exact evidence for a containment answer, bound to its request.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainmentEvidence {
    request: ContainmentRequest,
    evidence: Evidence,
}
impl ContainmentEvidence {
    pub fn try_new(
        request: ContainmentRequest,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::IncompleteClearanceEvidence);
        }
        Ok(Self { request, evidence })
    }
    pub fn request(&self) -> &ContainmentRequest {
        &self.request
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Whether a clearance footprint lies inside its scopes.
///
/// Both answers are claims about the whole footprint: `Inside` that no part
/// of positive area lies outside every scope, `Outside` that some part does.
/// A backend that can only bound the footprint (a cylinder's disc) answers
/// neither while the bounds disagree.
#[derive(Clone, Debug, PartialEq)]
pub enum ContainmentOutcome {
    Inside(ContainmentEvidence),
    Outside(ContainmentEvidence),
}

/// Asks whether the tops of `supports` hold a clearance footprint (the
/// plan of `frame` and `shape`, as for [`ContainmentRequest`]): the
/// upward-facing surfaces of the supports between `from` and `to` metres of
/// elevation, such as a slab or landing near the floor under a door's clear
/// area.
///
/// Only the plan and the elevation band are compared. The supports are the
/// rule's selection, sorted and deduplicated; with none, nothing holds the
/// footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportCoverageRequest {
    frame: MetricFrame,
    shape: ClearanceShape,
    supports: Vec<ObjectId>,
    from: f64,
    to: f64,
}
impl SupportCoverageRequest {
    /// Refuses a band that is not finite or whose ends are reversed.
    pub fn try_new(
        frame: MetricFrame,
        shape: ClearanceShape,
        mut supports: Vec<ObjectId>,
        from_metres: f64,
        to_metres: f64,
    ) -> Result<Self, FreeSpaceError> {
        if !(from_metres.is_finite() && to_metres.is_finite() && from_metres <= to_metres) {
            return Err(FreeSpaceError::InvalidElevationBand);
        }
        supports.sort();
        supports.dedup();
        Ok(Self {
            frame,
            shape,
            supports,
            from: from_metres,
            to: to_metres,
        })
    }
    pub fn frame(&self) -> &MetricFrame {
        &self.frame
    }
    pub fn shape(&self) -> ClearanceShape {
        self.shape
    }
    pub fn supports(&self) -> &[ObjectId] {
        &self.supports
    }
    /// The lowest elevation a top may lie at, in metres.
    pub fn from_metres(&self) -> f64 {
        self.from
    }
    /// The highest elevation a top may lie at, in metres.
    pub fn to_metres(&self) -> f64 {
        self.to
    }
}

/// Exact evidence for a support-coverage answer, bound to its request.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportCoverageEvidence {
    request: SupportCoverageRequest,
    evidence: Evidence,
}
impl SupportCoverageEvidence {
    pub fn try_new(
        request: SupportCoverageRequest,
        evidence: Evidence,
    ) -> Result<Self, FreeSpaceError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(FreeSpaceError::InexactSupportEvidence);
        }
        Ok(Self { request, evidence })
    }
    pub fn request(&self) -> &SupportCoverageRequest {
        &self.request
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Whether the supports' tops hold a clearance footprint.
///
/// Both answers are whole-footprint claims: `Supported` that no part of
/// positive area lies outside the tops within the band, `Unsupported` that
/// some part does. A backend that can only bound the footprint (a
/// cylinder's disc) answers neither while the bounds disagree.
#[derive(Clone, Debug, PartialEq)]
pub enum SupportCoverageOutcome {
    Supported(SupportCoverageEvidence),
    Unsupported(SupportCoverageEvidence),
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
    if let Some(PlacementOrientation::Fixed(fixed)) = request.shape().orientation() {
        if !same_axes(fixed, frame) {
            return Err(FreeSpaceError::PlacementOrientationMismatch);
        }
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
    /// Whether a clearance footprint lies inside its scopes. A service that
    /// does not compare footprints refuses, never answering either way.
    fn assess_containment(
        &self,
        request: &ContainmentRequest,
    ) -> Result<ContainmentOutcome, FreeSpaceError> {
        let _ = request;
        Err(FreeSpaceError::Unavailable(
            "this free-space service does not compare clearance footprints with scopes".into(),
        ))
    }
    /// Whether the tops of the request's supports hold a clearance
    /// footprint. A service that does not compare footprints with supports
    /// refuses, never answering either way.
    fn assess_support_coverage(
        &self,
        request: &SupportCoverageRequest,
    ) -> Result<SupportCoverageOutcome, FreeSpaceError> {
        let _ = request;
        Err(FreeSpaceError::Unavailable(
            "this free-space service does not compare clearance footprints with supports".into(),
        ))
    }
    /// Where a path of the request's width can run in its space, and which
    /// entrances and components it comes near (see [`CirculationMap`]). A
    /// service that does not map circulation refuses.
    fn map_circulation(
        &self,
        request: &CirculationRequest,
    ) -> Result<CirculationMap, FreeSpaceError> {
        let _ = request;
        Err(FreeSpaceError::Unavailable(
            "this free-space service does not map circulation".into(),
        ))
    }
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
    pub fn assess_containment(
        &self,
        request: &ContainmentRequest,
    ) -> Result<ContainmentOutcome, FreeSpaceError> {
        let outcome = self.0.assess_containment(request)?;
        let actual = match &outcome {
            ContainmentOutcome::Inside(value) | ContainmentOutcome::Outside(value) => {
                value.request()
            }
        };
        if actual != request {
            return Err(FreeSpaceError::ResponseRequestMismatch);
        }
        Ok(outcome)
    }
}

impl FreeSpaceServiceHandle {
    /// Whether the supports' tops hold the footprint; an answer to another
    /// request is refused.
    ///
    /// # Errors
    ///
    /// The backend's refusal, or [`FreeSpaceError::ResponseRequestMismatch`].
    pub fn assess_support_coverage(
        &self,
        request: &SupportCoverageRequest,
    ) -> Result<SupportCoverageOutcome, FreeSpaceError> {
        let outcome = self.0.assess_support_coverage(request)?;
        let actual = match &outcome {
            SupportCoverageOutcome::Supported(value)
            | SupportCoverageOutcome::Unsupported(value) => value.request(),
        };
        if actual != request {
            return Err(FreeSpaceError::ResponseRequestMismatch);
        }
        Ok(outcome)
    }

    /// Maps circulation and checks that the map answers `request`.
    ///
    /// # Errors
    ///
    /// The backend's refusal, or [`FreeSpaceError::ResponseRequestMismatch`]
    /// for a map of another request.
    pub fn map_circulation(
        &self,
        request: &CirculationRequest,
    ) -> Result<CirculationMap, FreeSpaceError> {
        let map = self.0.map_circulation(request)?;
        if map.request() != request {
            return Err(FreeSpaceError::ResponseRequestMismatch);
        }
        Ok(map)
    }
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}
fn valid_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}
