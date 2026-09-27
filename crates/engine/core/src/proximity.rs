//! Source-neutral proximity evidence between two model objects.
//!
//! ADR 0004: the service measures how close two bodies come and how deeply
//! they overlap; whether that is a clash, a clearance shortfall or an
//! acceptable joint is decided by a capability against declared tolerances.
//!
//! Two properties of the measurement are carried explicitly rather than
//! folded into one number:
//!
//! - **Fidelity.** A mesh of a curved part is a tessellation. Its chords cut
//!   inside the true surface, so a separation measured on it can be wrong by up
//!   to the chord deviation of each participant. Such evidence is marked
//!   approximate and carries that deviation, so a policy can widen its
//!   comparison instead of trusting a number the geometry cannot support.
//! - **Penetration is witnessed, not computed.** Zero separation does not say
//!   whether two bodies touch or interpenetrate. The adapter reports the
//!   deepest point it found inside the other body. That is a lower bound on
//!   the true depth. Only a closed solid has an inside; a surface entering a
//!   solid is measured against the solid, and two open surfaces, which share
//!   no volume, report `None`.
//! - **Shape comparisons are intervals.** The extent of the intersection
//!   along each axis ([`OverlapExtents`]) and the Hausdorff distance between
//!   the two surfaces are reported as [`LengthInterval`]s the true values lie
//!   in, so a policy judging them against a tolerance can tell a certain
//!   answer from an open one. The intersection's volume is not measured.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::LengthInterval;

/// Why a proximity measurement could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProximityError {
    /// The adapter holds no measurable geometry for an object.
    #[error("proximity measurement is unavailable for the requested object")]
    Unavailable,
    /// The object is declared to occupy no material, such as a storey or an
    /// opening. A fact about the object, not a failure to measure it: an
    /// object whose body could not be measured is [`Self::Unavailable`].
    #[error("the requested object occupies no material")]
    NoBody,
    /// Coordinates or measured quantities are non-finite, negative or incoherent.
    #[error("proximity measurement is not finite, non-negative and coherent")]
    InvalidMeasurement,
    /// The evidence's exactness disagrees with the measured geometry's fidelity.
    #[error("proximity evidence exactness must match geometry fidelity and be reviewable")]
    EvidenceFidelityMismatch,
    /// A body cannot be measured against itself.
    #[error("proximity of an object to itself is undefined")]
    SameObject,
    /// The request's projection is not measured by this method or service.
    #[error("the requested proximity projection is not supported")]
    UnsupportedProjection,
}

/// An axis-aligned box in canonical metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3 {
    min: [f64; 3],
    max: [f64; 3],
}

impl Bounds3 {
    /// Creates a box, rejecting non-finite or inverted extents.
    pub fn try_new(min: [f64; 3], max: [f64; 3]) -> Result<Self, ProximityError> {
        let coherent = (0..3)
            .all(|axis| min[axis].is_finite() && max[axis].is_finite() && min[axis] <= max[axis]);
        if !coherent {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self { min, max })
    }
    pub fn min(&self) -> [f64; 3] {
        self.min
    }
    pub fn max(&self) -> [f64; 3] {
        self.max
    }
    /// The box grown by `margin` metres on every side.
    #[must_use]
    pub fn expanded(&self, margin: f64) -> Self {
        Self {
            min: self.min.map(|value| value - margin),
            max: self.max.map(|value| value + margin),
        }
    }
    /// Euclidean distance between the two boxes; zero when they meet.
    ///
    /// A lower bound on the separation of anything the boxes enclose, which
    /// is what makes it safe to discard pairs by it.
    pub fn gap(&self, other: &Self) -> f64 {
        (0..3)
            .map(|axis| {
                let gap = (other.min[axis] - self.max[axis])
                    .max(self.min[axis] - other.max[axis])
                    .max(0.0);
                gap * gap
            })
            .sum::<f64>()
            .sqrt()
    }
}

/// How faithfully the measured geometry represents an object's true shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GeometryFidelity {
    /// The mesh is the shape: every face of the object is planar.
    Exact,
    /// The mesh approximates curved faces; no point of the true surface lies
    /// farther than `chord_deviation_metres` from the mesh.
    Tessellated { chord_deviation_metres: f64 },
}

