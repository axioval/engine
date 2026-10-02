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
//!   answer from an open one.
//! - **Volumes are certified intervals.** The volume two closed bodies share
//!   and each body's own volume ([`IntersectionVolume`]) are
//!   [`VolumeInterval`]s sure to contain the true values, never a point
//!   estimate.
//!
//! - **Extents along the bodies' own axes.** World axes misjudge a wall at
//!   an angle: a slab edge sunk 10 mm into it reaches far along x and y. An
//!   [`OverlapAlongRequest`] names the directions to measure along (such as
//!   each body's placement axes), and [`OverlapAlongEvidence`] answers the
//!   intersection's extent along each as a [`LengthInterval`].
//!
//! A second question rides on the same service: how far a body lies from
//! one class of another body's faces ([`FaceDistanceRequest`]), signed by
//! whether the body lies inside the other. It is what cover and protrusion
//! checks measure.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::{ConvexPlanRegion, LengthInterval, MetricDirection, SignedDistanceInterval};

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
    /// by the offset), and when the counterpart lies in `direction` from the
    /// subject. Unrelated bodies have no distance in this projection.
    ///
    /// `surfaces` other than [`VerticalSurfaces::Extents`] measure between
    /// two chosen surfaces instead (see [`VerticalSurfaces`]).
    Vertical {
        footprint_offset_metres: f64,
        direction: VerticalDirection,
        surfaces: VerticalSurfaces,
    },
    /// Whether the footprints overlap with positive area: distance zero when
    /// they do, none when they do not.
    PlanOverlap,
}

/// Where the counterpart of a [`ProximityProjection::Vertical`] distance must
/// lie relative to the subject.
///
/// Compare the two vertical extents (bottom to top) end by end. A counterpart
/// lies **above** the subject unless it lies lower at both ends (its top
/// below the subject's top and its bottom below the subject's bottom), and
/// **below** unless it lies higher at both ends. A counterpart overlapping
/// the subject in height is therefore above, below or both at distance zero:
/// a pendant reaching down past a sprinkler's top is above it, a riser
/// passing the sprinkler is both. Every counterpart is above or below, so the
/// `Either` distance is the lesser of the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerticalDirection {
    /// Above or below: the gap between the two extents.
    Either,
    /// The counterpart lies above: the gap from the subject's top up to the
    /// counterpart's bottom, zero when the extents overlap.
    Above,
    /// The counterpart lies below: the gap from the subject's bottom down to
    /// the counterpart's top, zero when the extents overlap.
    Below,
}

/// Which surfaces a [`ProximityProjection::Vertical`] distance runs between.
///
/// A subject surface is a level: the subject's highest point (`Top`) or its
/// lowest (`Bottom`). A counterpart's `Top` and `Bottom` are levels too, and
/// the distance is the difference of the two levels in `direction`: a
/// counterpart level on the other side of the subject's is unrelated
/// (`Either` takes the absolute difference); the footprints must be related
/// as for [`Self::Extents`]. `Nearest` is the counterpart's surface directly
/// over or under the subject's footprint (the plan part of it overlapping
/// the footprint with positive area) nearest to the subject's level in
/// `direction`: a sprinkler's top to the underside of a sloped slab right
/// above it, not to the slab's lowest point elsewhere. A counterpart with no
/// such surface in that direction is unrelated. `Nearest` needs a footprint
/// offset of zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerticalSurfaces {
    /// The gap between the two vertical extents, zero when they overlap.
    #[default]
    Extents,
    /// From a level of the subject to a surface of the counterpart.
    Between {
        subject: SubjectSurface,
        counterpart: CounterpartSurface,
    },
}

/// The subject's level a [`VerticalSurfaces::Between`] distance starts at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubjectSurface {
    Top,
    Bottom,
}

/// The counterpart's surface a [`VerticalSurfaces::Between`] distance ends at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CounterpartSurface {
    Top,
    Bottom,
    /// The nearest surface overlapping the subject's footprint in plan.
    Nearest,
}

impl SubjectSurface {
    /// The surface's spelling in rule parameters, evidence and messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

impl CounterpartSurface {
    /// The surface's spelling in rule parameters, evidence and messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Nearest => "nearest",
        }
    }
}

impl VerticalDirection {
    /// The direction's spelling in rule parameters, evidence and messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Either => "either",
            Self::Above => "above",
            Self::Below => "below",
        }
    }
}

