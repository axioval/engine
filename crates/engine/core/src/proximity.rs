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
//!   the true depth, and `None` when a body is not a closed solid and has no
//!   inside to test.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

/// Why a proximity measurement could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProximityError {
    /// The adapter holds no measurable geometry for an object.
    #[error("proximity measurement is unavailable for the requested object")]
    Unavailable,
    /// Coordinates or measured quantities are non-finite, negative or incoherent.
    #[error("proximity measurement is not finite, non-negative and coherent")]
    InvalidMeasurement,
    /// The evidence's exactness disagrees with the measured geometry's fidelity.
    #[error("proximity evidence exactness must match geometry fidelity and be reviewable")]
    EvidenceFidelityMismatch,
    /// A body cannot be measured against itself.
    #[error("proximity of an object to itself is undefined")]
    SameObject,
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

/// A request to measure the proximity of two distinct objects.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProximityRequest {
    subject: ObjectId,
    counterpart: ObjectId,
}

impl ProximityRequest {
    pub fn try_new(subject: ObjectId, counterpart: ObjectId) -> Result<Self, ProximityError> {
        if subject == counterpart {
            return Err(ProximityError::SameObject);
        }
        Ok(Self {
            subject,
            counterpart,
        })
    }
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }
    pub fn counterpart(&self) -> &ObjectId {
        &self.counterpart
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
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl ProximityEvidence {
    /// Rejects incoherent measurements and evidence whose exactness does not
    /// match the geometry it was measured on.
    ///
    /// - `separation_metres`: shortest distance between the two surfaces.
    /// - `penetration_metres`: depth of the deepest witnessed point of either
    ///   body inside the other; `None` when a body is not a closed solid.
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
        Ok(Self {
            request,
            separation_metres,
            penetration_metres,
            plan_overlap_square_metres,
            containment,
            fidelity,
            evidence,
        })
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
    /// How close two objects come and how far they overlap.
    fn measure_proximity(
        &self,
        request: &ProximityRequest,
    ) -> Result<ProximityEvidence, ProximityError>;
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