impl GeometryFidelity {
    /// A tessellation with a declared chord deviation.
    pub fn tessellated(chord_deviation_metres: f64) -> Result<Self, ProximityError> {
        if !chord_deviation_metres.is_finite() || chord_deviation_metres < 0.0 {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self::Tessellated {
            chord_deviation_metres,
        })
    }
    /// Whether measurements on this geometry are exact.
    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact)
    }
    /// How far the true surface may lie from the measured mesh.
    pub fn deviation_metres(&self) -> f64 {
        match self {
            Self::Exact => 0.0,
            Self::Tessellated {
                chord_deviation_metres,
            } => *chord_deviation_metres,
        }
    }
    /// Fidelity of a measurement taken between two bodies.
    ///
    /// Deviations add: each body's true surface may sit its own deviation away
    /// from its mesh, in the direction that shortens or lengthens the gap.
    #[must_use]
    pub fn combined(self, other: Self) -> Self {
        if self.is_exact() && other.is_exact() {
            Self::Exact
        } else {
            Self::Tessellated {
                chord_deviation_metres: self.deviation_metres() + other.deviation_metres(),
            }
        }
    }
}

/// An object's axis-aligned extent, for broad-phase candidate search.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectBounds {
    object: ObjectId,
    bounds: Bounds3,
    fidelity: GeometryFidelity,
}

impl ObjectBounds {
    pub fn try_new(
        object: ObjectId,
        bounds: Bounds3,
        fidelity: GeometryFidelity,
    ) -> Result<Self, ProximityError> {
        // Re-validate: `Tessellated` is constructible directly.
        let deviation = fidelity.deviation_metres();
        if !deviation.is_finite() || deviation < 0.0 {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self {
            object,
            bounds,
            fidelity,
        })
    }
    pub fn object(&self) -> &ObjectId {
        &self.object
    }
    /// The extent of the measured mesh.
    pub fn bounds(&self) -> Bounds3 {
        self.bounds
    }
    pub fn fidelity(&self) -> GeometryFidelity {
        self.fidelity
    }
    /// A box guaranteed to enclose the true body, mesh extent grown by the
    /// chord deviation. A tessellated cylinder's true surface bulges past its
    /// mesh, so the mesh box alone could discard a pair that really clashes.
    pub fn enclosing(&self) -> Bounds3 {
        self.bounds.expanded(self.fidelity.deviation_metres())
    }
}

/// The direction in which a distance between two bodies is measured.
///
/// No projection's distance bounds another's from below, so the broad phase
/// prunes each by its own box gap ([`crate::projected_candidate_pairs`]).
#[derive(Clone, Copy, Debug)]
pub enum ProximityProjection {
    /// Shortest distance between the two surfaces in space.
    Minimum3d,
    /// Plan distance between the two footprints: zero when they meet in plan.
    Horizontal,
    /// Gap between the two bodies' vertical extents (bottom to top), for
    /// bodies above or below one another. The bodies are related when their
    /// footprints overlap with positive area or, with a positive
    /// `footprint_offset_metres`, when the counterpart's footprint comes
    /// closer than the offset to the subject's (the subject's footprint grown
    /// by the offset). Unrelated bodies have no distance in this projection.
    Vertical { footprint_offset_metres: f64 },
    /// Whether the footprints overlap with positive area: distance zero when
    /// they do, none when they do not.
    PlanOverlap,
}

impl ProximityProjection {
    /// Whether the projection's offset is finite and non-negative.
    fn is_valid(&self) -> bool {
        match self {
            Self::Vertical {
                footprint_offset_metres,
            } => footprint_offset_metres.is_finite() && *footprint_offset_metres >= 0.0,
            _ => true,
        }
    }
    /// Whether a pair may have no distance at all in this projection.
    pub fn may_be_unrelated(&self) -> bool {
        matches!(self, Self::Vertical { .. } | Self::PlanOverlap)
    }
    /// The projection's spelling in evidence locators and messages.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Minimum3d => "minimum_3d",
            Self::Horizontal => "horizontal",
            Self::Vertical { .. } => "vertical",
            Self::PlanOverlap => "plan_overlap",
        }
    }
    fn key(&self) -> (u8, f64) {
        match self {
            Self::Minimum3d => (0, 0.0),
            Self::Horizontal => (1, 0.0),
            Self::Vertical {
                footprint_offset_metres,
            } => (2, *footprint_offset_metres),
            Self::PlanOverlap => (3, 0.0),
        }
    }
}