impl ProximityProjection {
    /// Whether the projection's offset is finite and non-negative.
    fn is_valid(&self) -> bool {
        match self {
            Self::Vertical {
                footprint_offset_metres,
                surfaces,
                ..
            } => {
                let nearest = matches!(
                    surfaces,
                    VerticalSurfaces::Between {
                        counterpart: CounterpartSurface::Nearest,
                        ..
                    }
                );
                footprint_offset_metres.is_finite()
                    && *footprint_offset_metres >= 0.0
                    && !(nearest && *footprint_offset_metres > 0.0)
            }
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
    fn key(&self) -> (u8, f64, VerticalDirection, VerticalSurfaces) {
        let none = (VerticalDirection::Either, VerticalSurfaces::Extents);
        match self {
            Self::Minimum3d => (0, 0.0, none.0, none.1),
            Self::Horizontal => (1, 0.0, none.0, none.1),
            Self::Vertical {
                footprint_offset_metres,
                direction,
                surfaces,
            } => (2, *footprint_offset_metres, *direction, *surfaces),
            Self::PlanOverlap => (3, 0.0, none.0, none.1),
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
        let (a, a_offset, a_direction, a_surfaces) = self.key();
        let (b, b_offset, b_direction, b_surfaces) = other.key();
        a.cmp(&b)
            .then_with(|| a_offset.total_cmp(&b_offset))
            .then_with(|| a_direction.cmp(&b_direction))
            .then_with(|| a_surfaces.cmp(&b_surfaces))
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

/// A request for the extents of two bodies' intersection along stated
/// directions, such as each body's own placement axes.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlapAlongRequest {
    subject: ObjectId,
    counterpart: ObjectId,
    directions: Vec<MetricDirection>,
}

impl OverlapAlongRequest {
    /// The most directions one request may name: two bodies' three axes.
    pub const MAX_DIRECTIONS: usize = 6;

    /// Refuses one object measured against itself, and no directions or
    /// more than [`Self::MAX_DIRECTIONS`].
    pub fn try_new(
        subject: ObjectId,
        counterpart: ObjectId,
        directions: Vec<MetricDirection>,
    ) -> Result<Self, ProximityError> {
        if subject == counterpart {
            return Err(ProximityError::SameObject);
        }
        if directions.is_empty() || directions.len() > Self::MAX_DIRECTIONS {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self {
            subject,
            counterpart,
            directions,
        })
    }
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }
    pub fn counterpart(&self) -> &ObjectId {
        &self.counterpart
    }
    /// The unit directions to measure along, in request order.
    pub fn directions(&self) -> &[MetricDirection] {
        &self.directions
    }
}

/// How far two bodies' intersection reaches along each direction of an
/// [`OverlapAlongRequest`].
///
/// Each extent is a [`LengthInterval`] as for [`OverlapExtents`]: its lower
/// bound witnessed (points found in both bodies), its upper bound proven
/// (the bodies' own extents along the direction overlap no further). An
/// empty intersection has zero extent along every direction. The evidence
/// is exact exactly when the geometry is.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlapAlongEvidence {
    request: OverlapAlongRequest,
    extents: Vec<LengthInterval>,
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl OverlapAlongEvidence {
    /// Refuses one extent too many or too few, and evidence whose
    /// exactness does not match the geometry.
    pub fn try_new(
        request: OverlapAlongRequest,
        extents: Vec<LengthInterval>,
        fidelity: GeometryFidelity,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        if extents.len() != request.directions.len() {
            return Err(ProximityError::InvalidMeasurement);
        }
        if evidence.exact != fidelity.is_exact() || evidence.locator.trim().is_empty() {
            return Err(ProximityError::EvidenceFidelityMismatch);
        }
        Ok(Self {
            request,
            extents,
            fidelity,
            evidence,
        })
    }
    pub fn request(&self) -> &OverlapAlongRequest {
        &self.request
    }
    /// The extent along each requested direction, in request order.
    pub fn extents(&self) -> &[LengthInterval] {
        &self.extents
    }
    pub fn fidelity(&self) -> GeometryFidelity {
        self.fidelity
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// One body lying wholly inside the other without their surfaces meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyContainment {
    SubjectInsideCounterpart,
    CounterpartInsideSubject,
}

/// Bounds on a volume in cubic metres: finite, non-negative, and sure to
/// contain the true value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeInterval {
    lower_cubic_metres: f64,
    upper_cubic_metres: f64,
}

impl VolumeInterval {
    /// Validates inclusive lower and upper bounds.
    pub fn try_new(
        lower_cubic_metres: f64,
        upper_cubic_metres: f64,
    ) -> Result<Self, ProximityError> {
        let valid = |value: f64| value.is_finite() && value >= 0.0;
        if !valid(lower_cubic_metres)
            || !valid(upper_cubic_metres)
            || lower_cubic_metres > upper_cubic_metres
        {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self {
            lower_cubic_metres,
            upper_cubic_metres,
        })
    }
    /// A volume known exactly.
    pub fn exact(cubic_metres: f64) -> Result<Self, ProximityError> {
        Self::try_new(cubic_metres, cubic_metres)
    }
    pub fn lower_cubic_metres(&self) -> f64 {
        self.lower_cubic_metres
    }
    pub fn upper_cubic_metres(&self) -> f64 {
        self.upper_cubic_metres
    }
    /// Whether the interval proves one value.
    #[allow(clippy::float_cmp)]
    pub fn is_exact(&self) -> bool {
        self.lower_cubic_metres == self.upper_cubic_metres
    }
    /// `(lower, upper)` bounds on this volume's share of `whole`, for a
    /// part that lies within the whole: outward rounded and clamped to
    /// `[0, 1]`, so the true share always lies inside.
    pub fn share_of(self, whole: Self) -> (f64, f64) {
        let lower = if whole.upper_cubic_metres > 0.0 {
            (self.lower_cubic_metres / whole.upper_cubic_metres).next_down()
        } else {
            0.0
        };
        let upper = if whole.lower_cubic_metres > 0.0 {
            (self.upper_cubic_metres / whole.lower_cubic_metres).next_up()
        } else {
            1.0
        };
        let upper = upper.clamp(0.0, 1.0);
        (lower.clamp(0.0, upper), upper)
    }
}

/// The volume two closed bodies share, with each body's own volume.
///
/// Every value is a certified [`VolumeInterval`]. The shared volume cannot
/// exceed either body's, so bounds claiming it must are refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntersectionVolume {
    shared: VolumeInterval,
    subject: VolumeInterval,
    counterpart: VolumeInterval,
}

impl IntersectionVolume {
    /// The shared volume and the volumes of the request's subject and
    /// counterpart.
    pub fn try_new(
        shared: VolumeInterval,
        subject: VolumeInterval,
        counterpart: VolumeInterval,
    ) -> Result<Self, ProximityError> {
        let most = subject
            .upper_cubic_metres
            .min(counterpart.upper_cubic_metres);
        if shared.lower_cubic_metres > most {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self {
            shared,
            subject,
            counterpart,
        })
    }
    /// The volume both bodies occupy.
    pub fn shared(&self) -> VolumeInterval {
        self.shared
    }
    /// The subject's own volume.
    pub fn subject(&self) -> VolumeInterval {
        self.subject
    }
    /// The counterpart's own volume.
    pub fn counterpart(&self) -> VolumeInterval {
        self.counterpart
    }
    /// The volume of the smaller body: the lesser of the two volumes.
    pub fn smaller(&self) -> VolumeInterval {
        VolumeInterval {
            lower_cubic_metres: self
                .subject
                .lower_cubic_metres
                .min(self.counterpart.lower_cubic_metres),
            upper_cubic_metres: self
                .subject
                .upper_cubic_metres
                .min(self.counterpart.upper_cubic_metres),
        }
    }
    /// `(lower, upper)` bounds on the shared volume's share of the smaller
    /// body, between zero and one: one when one body lies wholly in the
    /// other. Rounded outward, so the true ratio always lies inside.
    pub fn ratio_of_smaller(&self) -> (f64, f64) {
        self.shared.share_of(self.smaller())
    }
}

/// The certified volume one closed body encloses.
///
/// A tessellated body's volume is widened by the band its chord deviation
/// allows, so the interval always holds the true volume; only an exact mesh
/// may carry exact evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyVolume {
    object: ObjectId,
    volume: VolumeInterval,
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl BodyVolume {
    /// The volume of `object`. Exact evidence needs exact geometry, and
    /// every piece of evidence needs a reviewable locator.
    pub fn try_new(
        object: ObjectId,
        volume: VolumeInterval,
        fidelity: GeometryFidelity,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        if evidence.exact && !fidelity.is_exact() || evidence.locator.trim().is_empty() {
            return Err(ProximityError::EvidenceFidelityMismatch);
        }
        Ok(Self {
            object,
            volume,
            fidelity,
            evidence,
        })
    }
    /// The measured object.
    pub fn object(&self) -> &ObjectId {
        &self.object
    }
    /// Bounds on the enclosed volume.
    pub fn volume(&self) -> VolumeInterval {
        self.volume
    }
    pub fn fidelity(&self) -> GeometryFidelity {
        self.fidelity
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// An exact boundary a backend registered for an object, handed out beside
/// its surface ([`BodySurface::exact_boundary`]) so a service of another
/// session can measure its own exact boundary against it.
///
/// Opaque to the engine, which never names a geometry kernel's types: only
/// a backend that knows the boundary's type reads it
/// ([`Self::downcast_ref`]); any other measures the mesh or refuses. Two
/// handles are equal when they share one boundary.
#[derive(Clone)]
pub struct ExactBoundaryHandle(Arc<dyn std::any::Any + Send + Sync>);

impl ExactBoundaryHandle {
    /// Wraps a backend's boundary.
    pub fn new<T: std::any::Any + Send + Sync>(boundary: Arc<T>) -> Self {
        Self(boundary)
    }
    /// The boundary, when it is a `T`.
    #[must_use]
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl std::fmt::Debug for ExactBoundaryHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExactBoundaryHandle(..)")
    }
}

impl PartialEq for ExactBoundaryHandle {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

/// One object's measured surface: a triangle mesh in world metres, and the
/// object's exact boundary where its backend registered one.
///
/// What [`ProximityService::body_surface`] hands out so another service,
/// possibly of another session, can measure its own body against it
/// ([`SurfaceDistanceRequest`]). Positions are in the session's world
/// coordinates, so two sessions' surfaces compare where they stand.
#[derive(Clone, Debug, PartialEq)]
pub struct BodySurface {
    object: ObjectId,
    positions: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    fidelity: GeometryFidelity,
    exact_boundary: Option<ExactBoundaryHandle>,
}

impl BodySurface {
    /// The surface of `object`. Refuses a surface without triangles, a
    /// corner naming no position, a non-finite position, or an invalid
    /// chord deviation.
    pub fn try_new(
        object: ObjectId,
        positions: Vec<[f64; 3]>,
        triangles: Vec<[u32; 3]>,
        fidelity: GeometryFidelity,
    ) -> Result<Self, ProximityError> {
        let deviation = fidelity.deviation_metres();
        let coherent = !triangles.is_empty()
            && deviation.is_finite()
            && deviation >= 0.0
            && positions.iter().flatten().all(|value| value.is_finite())
            && triangles.iter().flatten().all(|corner| {
                usize::try_from(*corner).is_ok_and(|corner| corner < positions.len())
            });
        if !coherent {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self {
            object,
            positions,
            triangles,
            fidelity,
            exact_boundary: None,
        })
    }
    /// The surface with the object's exact boundary, in the same world
    /// coordinates: the mesh may be a tessellation of it, the boundary is
    /// the body itself.
    #[must_use]
    pub fn with_exact_boundary(mut self, boundary: ExactBoundaryHandle) -> Self {
        self.exact_boundary = Some(boundary);
        self
    }
    /// The object's exact boundary, when its backend registered one.
    pub fn exact_boundary(&self) -> Option<&ExactBoundaryHandle> {
        self.exact_boundary.as_ref()
    }
    /// The object the surface bounds.
    pub fn object(&self) -> &ObjectId {
        &self.object
    }
    /// Vertex positions in world metres.
    pub fn positions(&self) -> &[[f64; 3]] {
        &self.positions
    }
    /// Triangles as three indices into [`Self::positions`].
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }
    /// How faithfully the mesh represents the object's true surface.
    pub fn fidelity(&self) -> GeometryFidelity {
        self.fidelity
    }
}

/// How far one body's surface lies from another's: the Hausdorff distance
/// between `subject` and the `counterpart` surface, refined to within
/// `accuracy_metres` where the backend can.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDistanceRequest {
    subject: ObjectId,
    counterpart: Arc<BodySurface>,
    accuracy_metres: f64,
}

impl SurfaceDistanceRequest {
    /// Measures `subject` against `counterpart`. Refuses a negative or
    /// non-finite accuracy, and a counterpart that is the subject's own
    /// surface.
    pub fn try_new(
        subject: ObjectId,
        counterpart: Arc<BodySurface>,
        accuracy_metres: f64,
    ) -> Result<Self, ProximityError> {
        if !accuracy_metres.is_finite() || accuracy_metres < 0.0 {
            return Err(ProximityError::InvalidMeasurement);
        }
        if counterpart.object() == &subject {
            return Err(ProximityError::SameObject);
        }
        Ok(Self {
            subject,
            counterpart,
            accuracy_metres,
        })
    }
    /// The object the service measures from its own geometry.
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }
    /// The surface it is measured against.
    pub fn counterpart(&self) -> &BodySurface {
        &self.counterpart
    }
    /// The interval width the request asks for; a backend may stop wider.
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }
}

/// A one-sided Hausdorff distance, `max over a in A of min over b in B of
/// |a - b|`, as an interval with its witness: `from`, a point of `A` at
/// least the lower bound from every point of `B`, and `to`, the nearest
/// point found to it on `B`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectedDistance {
    interval: LengthInterval,
    from: [f64; 3],
    to: [f64; 3],
}

impl DirectedDistance {
    /// Refuses a non-finite witness.
    pub fn try_new(
        interval: LengthInterval,
        from: [f64; 3],
        to: [f64; 3],
    ) -> Result<Self, ProximityError> {
        if !from.iter().chain(&to).all(|value| value.is_finite()) {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self { interval, from, to })
    }
    /// Bounds on the distance.
    pub fn interval(&self) -> LengthInterval {
        self.interval
    }
    /// The witness on the side measured from, in world metres.
    pub fn from(&self) -> [f64; 3] {
        self.from
    }
    /// Its nearest point found on the other side, in world metres.
    pub fn to(&self) -> [f64; 3] {
        self.to
    }
}

/// Which way a [`DirectedDistance`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceDirection {
    /// From the subject to the counterpart.
    FromSubject,
    /// From the counterpart to the subject.
    FromCounterpart,
}

/// What a [`SurfaceDistanceEvidence`] was measured between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceBasis {
    /// The two exact meshes.
    Mesh,
    /// The two exact boundaries ([`BodySurface::exact_boundary`]), whatever
    /// the meshes' fidelity.
    ExactBoundary,
}

/// The certified Hausdorff distance between a subject's surface and a
/// counterpart surface.
///
/// Only exact surfaces are measured: two exact meshes, or two exact
/// boundaries ([`SurfaceBasis`]). A tessellation bounds how far its true
/// surface may lie from the mesh, not how far the mesh may lie from the true
/// surface, so no distance between tessellations is certified: mesh
/// evidence for a tessellated counterpart, boundary evidence for a
/// counterpart without an exact boundary, and evidence that is not exact
/// are refused.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDistanceEvidence {
    request: SurfaceDistanceRequest,
    forward: DirectedDistance,
    backward: DirectedDistance,
    evidence: Evidence,
    basis: SurfaceBasis,
}