impl PartialEq for ProximityProjection {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for ProximityProjection {}
impl PartialOrd for ProximityProjection {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ProximityProjection {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let (a, a_offset) = self.key();
        let (b, b_offset) = other.key();
        a.cmp(&b).then_with(|| a_offset.total_cmp(&b_offset))
    }
}

/// A request to measure the proximity of two distinct objects.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProximityRequest {
    subject: ObjectId,
    counterpart: ObjectId,
    projection: ProximityProjection,
}

impl ProximityRequest {
    /// A request in space ([`ProximityProjection::Minimum3d`]).
    pub fn try_new(subject: ObjectId, counterpart: ObjectId) -> Result<Self, ProximityError> {
        Self::projected(subject, counterpart, ProximityProjection::Minimum3d)
    }
    /// A request measured in `projection`; a vertical offset must be finite
    /// and non-negative.
    pub fn projected(
        subject: ObjectId,
        counterpart: ObjectId,
        projection: ProximityProjection,
    ) -> Result<Self, ProximityError> {
        if subject == counterpart {
            return Err(ProximityError::SameObject);
        }
        if !projection.is_valid() {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self {
            subject,
            counterpart,
            projection,
        })
    }
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }
    pub fn counterpart(&self) -> &ObjectId {
        &self.counterpart
    }
    pub fn projection(&self) -> ProximityProjection {
        self.projection
    }
}

/// How far the intersection of two bodies reaches along each world axis.
///
/// Each axis carries a [`LengthInterval`]: the extent of the intersection's
/// axis-aligned box along it. The lower bound is witnessed (points found in
/// both bodies), the upper bound proven (the bodies' boxes overlap no
/// further), so exact geometry need not report a point. An empty
/// intersection has zero extent on every axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlapExtents {
    axes: [LengthInterval; 3],
}

impl OverlapExtents {
    /// The extents along x, y and z.
    #[must_use]
    pub fn new(x: LengthInterval, y: LengthInterval, z: LengthInterval) -> Self {
        Self { axes: [x, y, z] }
    }
    pub fn x(&self) -> LengthInterval {
        self.axes[0]
    }
    pub fn y(&self) -> LengthInterval {
        self.axes[1]
    }
    pub fn z(&self) -> LengthInterval {
        self.axes[2]
    }
    /// The lesser of the x and y extents: how far the intersection reaches in
    /// plan along its narrower axis.
    pub fn horizontal(&self) -> LengthInterval {
        let (x, y) = (self.axes[0], self.axes[1]);
        LengthInterval::try_new(
            x.lower_metres().min(y.lower_metres()),
            x.upper_metres().min(y.upper_metres()),
        )
        .unwrap_or_else(|_| unreachable!("the lesser of two intervals is an interval"))
    }
    /// The z extent.
    pub fn vertical(&self) -> LengthInterval {
        self.axes[2]
    }
    fn is_empty(&self) -> bool {
        self.axes.iter().all(|axis| axis.lower_metres() == 0.0)
    }
}

/// One body lying wholly inside the other without their surfaces meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyContainment {
    SubjectInsideCounterpart,
    CounterpartInsideSubject,
}