impl SurfaceDistanceEvidence {
    /// Measured between the two meshes: `forward` runs from the subject to
    /// the counterpart, `backward` the other way. Refuses a tessellated
    /// counterpart and evidence that is not exact or has no reviewable
    /// locator.
    pub fn try_new(
        request: SurfaceDistanceRequest,
        forward: DirectedDistance,
        backward: DirectedDistance,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        Self::measured(SurfaceBasis::Mesh, request, (forward, backward), evidence)
    }
    /// Measured between the two exact boundaries, as [`Self::try_new`]
    /// otherwise. Refuses a counterpart without an exact boundary and
    /// evidence that is not exact or has no reviewable locator; the meshes'
    /// fidelity does not enter.
    pub fn try_from_boundaries(
        request: SurfaceDistanceRequest,
        forward: DirectedDistance,
        backward: DirectedDistance,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        Self::measured(
            SurfaceBasis::ExactBoundary,
            request,
            (forward, backward),
            evidence,
        )
    }
    fn measured(
        basis: SurfaceBasis,
        request: SurfaceDistanceRequest,
        (forward, backward): (DirectedDistance, DirectedDistance),
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        let counterpart = request.counterpart();
        let certified = match basis {
            SurfaceBasis::Mesh => counterpart.fidelity().is_exact(),
            SurfaceBasis::ExactBoundary => counterpart.exact_boundary().is_some(),
        };
        if !evidence.exact || !certified || evidence.locator.trim().is_empty() {
            return Err(ProximityError::EvidenceFidelityMismatch);
        }
        Ok(Self {
            request,
            forward,
            backward,
            evidence,
            basis,
        })
    }
    pub fn request(&self) -> &SurfaceDistanceRequest {
        &self.request
    }
    /// What the distance was measured between.
    pub fn basis(&self) -> SurfaceBasis {
        self.basis
    }
    /// How far the subject strays from the counterpart.
    pub fn forward(&self) -> DirectedDistance {
        self.forward
    }
    /// How far the counterpart strays from the subject.
    pub fn backward(&self) -> DirectedDistance {
        self.backward
    }
    /// The two-sided distance, `max(forward, backward)`: each bound is the
    /// larger of the two sides'.
    pub fn distance(&self) -> LengthInterval {
        let (forward, backward) = (self.forward.interval, self.backward.interval);
        LengthInterval::try_new(
            forward.lower_metres().max(backward.lower_metres()),
            forward.upper_metres().max(backward.upper_metres()),
        )
        .unwrap_or(forward)
    }
    /// The witness of the two-sided distance: the side with the larger
    /// lower bound, the subject's on a tie.
    pub fn witness(&self) -> (SurfaceDirection, DirectedDistance) {
        if self.backward.interval.lower_metres() > self.forward.interval.lower_metres() {
            (SurfaceDirection::FromCounterpart, self.backward)
        } else {
            (SurfaceDirection::FromSubject, self.forward)
        }
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
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
    volume: Option<IntersectionVolume>,
    certified_separation: Option<LengthInterval>,
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
            volume: None,
            certified_separation: None,
            fidelity,
            evidence,
        })
    }

    /// Adds a certified interval on the separation of the true surfaces,
    /// measured on their exact boundaries rather than on the meshes.
    ///
    /// Both it and the fidelity's interval hold the true separation, so
    /// [`Self::separation_interval_metres`] becomes their intersection. An
    /// interval missing the fidelity's is refused: the two measurements
    /// cannot describe one pair of bodies. Attach it before the Hausdorff
    /// distance, which is checked against the separation.
    pub fn with_certified_separation(
        mut self,
        interval: LengthInterval,
    ) -> Result<Self, ProximityError> {
        let (lower, upper) = self.separation_interval_metres();
        let lower = lower.max(interval.lower_metres());
        let upper = upper.min(interval.upper_metres());
        if lower > upper {
            return Err(ProximityError::InvalidMeasurement);
        }
        self.certified_separation = Some(
            LengthInterval::try_new(lower, upper)
                .map_err(|_| ProximityError::InvalidMeasurement)?,
        );
        Ok(self)
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

    /// Adds the certified volume the two bodies share, with their own
    /// volumes.
    ///
    /// Refused when no penetration was measured (open surfaces enclose no
    /// volume), when bodies apart at the surface and not contained in one
    /// another claim a shared volume, and when a body said to lie inside
    /// the other cannot share its whole volume with it.
    pub fn with_intersection_volume(
        mut self,
        volume: IntersectionVolume,
    ) -> Result<Self, ProximityError> {
        let disjoint = self.separation_metres > 0.0 && self.containment.is_none();
        let shared = volume.shared();
        let whole = |inner: VolumeInterval| {
            shared.upper_cubic_metres >= inner.lower_cubic_metres
                && shared.lower_cubic_metres <= inner.upper_cubic_metres
        };
        let contained = match self.containment {
            Some(BodyContainment::SubjectInsideCounterpart) => whole(volume.subject()),
            Some(BodyContainment::CounterpartInsideSubject) => whole(volume.counterpart()),
            None => true,
        };
        if self.penetration_metres.is_none()
            || (disjoint && shared.lower_cubic_metres > 0.0)
            || !contained
        {
            return Err(ProximityError::InvalidMeasurement);
        }
        self.volume = Some(volume);
        Ok(self)
    }

    pub fn request(&self) -> &ProximityRequest {
        &self.request
    }
    /// Shortest distance between the measured surfaces.
    pub fn separation_metres(&self) -> f64 {
        self.separation_metres
    }
    /// The separation the true surfaces may have, given the fidelity, or
    /// the certified interval where one was attached.
    ///
    /// `(lower, upper)`; both equal the measured separation for exact geometry.
    pub fn separation_interval_metres(&self) -> (f64, f64) {
        if let Some(certified) = self.certified_separation {
            return (certified.lower_metres(), certified.upper_metres());
        }
        let deviation = self.fidelity.deviation_metres();
        (
            (self.separation_metres - deviation).max(0.0),
            self.separation_metres + deviation,
        )
    }
    /// The certified separation interval, when the service measured the
    /// exact boundaries; `None` when only the mesh was measured.
    pub fn certified_separation(&self) -> Option<LengthInterval> {
        self.certified_separation
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
    /// The certified volume the bodies share and their own volumes; `None`
    /// when the service did not measure it or a body is not a closed solid.
    pub fn intersection_volume(&self) -> Option<IntersectionVolume> {
        self.volume
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

/// A request for the plan distance from a stated convex region to an
/// object's footprint, such as from the floor area a door leaf sweeps to a
/// column.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionDistanceRequest {
    region: ConvexPlanRegion,
    counterpart: ObjectId,
}

impl RegionDistanceRequest {
    pub fn new(region: ConvexPlanRegion, counterpart: ObjectId) -> Self {
        Self {
            region,
            counterpart,
        }
    }
    pub fn region(&self) -> &ConvexPlanRegion {
        &self.region
    }
    pub fn counterpart(&self) -> &ObjectId {
        &self.counterpart
    }
}

/// The plan distance from a region to an object's footprint, as an
/// interval: zero when they meet in plan, a point exactly when the
/// counterpart's geometry is exact.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionDistanceEvidence {
    request: RegionDistanceRequest,
    lower_metres: f64,
    upper_metres: f64,
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl RegionDistanceEvidence {
    /// Rejects incoherent intervals, evidence whose exactness does not match
    /// the counterpart's geometry, and evidence from another source than the
    /// counterpart's.
    pub fn try_new(
        request: RegionDistanceRequest,
        lower_metres: f64,
        upper_metres: f64,
        fidelity: GeometryFidelity,
        evidence: Evidence,
    ) -> Result<Self, ProximityError> {
        let bound_ok = |value: f64| value.is_finite() && value >= 0.0;
        if !bound_ok(lower_metres)
            || !bound_ok(upper_metres)
            || lower_metres > upper_metres
            || (fidelity.is_exact() && lower_metres < upper_metres)
        {
            return Err(ProximityError::InvalidMeasurement);
        }
        if evidence.exact != fidelity.is_exact()
            || evidence.locator.trim().is_empty()
            || evidence.source != request.counterpart().source
        {
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
    pub fn request(&self) -> &RegionDistanceRequest {
        &self.request
    }
    /// `(lower, upper)` bounds on the true plan distance.
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

/// Which faces of a body a [`FaceDistanceRequest`] measures to, by the
/// direction of their outward normal.
///
/// A face is **top** when its outward normal points upwards within 45° of
/// vertical (its z component is at least √½), **bottom** when it points
/// downwards within 45°, and a **side** face otherwise. `Any` takes every
/// face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaceClass {
    Top,
    Side,
    Bottom,
    Any,
}

impl FaceClass {
    /// The class's spelling in rule parameters, evidence and messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Side => "side",
            Self::Bottom => "bottom",
            Self::Any => "any",
        }
    }
    /// Parses [`Self::name`].
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Top, Self::Side, Self::Bottom, Self::Any]
            .into_iter()
            .find(|class| class.name() == name)
    }
}

/// Why a face distance could not be measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FaceDistanceError {
    /// The service does not measure face distances.
    #[error("face distances are not measured by this service")]
    Unsupported,
    /// The adapter holds no measurable geometry for an object.
    #[error("face distance is unavailable for the requested object")]
    Unavailable,
    /// An object is declared to occupy no material.
    #[error("the requested object occupies no material")]
    NoBody,
    /// The host is not a closed solid, so it has no inside and no outward
    /// normals.
    #[error("the host body is not a closed solid")]
    NotClosed,
    /// The host has no face of the requested class.
    #[error("the host body has no face of the requested class")]
    NoFaces,
    /// The host is a tessellation: its chords do not state the true faces'
    /// normals, so no face class can be read from them.
    #[error("the host body is tessellated, so its face classes are unknown")]
    InexactHost,
    /// A face of the host lies at the 45° boundary between two classes
    /// within rounding, so which class it belongs to is undecided.
    #[error("a face of the host lies on the boundary between two face classes")]
    AmbiguousFace,
    /// Coordinates or measured quantities are non-finite or incoherent.
    #[error("face distance is not finite and coherent")]
    InvalidMeasurement,
    /// The evidence's exactness disagrees with the measured geometry's fidelity.
    #[error("face distance evidence exactness must match geometry fidelity and be reviewable")]
    EvidenceFidelityMismatch,
    /// A body cannot be measured against its own faces.
    #[error("the face distance of an object to itself is undefined")]
    SameObject,
}

/// A request for the signed distance from one body to one class of another
/// body's faces.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FaceDistanceRequest {
    body: ObjectId,
    host: ObjectId,
    faces: FaceClass,
}

impl FaceDistanceRequest {
    /// The distance from `body` to the `faces` of `host`.
    pub fn try_new(
        body: ObjectId,
        host: ObjectId,
        faces: FaceClass,
    ) -> Result<Self, FaceDistanceError> {
        if body == host {
            return Err(FaceDistanceError::SameObject);
        }
        Ok(Self { body, host, faces })
    }
    /// The body measured from.
    pub fn body(&self) -> &ObjectId {
        &self.body
    }
    /// The body whose faces are measured to.
    pub fn host(&self) -> &ObjectId {
        &self.host
    }
    pub fn faces(&self) -> FaceClass {
        self.faces
    }
}

/// The signed distance from a body to a class of a host body's faces.
///
/// With `F` the host's faces of the class, each point `p` of the body has
/// the signed distance `+d(p, F)` when it lies in the host (boundary
/// included) and `-d(p, F)` when it lies outside. The body's distance is the
/// least over its points:
///
/// - **positive**: the whole body lies in the host, that far from the faces
///   (a cover);
/// - **negative**: part of the body lies outside the host, the farthest of
///   it that far from the faces (a protrusion);
/// - **zero**: the body reaches the faces.
///
/// It is a [`SignedDistanceInterval`] sure to contain the true value, a
/// point only when the measurement proves one.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceDistanceEvidence {
    request: FaceDistanceRequest,
    signed: SignedDistanceInterval,
    fidelity: GeometryFidelity,
    evidence: Evidence,
}