/// How close two bodies come and how far they overlap.
#[derive(Clone, Debug, PartialEq)]
pub struct ProximityEvidence {
    request: ProximityRequest,
    separation_metres: f64,
    penetration_metres: Option<f64>,
    plan_overlap_square_metres: f64,
    containment: Option<BodyContainment>,
    overlap_extents: Option<OverlapExtents>,
    hausdorff: Option<LengthInterval>,
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl ProximityEvidence {
    /// Rejects incoherent measurements and evidence whose exactness does not
    /// match the geometry it was measured on.
    ///
    /// - `separation_metres`: shortest distance between the two surfaces.
    /// - `penetration_metres`: depth of the deepest witnessed point of either
    ///   body inside the other; `None` when neither body is a closed solid.
    /// - `plan_overlap_square_metres`: area of the two footprints' overlap.
    pub fn try_new(
        request: ProximityRequest,
        separation_metres: f64,
        penetration_metres: Option<f64>,
        plan_overlap_square_metres: f64,
        containment: Option<BodyContainment>,
        fidelity: GeometryFidelity,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        let finite_non_negative = |v: f64| v.is_finite() && v >= 0.0;
        if !finite_non_negative(separation_metres)
            || !finite_non_negative(plan_overlap_square_metres)
            || penetration_metres.is_some_and(|depth| !finite_non_negative(depth))
            || !finite_non_negative(fidelity.deviation_metres())
        {
            return Err(ProximityError::InvalidMeasurement);
        }
        // Disjoint bodies cannot share volume. BodyContainment is the one way to be
        // apart at the surface and overlapping in volume, and it needs an
        // inside, so it cannot be claimed without a penetration measurement.
        let separated = separation_metres > 0.0;
        if (separated && containment.is_none() && penetration_metres.is_some_and(|d| d > 0.0))
            || (containment.is_some() && (!separated || penetration_metres.is_none()))
        {
            return Err(ProximityError::InvalidMeasurement);
        }
        if evidence.exact != fidelity.is_exact() || evidence.locator.trim().is_empty() {
            return Err(ProximityError::EvidenceFidelityMismatch);
        }
        // Penetration and containment are questions in space.
        if request.projection() != ProximityProjection::Minimum3d {
            return Err(ProximityError::UnsupportedProjection);
        }
        Ok(Self {
            request,
            separation_metres,
            penetration_metres,
            plan_overlap_square_metres,
            containment,
            overlap_extents: None,
            hausdorff: None,
            fidelity,
            evidence,
        })
    }

    /// Adds the extents of the bodies' intersection along each axis.
    ///
    /// Refused when no penetration was measured (two open surfaces share no
    /// volume), and when bodies apart at the surface and not contained in
    /// one another claim a non-empty intersection.
    pub fn with_overlap_extents(mut self, extents: OverlapExtents) -> Result<Self, ProximityError> {
        let disjoint = self.separation_metres > 0.0 && self.containment.is_none();
        if self.penetration_metres.is_none() || (disjoint && !extents.is_empty()) {
            return Err(ProximityError::InvalidMeasurement);
        }
        self.overlap_extents = Some(extents);
        Ok(self)
    }

    /// Adds the Hausdorff distance between the two surfaces: the farthest
    /// any point of either surface lies from the other surface.
    ///
    /// It is never smaller than the separation, so an interval lying wholly
    /// below the separation's own interval is refused.
    pub fn with_hausdorff(mut self, interval: LengthInterval) -> Result<Self, ProximityError> {
        if interval.upper_metres() < self.separation_interval_metres().0 {
            return Err(ProximityError::InvalidMeasurement);
        }
        self.hausdorff = Some(interval);
        Ok(self)
    }

    pub fn request(&self) -> &ProximityRequest {
        &self.request
    }
    /// Shortest distance between the measured surfaces.
    pub fn separation_metres(&self) -> f64 {
        self.separation_metres
    }
    /// The separation the true surfaces may have, given the fidelity.
    ///
    /// `(lower, upper)`; both equal the measured separation for exact geometry.
    pub fn separation_interval_metres(&self) -> (f64, f64) {
        let deviation = self.fidelity.deviation_metres();
        (
            (self.separation_metres - deviation).max(0.0),
            self.separation_metres + deviation,
        )
    }
    /// Witnessed interpenetration depth, a lower bound on the true depth.
    pub fn penetration_metres(&self) -> Option<f64> {
        self.penetration_metres
    }
    pub fn plan_overlap_square_metres(&self) -> f64 {
        self.plan_overlap_square_metres
    }
    pub fn containment(&self) -> Option<BodyContainment> {
        self.containment
    }
    /// Extents of the intersection along each axis; `None` when the service
    /// did not measure them or neither body is a closed solid.
    pub fn overlap_extents(&self) -> Option<OverlapExtents> {
        self.overlap_extents
    }
    /// Hausdorff distance between the two surfaces; `None` when the service
    /// did not measure it. Zero exactly when the surfaces coincide, so it is
    /// what tells a duplicate from a mere overlap.
    pub fn hausdorff_interval_metres(&self) -> Option<LengthInterval> {
        self.hausdorff
    }
    pub fn fidelity(&self) -> GeometryFidelity {
        self.fidelity
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// The distance between two bodies in a request's projection, as an interval.
///
/// `(lower, upper)` in metres, both equal for exact geometry. An upper bound
/// of infinity says the bodies may be unrelated in the projection (not above
/// one another, not overlapping in plan); a lower bound of infinity says they
/// are. Only [`ProximityProjection::may_be_unrelated`] projections may report
/// an infinite bound, and exact evidence is a point: whether exact bodies are
/// related is decided, never left open.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedDistanceEvidence {
    request: ProximityRequest,
    lower_metres: f64,
    upper_metres: f64,
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl ProjectedDistanceEvidence {
    /// Rejects incoherent intervals and evidence whose exactness does not
    /// match the geometry it was measured on.
    pub fn try_new(
        request: ProximityRequest,
        lower_metres: f64,
        upper_metres: f64,
        fidelity: GeometryFidelity,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        let deviation = fidelity.deviation_metres();
        let unbounded_allowed = request.projection().may_be_unrelated();
        let bound_ok = |value: f64| {
            value >= 0.0 && (value.is_finite() || (unbounded_allowed && value == f64::INFINITY))
        };
        if !bound_ok(lower_metres)
            || !bound_ok(upper_metres)
            || lower_metres > upper_metres
            || !deviation.is_finite()
            || deviation < 0.0
            || (fidelity.is_exact() && lower_metres < upper_metres)
        {
            return Err(ProximityError::InvalidMeasurement);
        }
        if evidence.exact != fidelity.is_exact() || evidence.locator.trim().is_empty() {
            return Err(ProximityError::EvidenceFidelityMismatch);
        }
        Ok(Self {
            request,
            lower_metres,
            upper_metres,
            fidelity,
            evidence,
        })
    }

    /// The distance interval of a measurement in space.
    pub fn from_proximity(measured: &ProximityEvidence) -> Result<Self, ProximityError> {
        let (lower, upper) = measured.separation_interval_metres();
        Self::try_new(
            measured.request().clone(),
            lower,
            upper,
            measured.fidelity(),
            measured.evidence().clone(),
        )
    }

    pub fn request(&self) -> &ProximityRequest {
        &self.request
    }
    /// `(lower, upper)` bounds on the true distance; infinite when unrelated.
    pub fn interval_metres(&self) -> (f64, f64) {
        (self.lower_metres, self.upper_metres)
    }
    pub fn fidelity(&self) -> GeometryFidelity {
        self.fidelity
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures extents and pairwise proximity of model objects.
///
/// ADR 0004: every method returns a measurement. None decides a clash.
pub trait ProximityService: Send + Sync + 'static {
    /// The extent of one object's measured geometry.
    fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError>;
    /// How close two objects come and how far they overlap, in space.
    ///
    /// A request in any other projection is refused with
    /// [`ProximityError::UnsupportedProjection`].
    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError>;
    /// The distance between two objects in the request's projection.
    ///
    /// The default answers [`ProximityProjection::Minimum3d`] from
    /// [`Self::measure_proximity`] and refuses every other projection, so a
    /// service that does not measure projections fails closed.
    fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        match request.projection() {
            ProximityProjection::Minimum3d => {
                ProjectedDistanceEvidence::from_proximity(&self.measure_proximity(request)?)
            }
            _ => Err(ProximityError::UnsupportedProjection),
        }
    }
}

/// Registry handle for a [`ProximityService`].
#[derive(Clone)]
pub struct ProximityServiceHandle(Arc<dyn ProximityService>);

impl ProximityServiceHandle {
    pub fn new(service: Arc<dyn ProximityService>) -> Self {
        Self(service)
    }
    pub fn bounds(&self, object: &ObjectId) -> Result<ObjectBounds, ProximityError> {
        self.0.bounds(object)
    }
    pub fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError> {
        self.0.measure_proximity(request)
    }
    /// The distance in the request's projection. Evidence answering another
    /// request, projection included, is refused.
    pub fn measure_distance(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProjectedDistanceEvidence, ProximityError> {
        let measured = self.0.measure_distance(request)?;
        if measured.request() != request {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(measured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }
    fn request() -> ProximityRequest {
        ProximityRequest::try_new(id("pipe"), id("wall")).unwrap()
    }
    fn exact() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "proximity:pipe:wall")
    }
    fn approximate() -> Evidence {
        Evidence {
            exact: false,
            ..exact()
        }
    }

    #[test]
    fn an_object_is_not_measured_against_itself() {
        assert_eq!(
            ProximityRequest::try_new(id("pipe"), id("pipe")),
            Err(ProximityError::SameObject)
        );
    }

    /// A tessellation cannot be presented as fact, nor exact geometry as an
    /// estimate: either would misstate what a reviewer can rely on.
    #[test]
    fn evidence_exactness_must_match_fidelity() {
        let tessellated = GeometryFidelity::tessellated(0.002).unwrap();
        assert_eq!(
            ProximityEvidence::try_new(request(), 0.1, Some(0.0), 0.0, None, tessellated, exact()),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        assert_eq!(
            ProximityEvidence::try_new(
                request(),
                0.1,
                Some(0.0),
                0.0,
                None,
                GeometryFidelity::Exact,
                approximate()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        assert!(
            ProximityEvidence::try_new(
                request(),
                0.1,
                Some(0.0),
                0.0,
                None,
                tessellated,
                approximate()
            )
            .is_ok()
        );
    }

    #[test]
    fn separated_bodies_cannot_penetrate_unless_one_contains_the_other() {
        let exact_fidelity = GeometryFidelity::Exact;
        assert_eq!(
            ProximityEvidence::try_new(
                request(),
                0.1,
                Some(0.05),
                0.0,
                None,
                exact_fidelity,
                exact()
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        assert!(
            ProximityEvidence::try_new(
                request(),
                0.1,
                Some(0.05),
                0.0,
                Some(BodyContainment::SubjectInsideCounterpart),
                exact_fidelity,
                exact()
            )
            .is_ok()
        );
        // Touching bodies are not contained in one another.
        assert_eq!(
            ProximityEvidence::try_new(
                request(),
                0.0,
                Some(0.05),
                0.0,
                Some(BodyContainment::SubjectInsideCounterpart),
                exact_fidelity,
                exact()
            ),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    #[test]
    fn tessellated_separation_widens_by_the_combined_deviation() {
        let fidelity = GeometryFidelity::tessellated(0.002)
            .unwrap()
            .combined(GeometryFidelity::tessellated(0.001).unwrap());
        let measured = ProximityEvidence::try_new(
            request(),
            0.01,
            Some(0.0),
            0.0,
            None,
            fidelity,
            approximate(),
        )
        .unwrap();
        let (lower, upper) = measured.separation_interval_metres();
        assert!((lower - 0.007).abs() < 1e-12 && (upper - 0.013).abs() < 1e-12);
    }

    #[test]
    fn a_vertical_offset_must_be_finite_and_non_negative() {
        for offset in [-0.1, f64::NAN, f64::INFINITY] {
            assert_eq!(
                ProximityRequest::projected(
                    id("pipe"),
                    id("wall"),
                    ProximityProjection::Vertical {
                        footprint_offset_metres: offset
                    }
                ),
                Err(ProximityError::InvalidMeasurement)
            );
        }
    }

    /// Penetration is a question in space; a projected request cannot carry it.
    #[test]
    fn full_proximity_evidence_is_only_measured_in_space() {
        let horizontal =
            ProximityRequest::projected(id("pipe"), id("wall"), ProximityProjection::Horizontal)
                .unwrap();
        assert_eq!(
            ProximityEvidence::try_new(
                horizontal,
                0.1,
                Some(0.0),
                0.0,
                None,
                GeometryFidelity::Exact,
                exact()
            ),
            Err(ProximityError::UnsupportedProjection)
        );
    }

    #[test]
    fn projected_distance_intervals_are_coherent() {
        let plan =
            ProximityRequest::projected(id("pipe"), id("wall"), ProximityProjection::PlanOverlap)
                .unwrap();
        let tessellated = GeometryFidelity::tessellated(0.002).unwrap();
        // Exact evidence is a point, related or not.
        assert!(
            ProjectedDistanceEvidence::try_new(
                plan.clone(),
                f64::INFINITY,
                f64::INFINITY,
                GeometryFidelity::Exact,
                exact()
            )
            .is_ok()
        );
        assert_eq!(
            ProjectedDistanceEvidence::try_new(
                plan.clone(),
                0.0,
                f64::INFINITY,
                GeometryFidelity::Exact,
                exact()
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        // A tessellation may leave the relation open, but never claim exactness.
        assert!(
            ProjectedDistanceEvidence::try_new(
                plan.clone(),
                0.0,
                f64::INFINITY,
                tessellated,
                approximate()
            )
            .is_ok()
        );
        assert_eq!(
            ProjectedDistanceEvidence::try_new(plan, 0.0, 0.0, tessellated, exact()),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        // Bodies always have a distance in space and in plan.
        assert_eq!(
            ProjectedDistanceEvidence::try_new(
                request(),
                0.0,
                f64::INFINITY,
                tessellated,
                approximate()
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            ProjectedDistanceEvidence::try_new(request(), 0.3, 0.2, tessellated, approximate()),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    /// The narrower plan axis bounds how far an intersection reaches in plan.
    #[test]
    fn horizontal_extent_is_the_narrower_plan_axis() {
        let extents = OverlapExtents::new(
            LengthInterval::try_new(0.1, 0.3).unwrap(),
            LengthInterval::try_new(0.2, 0.25).unwrap(),
            LengthInterval::exact(1.0).unwrap(),
        );
        assert_eq!(
            extents.horizontal(),
            LengthInterval::try_new(0.1, 0.25).unwrap()
        );
        assert_eq!(extents.vertical(), LengthInterval::exact(1.0).unwrap());
    }

    #[test]
    fn overlap_extents_need_a_shared_volume() {
        let extents = OverlapExtents::new(
            LengthInterval::exact(0.1).unwrap(),
            LengthInterval::exact(0.1).unwrap(),
            LengthInterval::exact(0.1).unwrap(),
        );
        let measured = |separation: f64, penetration: Option<f64>| {
            ProximityEvidence::try_new(
                request(),
                separation,
                penetration,
                0.0,
                None,
                GeometryFidelity::Exact,
                exact(),
            )
            .unwrap()
        };
        assert!(
            measured(0.0, Some(0.1))
                .with_overlap_extents(extents)
                .is_ok()
        );
        // Two open surfaces have no inside to share.
        assert_eq!(
            measured(0.0, None).with_overlap_extents(extents),
            Err(ProximityError::InvalidMeasurement)
        );
        // Bodies apart at the surface, neither inside the other, share nothing.
        assert_eq!(
            measured(0.2, Some(0.0)).with_overlap_extents(extents),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    /// No point of a surface lies closer to the other than the separation.
    #[test]
    fn hausdorff_distance_is_never_below_the_separation() {
        let measured = ProximityEvidence::try_new(
            request(),
            0.2,
            Some(0.0),
            0.0,
            None,
            GeometryFidelity::Exact,
            exact(),
        )
        .unwrap();
        assert_eq!(
            measured
                .clone()
                .with_hausdorff(LengthInterval::try_new(0.0, 0.1).unwrap()),
            Err(ProximityError::InvalidMeasurement)
        );
        assert!(
            measured
                .with_hausdorff(LengthInterval::try_new(0.2, 0.5).unwrap())
                .is_ok()
        );
    }

    #[test]
    fn box_gap_is_euclidean_and_zero_when_boxes_meet() {
        let a = Bounds3::try_new([0.0; 3], [1.0; 3]).unwrap();
        let b = Bounds3::try_new([4.0, 5.0, 0.0], [5.0, 6.0, 1.0]).unwrap();
        assert!((a.gap(&b) - 5.0).abs() < 1e-12);
        let touching = Bounds3::try_new([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]).unwrap();
        assert!(a.gap(&touching).abs() < f64::EPSILON);
        assert_eq!(
            Bounds3::try_new([1.0, 0.0, 0.0], [0.0, 1.0, 1.0]),
            Err(ProximityError::InvalidMeasurement)
        );
    }
}