impl FaceDistanceEvidence {
    /// Rejects evidence whose exactness does not match the geometry it was
    /// measured on.
    pub fn try_new(
        request: FaceDistanceRequest,
        signed: SignedDistanceInterval,
        fidelity: GeometryFidelity,
        evidence: Evidence,
    ) -> Result<Self, FaceDistanceError> {
        let deviation = fidelity.deviation_metres();
        if !deviation.is_finite() || deviation < 0.0 {
            return Err(FaceDistanceError::InvalidMeasurement);
        }
        if evidence.exact != fidelity.is_exact() || evidence.locator.trim().is_empty() {
            return Err(FaceDistanceError::EvidenceFidelityMismatch);
        }
        Ok(Self {
            request,
            signed,
            fidelity,
            evidence,
        })
    }
    pub fn request(&self) -> &FaceDistanceRequest {
        &self.request
    }
    /// Bounds on the signed distance: positive inside the host, negative
    /// outside.
    pub fn signed(&self) -> SignedDistanceInterval {
        self.signed
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
    /// The signed distance from one body to a class of another's faces.
    ///
    /// The default refuses with [`FaceDistanceError::Unsupported`], so a
    /// service that does not measure face distances fails closed.
    fn measure_face_distance(
        &self,
        request: &FaceDistanceRequest,
    ) -> Result<FaceDistanceEvidence, FaceDistanceError> {
        let _ = request;
        Err(FaceDistanceError::Unsupported)
    }
    /// The plan distance from a stated region to an object's footprint.
    ///
    /// The default refuses with [`ProximityError::UnsupportedProjection`],
    /// so a service that does not measure regions fails closed.
    fn measure_region_distance(
        &self,
        request: &RegionDistanceRequest,
    ) -> Result<RegionDistanceEvidence, ProximityError> {
        let _ = request;
        Err(ProximityError::UnsupportedProjection)
    }
    /// The extents of two bodies' intersection along stated directions.
    ///
    /// The default refuses with [`ProximityError::UnsupportedProjection`],
    /// so a service that does not measure them fails closed.
    fn measure_overlap_along(
        &self,
        request: &OverlapAlongRequest,
    ) -> Result<OverlapAlongEvidence, ProximityError> {
        let _ = request;
        Err(ProximityError::UnsupportedProjection)
    }

    /// The volume one closed body encloses.
    ///
    /// The default refuses with [`ProximityError::Unavailable`], so a
    /// service that does not measure volumes fails closed.
    fn measure_body_volume(&self, object: &ObjectId) -> Result<BodyVolume, ProximityError> {
        let _ = object;
        Err(ProximityError::Unavailable)
    }

    /// One object's measured surface in world metres, for another service
    /// to measure against ([`Self::measure_surface_distance`]).
    ///
    /// The default refuses with [`ProximityError::Unavailable`], so a
    /// service that does not hand out surfaces fails closed.
    fn body_surface(&self, object: &ObjectId) -> Result<BodySurface, ProximityError> {
        let _ = object;
        Err(ProximityError::Unavailable)
    }

    /// The certified Hausdorff distance between the subject's surface and
    /// the request's counterpart surface, which may come from another
    /// session. Only exact surfaces are measured: where the subject and the
    /// counterpart both have an exact boundary a service may measure between
    /// the boundaries ([`SurfaceBasis::ExactBoundary`]); otherwise it refuses
    /// a tessellated subject or counterpart rather than widen a distance it
    /// cannot certify.
    ///
    /// The default refuses with [`ProximityError::Unavailable`], so a
    /// service that does not measure surface distances fails closed.
    fn measure_surface_distance(
        &self,
        request: &SurfaceDistanceRequest,
    ) -> Result<SurfaceDistanceEvidence, ProximityError> {
        let _ = request;
        Err(ProximityError::Unavailable)
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
    /// The signed face distance. Evidence answering another request is
    /// refused.
    pub fn measure_face_distance(
        &self,
        request: &FaceDistanceRequest,
    ) -> Result<FaceDistanceEvidence, FaceDistanceError> {
        let measured = self.0.measure_face_distance(request)?;
        if measured.request() != request {
            return Err(FaceDistanceError::InvalidMeasurement);
        }
        Ok(measured)
    }
    /// The intersection's extents along stated directions. Evidence
    /// answering another request is refused.
    pub fn measure_overlap_along(
        &self,
        request: &OverlapAlongRequest,
    ) -> Result<OverlapAlongEvidence, ProximityError> {
        let measured = self.0.measure_overlap_along(request)?;
        if measured.request() != request {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(measured)
    }
    /// The volume `object` encloses. A volume naming another object answers
    /// a different question and is refused.
    pub fn measure_body_volume(&self, object: &ObjectId) -> Result<BodyVolume, ProximityError> {
        let measured = self.0.measure_body_volume(object)?;
        if measured.object() != object {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(measured)
    }
    /// The surface of `object`. A surface of another object is refused.
    pub fn body_surface(&self, object: &ObjectId) -> Result<BodySurface, ProximityError> {
        let surface = self.0.body_surface(object)?;
        if surface.object() != object {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(surface)
    }
    /// The certified surface distance. Evidence answering another request
    /// is refused.
    pub fn measure_surface_distance(
        &self,
        request: &SurfaceDistanceRequest,
    ) -> Result<SurfaceDistanceEvidence, ProximityError> {
        let measured = self.0.measure_surface_distance(request)?;
        if measured.request() != request {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(measured)
    }
    /// The plan distance from a region. Evidence answering another request
    /// is refused.
    pub fn measure_region_distance(
        &self,
        request: &RegionDistanceRequest,
    ) -> Result<RegionDistanceEvidence, ProximityError> {
        let measured = self.0.measure_region_distance(request)?;
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

    /// A service answering one volume whatever it is asked.
    struct OneVolume(BodyVolume);

    #[test]
    fn a_body_volume_keeps_its_exactness_honest_and_names_its_object() {
        let tessellated = GeometryFidelity::Tessellated {
            chord_deviation_metres: 0.01,
        };
        let volume = VolumeInterval::try_new(1.0, 1.1).unwrap();
        assert_eq!(
            BodyVolume::try_new(id("wall"), volume, tessellated, exact()),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        let measured = BodyVolume::try_new(id("wall"), volume, tessellated, approximate()).unwrap();
        let handle = ProximityServiceHandle::new(Arc::new(OneVolume(measured.clone())));
        assert_eq!(handle.measure_body_volume(&id("wall")), Ok(measured));
        assert_eq!(
            handle.measure_body_volume(&id("pipe")),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    impl ProximityService for OneVolume {
        fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
            Err(ProximityError::Unavailable)
        }
        fn measure_proximity(
            &self,
            _: &ProximityRequest,
        ) -> Result<ProximityEvidence, ProximityError> {
            Err(ProximityError::Unavailable)
        }
        fn measure_body_volume(&self, _: &ObjectId) -> Result<BodyVolume, ProximityError> {
            Ok(self.0.clone())
        }
    }
    fn triangle(object: &str, fidelity: GeometryFidelity) -> BodySurface {
        BodySurface::try_new(
            id(object),
            vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
            fidelity,
        )
        .unwrap()
    }

    fn directed(lower: f64, upper: f64) -> DirectedDistance {
        DirectedDistance::try_new(
            LengthInterval::try_new(lower, upper).unwrap(),
            [lower, 0.0, 0.0],
            [0.0; 3],
        )
        .unwrap()
    }

    /// A service answering one surface and one distance whatever it is asked.
    struct OneSurface(BodySurface, SurfaceDistanceEvidence);

    impl ProximityService for OneSurface {
        fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
            Err(ProximityError::Unavailable)
        }
        fn measure_proximity(
            &self,
            _: &ProximityRequest,
        ) -> Result<ProximityEvidence, ProximityError> {
            Err(ProximityError::Unavailable)
        }
        fn body_surface(&self, _: &ObjectId) -> Result<BodySurface, ProximityError> {
            Ok(self.0.clone())
        }
        fn measure_surface_distance(
            &self,
            _: &SurfaceDistanceRequest,
        ) -> Result<SurfaceDistanceEvidence, ProximityError> {
            Ok(self.1.clone())
        }
    }

    #[test]
    fn a_surface_refuses_corners_it_does_not_have() {
        assert_eq!(
            BodySurface::try_new(
                id("wall"),
                vec![[0.0; 3]],
                vec![[0, 1, 2]],
                GeometryFidelity::Exact
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            BodySurface::try_new(id("wall"), vec![], vec![], GeometryFidelity::Exact),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            BodySurface::try_new(
                id("wall"),
                vec![[f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![[0, 1, 2]],
                GeometryFidelity::Exact
            ),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    #[test]
    fn a_surface_distance_is_certified_on_exact_surfaces_only() {
        let surface = Arc::new(triangle("revised", GeometryFidelity::Exact));
        let request = SurfaceDistanceRequest::try_new(id("base"), surface.clone(), 0.001).unwrap();
        assert_eq!(
            SurfaceDistanceRequest::try_new(id("revised"), surface.clone(), 0.001),
            Err(ProximityError::SameObject)
        );
        assert_eq!(
            SurfaceDistanceRequest::try_new(id("base"), surface, -1.0),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            SurfaceDistanceEvidence::try_new(
                request.clone(),
                directed(0.1, 0.2),
                directed(0.0, 0.1),
                approximate()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        let tessellated = Arc::new(triangle(
            "revised",
            GeometryFidelity::tessellated(0.001).unwrap(),
        ));
        let curved = SurfaceDistanceRequest::try_new(id("base"), tessellated, 0.001).unwrap();
        assert_eq!(
            SurfaceDistanceEvidence::try_new(
                curved,
                directed(0.1, 0.2),
                directed(0.0, 0.1),
                exact()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );

        // The two-sided distance takes each bound from the larger side, and
        // its witness from the side with the larger lower bound.
        let measured = SurfaceDistanceEvidence::try_new(
            request.clone(),
            directed(0.1, 0.15),
            directed(0.12, 0.13),
            exact(),
        )
        .unwrap();
        assert_eq!(
            measured.distance(),
            LengthInterval::try_new(0.12, 0.15).unwrap()
        );
        assert_eq!(
            measured.witness(),
            (SurfaceDirection::FromCounterpart, directed(0.12, 0.13))
        );

        let handle = ProximityServiceHandle::new(Arc::new(OneSurface(
            triangle("revised", GeometryFidelity::Exact),
            measured.clone(),
        )));
        assert!(handle.body_surface(&id("revised")).is_ok());
        assert_eq!(
            handle.body_surface(&id("other")),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(handle.measure_surface_distance(&request), Ok(measured));
        let other = SurfaceDistanceRequest::try_new(
            id("other"),
            Arc::new(triangle("revised", GeometryFidelity::Exact)),
            0.001,
        )
        .unwrap();
        assert_eq!(
            handle.measure_surface_distance(&other),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    /// Two exact boundaries certify a distance whatever their meshes'
    /// fidelity; a counterpart without one certifies nothing.
    #[test]
    fn a_surface_distance_between_exact_boundaries_needs_the_counterparts() {
        let tessellated = GeometryFidelity::tessellated(0.001).unwrap();
        let boundary = ExactBoundaryHandle::new(Arc::new(7_u32));
        assert_eq!(boundary.downcast_ref::<u32>(), Some(&7));
        assert_eq!(boundary.downcast_ref::<f64>(), None);
        assert_eq!(boundary, boundary.clone());
        assert_ne!(boundary, ExactBoundaryHandle::new(Arc::new(7_u32)));

        let bare = Arc::new(triangle("revised", tessellated));
        let bounded = Arc::new(triangle("revised", tessellated).with_exact_boundary(boundary));
        assert!(bare.exact_boundary().is_none());
        assert!(bounded.exact_boundary().is_some());
        let request = |surface: &Arc<BodySurface>| {
            SurfaceDistanceRequest::try_new(id("base"), surface.clone(), 0.001).unwrap()
        };
        assert_eq!(
            SurfaceDistanceEvidence::try_from_boundaries(
                request(&bare),
                directed(0.1, 0.2),
                directed(0.0, 0.1),
                exact()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        assert_eq!(
            SurfaceDistanceEvidence::try_from_boundaries(
                request(&bounded),
                directed(0.1, 0.2),
                directed(0.0, 0.1),
                approximate()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        // The mesh basis still refuses the tessellation.
        assert_eq!(
            SurfaceDistanceEvidence::try_new(
                request(&bounded),
                directed(0.1, 0.2),
                directed(0.0, 0.1),
                exact()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        let measured = SurfaceDistanceEvidence::try_from_boundaries(
            request(&bounded),
            directed(0.1, 0.2),
            directed(0.0, 0.1),
            exact(),
        )
        .unwrap();
        assert_eq!(measured.basis(), SurfaceBasis::ExactBoundary);
        assert_eq!(
            measured.distance(),
            LengthInterval::try_new(0.1, 0.2).unwrap()
        );
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
                        footprint_offset_metres: offset,
                        direction: VerticalDirection::Either,
                        surfaces: crate::VerticalSurfaces::Extents,
                    }
                ),
                Err(ProximityError::InvalidMeasurement)
            );
        }
    }

    #[test]
    fn surface_pairs_are_distinct_projections_and_nearest_takes_no_offset() {
        use crate::{CounterpartSurface, SubjectSurface, VerticalSurfaces};
        let between = |counterpart, offset| ProximityProjection::Vertical {
            footprint_offset_metres: offset,
            direction: VerticalDirection::Above,
            surfaces: VerticalSurfaces::Between {
                subject: SubjectSurface::Top,
                counterpart,
            },
        };
        let extents = ProximityProjection::Vertical {
            footprint_offset_metres: 0.0,
            direction: VerticalDirection::Above,
            surfaces: VerticalSurfaces::Extents,
        };
        assert_ne!(between(CounterpartSurface::Nearest, 0.0), extents);
        assert_ne!(
            between(CounterpartSurface::Nearest, 0.0),
            between(CounterpartSurface::Bottom, 0.0)
        );
        assert!(
            ProximityRequest::projected(
                id("pipe"),
                id("wall"),
                between(CounterpartSurface::Bottom, 0.5)
            )
            .is_ok()
        );
        assert_eq!(
            ProximityRequest::projected(
                id("pipe"),
                id("wall"),
                between(CounterpartSurface::Nearest, 0.5)
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(SubjectSurface::Bottom.name(), "bottom");
        assert_eq!(CounterpartSurface::Nearest.name(), "nearest");
    }

    /// Above and below are different questions.
    #[test]
    fn a_vertical_direction_distinguishes_requests() {
        let vertical = |direction| ProximityProjection::Vertical {
            footprint_offset_metres: 0.0,
            direction,
            surfaces: crate::VerticalSurfaces::Extents,
        };
        let above = vertical(VerticalDirection::Above);
        assert_ne!(above, vertical(VerticalDirection::Below));
        assert_ne!(above, vertical(VerticalDirection::Either));
        assert!(vertical(VerticalDirection::Either) < above);
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

    /// A certified interval narrows the fidelity's to their intersection,
    /// and one missing it cannot describe the same bodies.
    #[test]
    fn a_certified_separation_narrows_the_fidelity_interval() {
        let measured = ProximityEvidence::try_new(
            request(),
            2.004,
            None,
            0.0,
            None,
            GeometryFidelity::tessellated(0.005).unwrap(),
            approximate(),
        )
        .unwrap();
        assert_eq!(measured.certified_separation(), None);
        let (lower, upper) = measured.separation_interval_metres();
        assert!((lower - 1.999).abs() < 1e-12 && (upper - 2.009).abs() < 1e-12);

        let certified = measured
            .clone()
            .with_certified_separation(LengthInterval::try_new(1.999_999, 2.000_001).unwrap())
            .unwrap();
        assert_eq!(
            certified.separation_interval_metres(),
            (1.999_999, 2.000_001)
        );
        assert!(certified.certified_separation().is_some());
        // Wider on one side than the fidelity allows: the intersection.
        let clipped = measured
            .clone()
            .with_certified_separation(LengthInterval::try_new(1.5, 2.0).unwrap())
            .unwrap();
        let (lower, upper) = clipped.separation_interval_metres();
        assert!((lower - 1.999).abs() < 1e-12 && (upper - 2.0).abs() < 1e-12);
        assert_eq!(
            measured.with_certified_separation(LengthInterval::try_new(2.5, 2.6).unwrap()),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    fn volume(lower: f64, upper: f64) -> VolumeInterval {
        VolumeInterval::try_new(lower, upper).unwrap()
    }

    #[test]
    fn volume_intervals_are_finite_non_negative_and_ordered() {
        for (lower, upper) in [
            (-0.1, 0.1),
            (0.2, 0.1),
            (0.0, f64::INFINITY),
            (f64::NAN, 1.0),
        ] {
            assert_eq!(
                VolumeInterval::try_new(lower, upper),
                Err(ProximityError::InvalidMeasurement)
            );
        }
        assert!(VolumeInterval::exact(0.0).unwrap().is_exact());
    }

    /// Two bodies cannot share more than the smaller of them holds.
    #[test]
    fn a_shared_volume_never_exceeds_either_body() {
        assert_eq!(
            IntersectionVolume::try_new(volume(0.5, 0.6), volume(1.0, 1.0), volume(0.4, 0.4)),
            Err(ProximityError::InvalidMeasurement)
        );
        let measured =
            IntersectionVolume::try_new(volume(0.3, 0.4), volume(1.0, 1.0), volume(0.4, 0.5))
                .unwrap();
        assert_eq!(measured.smaller(), volume(0.4, 0.5));
        let (lower, upper) = measured.ratio_of_smaller();
        assert!(lower <= 0.6 && lower > 0.599, "{lower}");
        assert!((upper - 1.0).abs() < f64::EPSILON, "{upper}");
    }

    /// A share's bounds are rounded outward, so the true share lies inside.
    #[test]
    fn a_volume_share_is_rounded_outward_and_clamped() {
        let third = volume(1.0 / 3.0, 1.0 / 3.0);
        let (lower, upper) = third.share_of(volume(1.0, 1.0));
        assert!(lower < 1.0 / 3.0 && upper > 1.0 / 3.0);
        assert_eq!(volume(0.0, 0.0).share_of(volume(0.0, 0.0)), (0.0, 1.0));
        assert!((volume(1.0, 1.0).share_of(volume(1.0, 1.0)).1 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_intersection_volume_needs_a_shared_inside() {
        let measured = |separation: f64, penetration: Option<f64>, containment| {
            ProximityEvidence::try_new(
                request(),
                separation,
                penetration,
                0.0,
                containment,
                GeometryFidelity::Exact,
                exact(),
            )
            .unwrap()
        };
        let shared = |lower: f64, upper: f64| {
            IntersectionVolume::try_new(volume(lower, upper), volume(1.0, 1.0), volume(8.0, 8.0))
                .unwrap()
        };
        assert!(
            measured(0.0, Some(0.1), None)
                .with_intersection_volume(shared(0.2, 0.2))
                .is_ok()
        );
        // Open surfaces enclose nothing.
        assert_eq!(
            measured(0.0, None, None).with_intersection_volume(shared(0.0, 0.0)),
            Err(ProximityError::InvalidMeasurement)
        );
        // Bodies apart at the surface share nothing unless one holds the
        // other,
        assert_eq!(
            measured(0.2, Some(0.0), None).with_intersection_volume(shared(0.1, 0.2)),
            Err(ProximityError::InvalidMeasurement)
        );
        assert!(
            measured(0.2, Some(0.0), None)
                .with_intersection_volume(shared(0.0, 0.0))
                .is_ok()
        );
        // and then they share the whole of the inner body.
        let inside = || {
            measured(
                0.2,
                Some(0.1),
                Some(BodyContainment::SubjectInsideCounterpart),
            )
        };
        assert!(inside().with_intersection_volume(shared(1.0, 1.0)).is_ok());
        assert_eq!(
            inside().with_intersection_volume(shared(0.5, 0.6)),
            Err(ProximityError::InvalidMeasurement)
        );
    }

    #[test]
    fn a_face_distance_is_between_two_objects() {
        assert_eq!(
            FaceDistanceRequest::try_new(id("pipe"), id("pipe"), FaceClass::Top),
            Err(FaceDistanceError::SameObject)
        );
        for class in [
            FaceClass::Top,
            FaceClass::Side,
            FaceClass::Bottom,
            FaceClass::Any,
        ] {
            assert_eq!(FaceClass::parse(class.name()), Some(class));
        }
        assert_eq!(FaceClass::parse("front"), None);
        let request =
            FaceDistanceRequest::try_new(id("pipe"), id("wall"), FaceClass::Side).unwrap();
        let signed = SignedDistanceInterval::try_new(-0.1, 0.2).unwrap();
        assert_eq!(
            FaceDistanceEvidence::try_new(
                request.clone(),
                signed,
                GeometryFidelity::tessellated(0.001).unwrap(),
                exact()
            ),
            Err(FaceDistanceError::EvidenceFidelityMismatch)
        );
        assert!(
            FaceDistanceEvidence::try_new(request, signed, GeometryFidelity::Exact, exact())
                .is_ok()
        );
    }

    /// A service that does not measure face distances says so.
    #[test]
    fn face_distances_are_refused_by_default() {
        struct Silent;
        impl ProximityService for Silent {
            fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
                Err(ProximityError::Unavailable)
            }
            fn measure_proximity(
                &self,
                _: &ProximityRequest,
            ) -> Result<ProximityEvidence, ProximityError> {
                Err(ProximityError::Unavailable)
            }
        }
        let request = FaceDistanceRequest::try_new(id("pipe"), id("wall"), FaceClass::Any).unwrap();
        assert_eq!(
            ProximityServiceHandle::new(Arc::new(Silent)).measure_face_distance(&request),
            Err(FaceDistanceError::Unsupported)
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

    #[test]
    fn region_distances_bind_their_request_and_default_to_refusal() {
        struct Nothing;
        impl ProximityService for Nothing {
            fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
                Err(ProximityError::Unavailable)
            }
            fn measure_proximity(
                &self,
                _: &ProximityRequest,
            ) -> Result<ProximityEvidence, ProximityError> {
                Err(ProximityError::Unavailable)
            }
        }
        let region = ConvexPlanRegion::try_new(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]).unwrap();
        let request = RegionDistanceRequest::new(region, id("wall"));
        let evidence = |exact: bool| Evidence {
            source: SourceId::new("cad", "m").unwrap(),
            locator: "region".into(),
            exact,
        };
        assert!(
            RegionDistanceEvidence::try_new(
                request.clone(),
                0.5,
                0.5,
                GeometryFidelity::Exact,
                evidence(true)
            )
            .is_ok()
        );
        // Exact evidence is a point; tessellated evidence is not exact.
        assert_eq!(
            RegionDistanceEvidence::try_new(
                request.clone(),
                0.4,
                0.5,
                GeometryFidelity::Exact,
                evidence(true)
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            RegionDistanceEvidence::try_new(
                request.clone(),
                0.4,
                0.5,
                GeometryFidelity::tessellated(0.05).unwrap(),
                evidence(true)
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        assert_eq!(
            ProximityServiceHandle::new(Arc::new(Nothing)).measure_region_distance(&request),
            Err(ProximityError::UnsupportedProjection)
        );
    }

    /// Extents along stated directions: one per direction, bound to their
    /// request, refused by default.
    #[test]
    fn extents_along_directions_bind_their_request_and_default_to_refusal() {
        struct Nothing;
        impl ProximityService for Nothing {
            fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
                Err(ProximityError::Unavailable)
            }
            fn measure_proximity(
                &self,
                _: &ProximityRequest,
            ) -> Result<ProximityEvidence, ProximityError> {
                Err(ProximityError::Unavailable)
            }
        }
        /// Answers every request with the evidence for another.
        struct Other(OverlapAlongEvidence);
        impl ProximityService for Other {
            fn bounds(&self, _: &ObjectId) -> Result<ObjectBounds, ProximityError> {
                Err(ProximityError::Unavailable)
            }
            fn measure_proximity(
                &self,
                _: &ProximityRequest,
            ) -> Result<ProximityEvidence, ProximityError> {
                Err(ProximityError::Unavailable)
            }
            fn measure_overlap_along(
                &self,
                _: &OverlapAlongRequest,
            ) -> Result<OverlapAlongEvidence, ProximityError> {
                Ok(self.0.clone())
            }
        }
        let axis = |vector| MetricDirection::try_new(vector).unwrap();
        let along = |directions: Vec<MetricDirection>| {
            OverlapAlongRequest::try_new(id("pipe"), id("wall"), directions)
        };
        assert_eq!(along(Vec::new()), Err(ProximityError::InvalidMeasurement));
        assert_eq!(
            along(vec![axis([1.0, 0.0, 0.0]); 7]),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            OverlapAlongRequest::try_new(id("pipe"), id("pipe"), vec![axis([0.0, 0.0, 1.0])]),
            Err(ProximityError::SameObject)
        );
        let request = along(vec![axis([1.0, 1.0, 0.0]), axis([0.0, 0.0, 1.0])]).unwrap();
        let extent = LengthInterval::try_new(0.01, 0.02).unwrap();
        assert_eq!(
            OverlapAlongEvidence::try_new(
                request.clone(),
                vec![extent],
                GeometryFidelity::Exact,
                exact()
            ),
            Err(ProximityError::InvalidMeasurement)
        );
        assert_eq!(
            OverlapAlongEvidence::try_new(
                request.clone(),
                vec![extent, extent],
                GeometryFidelity::tessellated(0.001).unwrap(),
                exact()
            ),
            Err(ProximityError::EvidenceFidelityMismatch)
        );
        let measured = OverlapAlongEvidence::try_new(
            request.clone(),
            vec![extent, extent],
            GeometryFidelity::Exact,
            exact(),
        )
        .unwrap();
        assert_eq!(measured.extents(), &[extent, extent]);

        assert_eq!(
            ProximityServiceHandle::new(Arc::new(Nothing)).measure_overlap_along(&request),
            Err(ProximityError::UnsupportedProjection)
        );

        let other = along(vec![axis([0.0, 1.0, 0.0]), axis([0.0, 0.0, 1.0])]).unwrap();
        assert_eq!(
            ProximityServiceHandle::new(Arc::new(Other(measured))).measure_overlap_along(&other),
            Err(ProximityError::InvalidMeasurement)
        );
    }
}
