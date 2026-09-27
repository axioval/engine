//! Plan spans: how far an object's footprint reaches across itself, how far
//! apart two footprints lie between their centres or their farthest points,
//! and where a footprint's centre lies.
//!
//! ADR 0004: this seam measures. Whether a room's exits are far enough apart
//! for its size is a rule's judgement over these measurements. The closest
//! distance between two footprints is the proximity service's `horizontal`
//! projection; this seam does not repeat it.
//!
//! It also owns the rectangle of least area enclosing a footprint, which
//! gives a footprint its own axes: a parking bay's length along the bay, a
//! wall's direction. The rectangle says how well its orientation is known;
//! a square has no long axis, and a tessellated footprint no proven one.
//!
//! It measures the recesses of a footprint (the pockets between it and its
//! convex hull) and the section several footprints share, such as the
//! clear shaft of a light well. A section's width and length are the sides
//! of the same least-area rectangle, and only when its orientation is
//! unique: another rectangle of least area may have other sides.
//!
//! A length is an interval. A mesh that is the object's exact shape measures
//! exactly; one that approximates curved faces measures within a bound the
//! adapter derives from its declared chord deviation, and a rule must decide
//! from the whole interval, never from its midpoint.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to measure a plan span.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PlanSpanError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The geometry could not be measured, for example an object without a
    /// footprint or an overlay that cannot be computed.
    #[error("plan span unavailable: {0}")]
    Unavailable(String),
    /// A measurement is non-finite, negative, or its bounds are reversed.
    #[error("plan span measurement is invalid")]
    InvalidMeasurement,
    /// An interval was reported as exact, or a point as inexact.
    #[error("plan span evidence does not match its exactness")]
    InexactEvidence,
}

/// Between which points of two footprints a span is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlanSpan {
    /// Between the footprints' centroids: the centres of their plan areas.
    Centres,
    /// Between the two points, one in each footprint, farthest apart.
    Farthest,
}

impl PlanSpan {
    /// The span's stable name, as evidence locators cite it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Centres => "centres",
            Self::Farthest => "farthest",
        }
    }
}

/// A measured plan length in metres, with its evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanLength {
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl PlanLength {
    /// A length known to lie in `[lower, upper]`.
    ///
    /// The evidence is exact exactly when the bounds coincide: an interval
    /// cannot be exact evidence, and a point cannot be approximate.
    pub fn try_new(lower: f64, upper: f64, evidence: Evidence) -> Result<Self, PlanSpanError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        Ok(Self {
            lower,
            upper,
            evidence,
        })
    }

    /// Shortest the length can be, in metres.
    #[must_use]
    pub fn lower_metres(&self) -> f64 {
        self.lower
    }

    /// Longest the length can be, in metres.
    #[must_use]
    pub fn upper_metres(&self) -> f64 {
        self.upper
    }

    /// Whether the length is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Where a footprint's centre lies with respect to the footprint itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CentrePlacement {
    /// Every point the centre may be lies inside the footprint, off its
    /// boundary.
    Inside,
    /// Every point the centre may be lies outside the footprint, as for an
    /// L- or U-shaped room.
    Outside,
    /// The centre lies on the boundary, or close enough to it that the
    /// measurement's uncertainty could put it on either side.
    Undecided,
}

/// The centroid of an object's footprint: a plan point, how far the true
/// centroid can lie from it, and whether it lies inside the footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanCentre {
    object: ObjectId,
    point: [f64; 2],
    radius: f64,
    placement: CentrePlacement,
    evidence: Evidence,
}

impl PlanCentre {
    /// The centre of `object`'s footprint, known to lie within `radius`
    /// metres of `point`.
    ///
    /// The evidence is exact exactly when the radius is zero.
    pub fn try_new(
        object: ObjectId,
        point: [f64; 2],
        radius: f64,
        placement: CentrePlacement,
        evidence: Evidence,
    ) -> Result<Self, PlanSpanError> {
        if !point.iter().all(|value| value.is_finite()) || !radius.is_finite() || radius < 0.0 {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = radius == 0.0;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        Ok(Self {
            object,
            point,
            radius,
            placement,
            evidence,
        })
    }

    /// The object whose footprint this is the centre of.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The measured centre, in canonical metres.
    #[must_use]
    pub fn point(&self) -> [f64; 2] {
        self.point
    }

    /// How far, in metres, the true centre can lie from [`Self::point`].
    #[must_use]
    pub fn radius_metres(&self) -> f64 {
        self.radius
    }

    /// Whether the centre lies inside the footprint.
    #[must_use]
    pub fn placement(&self) -> CentrePlacement {
        self.placement
    }

    /// Whether the centre is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// How well the orientation of a [`PlanRectangle`] is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RectangleOrientation {
    /// Exactly one orientation encloses the footprint with the least area,
    /// up to quarter turns, and the rectangle has it: its axes are the
    /// footprint's own.
    Unique,
    /// Several orientations share the least area. The rectangle is one of
    /// them; another may have other sides, so neither its axes nor its
    /// sides are the footprint's own.
    Tied,
    /// The mesh approximates the object's shape, so which orientation
    /// encloses the true footprint with the least area is not known. The
    /// half extents still bound the true footprint along these axes.
    Unproven,
}

impl RectangleOrientation {
    /// The orientation's stable name, as evidence locators cite it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Unique => "unique",
            Self::Tied => "tied",
            Self::Unproven => "unproven",
        }
    }
}

/// The rectangle of least area enclosing an object's footprint, oriented
/// in plan: a centre, two unit axes and the half extents along them.
///
/// Every value is bounded: the true centre lies within
/// [`Self::centre_radius_metres`] of [`Self::centre`], each true axis within
/// [`Self::axis_error_radians`] of the stated one, and each true half
/// extent inside its interval. The evidence is exact exactly when every
/// bound is a point and the orientation is [`RectangleOrientation::Unique`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlanRectangle {
    object: ObjectId,
    centre: [f64; 2],
    centre_radius: f64,
    axes: [[f64; 2]; 2],
    axis_error: f64,
    half_extents: [(f64, f64); 2],
    orientation: RectangleOrientation,
    evidence: Evidence,
}

/// How far a stated axis may stray from unit length and from a quarter
/// turn of the other: the rounding of a normalised vector, with room.
const AXIS_ROUNDING: f64 = 1e-9;

impl PlanRectangle {
    /// The rectangle enclosing `object`'s footprint.
    ///
    /// `axes` are unit vectors, the second the first turned a quarter
    /// counter-clockwise, the first pointing into `[0, 90)` degrees, so one
    /// rectangle has one spelling. `half_extents` are `(lower, upper)`
    /// intervals along the axes in order.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        object: ObjectId,
        centre: [f64; 2],
        centre_radius: f64,
        axes: [[f64; 2]; 2],
        axis_error: f64,
        half_extents: [(f64, f64); 2],
        orientation: RectangleOrientation,
        evidence: Evidence,
    ) -> Result<Self, PlanSpanError> {
        let finite = centre
            .iter()
            .chain(axes.iter().flatten())
            .all(|value| value.is_finite());
        let bounded = |value: f64| value.is_finite() && value >= 0.0;
        let [[ux, uy], [vx, vy]] = axes;
        let unit = (ux.mul_add(ux, uy * uy) - 1.0).abs() <= AXIS_ROUNDING;
        let quarter = (vx + uy).abs() <= AXIS_ROUNDING && (vy - ux).abs() <= AXIS_ROUNDING;
        if !finite
            || !bounded(centre_radius)
            || !bounded(axis_error)
            || axis_error > std::f64::consts::FRAC_PI_2
            || !unit
            || !quarter
            || !(ux > 0.0 && uy >= 0.0)
            || half_extents
                .iter()
                .any(|&(lower, upper)| !bounded(lower) || !upper.is_finite() || lower > upper)
        {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = centre_radius == 0.0
            && axis_error == 0.0
            && half_extents.iter().all(|(lower, upper)| lower == upper)
            && orientation == RectangleOrientation::Unique;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        Ok(Self {
            object,
            centre,
            centre_radius,
            axes,
            axis_error,
            half_extents,
            orientation,
            evidence,
        })
    }

    /// The object whose footprint the rectangle encloses.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The measured centre, in canonical metres.
    #[must_use]
    pub fn centre(&self) -> [f64; 2] {
        self.centre
    }

    /// How far, in metres, the true centre can lie from [`Self::centre`].
    #[must_use]
    pub fn centre_radius_metres(&self) -> f64 {
        self.centre_radius
    }

    /// The two unit axes, the second the first turned a quarter
    /// counter-clockwise.
    #[must_use]
    pub fn axes(&self) -> [[f64; 2]; 2] {
        self.axes
    }

    /// How far, in radians, each true axis can be turned from the stated
    /// one.
    #[must_use]
    pub fn axis_error_radians(&self) -> f64 {
        self.axis_error
    }

    /// The half extents along the two axes, each `(lower, upper)` metres.
    #[must_use]
    pub fn half_extents_metres(&self) -> [(f64, f64); 2] {
        self.half_extents
    }

    /// How well the orientation is known.
    #[must_use]
    pub fn orientation(&self) -> RectangleOrientation {
        self.orientation
    }

    /// Whether the rectangle is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// Why the axes are not the footprint's own, or `None` when they are.
    fn unoriented(&self) -> Option<String> {
        match self.orientation {
            RectangleOrientation::Unique => None,
            RectangleOrientation::Tied => Some(format!(
                "several orientations enclose {} with the least area",
                self.object
            )),
            RectangleOrientation::Unproven => Some(format!(
                "{} is tessellated, so the orientation enclosing its true footprint with the \
                 least area is not known",
                self.object
            )),
        }
    }

    /// The footprint's width and length along its own axes: the shorter and
    /// the longer side, each `(lower, upper)` metres.
    ///
    /// Refused unless the orientation is [`RectangleOrientation::Unique`]:
    /// another rectangle of least area may have other sides. Near-equal
    /// sides are fine; which of them is the longer does not matter here.
    pub fn width_and_length(&self) -> Result<[(f64, f64); 2], String> {
        if let Some(reason) = self.unoriented() {
            return Err(reason);
        }
        let [(a0, a1), (b0, b1)] = self.half_extents;
        Ok([
            (2.0 * a0.min(b0), 2.0 * a1.min(b1)),
            (2.0 * a0.max(b0), 2.0 * a1.max(b1)),
        ])
    }

    /// The index of the longer axis, when the orientation is unique and one
    /// side is surely longer than the other. A square, or a rectangle whose
    /// sides the measurement cannot order, has no long axis.
    pub fn long_axis(&self) -> Result<usize, String> {
        if let Some(reason) = self.unoriented() {
            return Err(reason);
        }
        let [(a0, a1), (b0, b1)] = self.half_extents;
        if a0 > b1 {
            Ok(0)
        } else if b0 > a1 {
            Ok(1)
        } else {
            Err(format!(
                "the sides of {} are too close to equal to tell its long axis",
                self.object
            ))
        }
    }

    /// The acute angle between this footprint's long axis and `other`'s, in
    /// degrees within `[0, 90]`, as `(lower, upper)` sure to hold the angle
    /// between the true axes.
    pub fn long_axis_angle(&self, other: &Self) -> Result<(f64, f64), String> {
        let own = self.axes[self.long_axis()?];
        let theirs = other.axes[other.long_axis()?];
        let dot = own[0].mul_add(theirs[0], own[1] * theirs[1]).abs();
        let cross = own[0].mul_add(theirs[1], -(own[1] * theirs[0])).abs();
        let angle = cross.atan2(dot).to_degrees();
        // The rounding of the products and the arctangent, well inside a
        // micro-degree, plus how far either axis may be turned.
        let slack = (self.axis_error + other.axis_error).to_degrees() + 1e-9;
        Ok(((angle - slack).max(0.0), (angle + slack).min(90.0)))
    }
}

/// One recess of a footprint: a pocket between the footprint and its convex
/// hull, closed by one hull edge, its mouth.
///
/// The width is the mouth's length; the depth the farthest the pocket
/// reaches from the mouth's line. Both are [`PlanLength`]s, so a rule judges
/// them as intervals.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanRecess {
    mouth: [[f64; 2]; 2],
    width: PlanLength,
    depth: PlanLength,
}

impl PlanRecess {
    /// A recess whose mouth runs from `mouth[0]` to `mouth[1]`.
    pub fn try_new(
        mouth: [[f64; 2]; 2],
        width: PlanLength,
        depth: PlanLength,
    ) -> Result<Self, PlanSpanError> {
        if !mouth.iter().flatten().all(|value| value.is_finite()) {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        Ok(Self {
            mouth,
            width,
            depth,
        })
    }

    /// The mouth's ends, in canonical metres, as a reviewer locates the
    /// recess.
    #[must_use]
    pub fn mouth(&self) -> [[f64; 2]; 2] {
        self.mouth
    }

    /// The width of the mouth.
    #[must_use]
    pub fn width(&self) -> &PlanLength {
        &self.width
    }

    /// How deep the recess reaches behind its mouth.
    #[must_use]
    pub fn depth(&self) -> &PlanLength {
        &self.depth
    }
}

/// Every recess of one object's footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanRecesses {
    object: ObjectId,
    recesses: Vec<PlanRecess>,
    evidence: Evidence,
}

impl PlanRecesses {
    /// The recesses of `object`, in the order the adapter walks its
    /// boundary; none for a convex footprint.
    pub fn try_new(
        object: ObjectId,
        recesses: Vec<PlanRecess>,
        evidence: Evidence,
    ) -> Result<Self, PlanSpanError> {
        if evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        Ok(Self {
            object,
            recesses,
            evidence,
        })
    }

    /// The object whose footprint was measured.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The recesses found.
    #[must_use]
    pub fn recesses(&self) -> &[PlanRecess] {
        &self.recesses
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// The plan section several objects share: the intersection of their
/// footprints, such as the clear shaft of spaces stacked into a light well.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanSection {
    objects: Vec<ObjectId>,
    area: (f64, f64),
    sides: Option<(PlanLength, PlanLength)>,
    evidence: Evidence,
}

impl PlanSection {
    /// The section of `objects`, with an area in `[lower, upper]` square
    /// metres and, unless it is empty, the short and long sides of its
    /// least-area enclosing rectangle, the one [`PlanRectangle`] describes
    /// for a footprint. A service states sides only for a unique
    /// orientation ([`RectangleOrientation::Unique`]) and refuses a section
    /// whose least-area rectangles may have other sides.
    ///
    /// The evidence is exact exactly when the area is a point. An empty
    /// section (upper bound zero) has no rectangle, and any other has one.
    pub fn try_new(
        objects: Vec<ObjectId>,
        area: (f64, f64),
        sides: Option<(PlanLength, PlanLength)>,
        evidence: Evidence,
    ) -> Result<Self, PlanSpanError> {
        let (lower, upper) = area;
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanSpanError::InexactEvidence);
        }
        if (upper == 0.0) != sides.is_none() {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        if let Some((short, long)) = &sides
            && short.lower_metres() > long.upper_metres()
        {
            return Err(PlanSpanError::InvalidMeasurement);
        }
        Ok(Self {
            objects,
            area,
            sides,
            evidence,
        })
    }

    /// The objects whose footprints were intersected.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }

    /// Smallest the section's area can be, in square metres.
    #[must_use]
    pub fn area_lower(&self) -> f64 {
        self.area.0
    }

    /// Largest the section's area can be, in square metres.
    #[must_use]
    pub fn area_upper(&self) -> f64 {
        self.area.1
    }

    /// The short side of the section's minimum-area enclosing rectangle,
    /// its width; `None` for an empty section.
    #[must_use]
    pub fn width(&self) -> Option<&PlanLength> {
        self.sides.as_ref().map(|(short, _)| short)
    }

    /// The long side of the section's minimum-area enclosing rectangle;
    /// `None` for an empty section.
    #[must_use]
    pub fn length(&self) -> Option<&PlanLength> {
        self.sides.as_ref().map(|(_, long)| long)
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures plan spans of model objects.
pub trait PlanSpanService: Send + Sync + 'static {
    /// The longest distance between two points of `object`'s footprint: its
    /// longest plan diagonal. A footprint with no point has none, and is
    /// refused, never zero.
    fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError>;
    /// The plan distance between the footprints of `first` and `second`,
    /// measured `between` their centres or their farthest points.
    fn measure_span(
        &self,
        first: &ObjectId,
        second: &ObjectId,
        between: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError>;
    /// The centroid of `object`'s footprint, the same centre
    /// [`PlanSpan::Centres`] measures between, and whether it lies inside
    /// the footprint. A service that does not locate centres refuses by
    /// default, never answering with another point.
    fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        Err(PlanSpanError::Unavailable(format!(
            "this plan-span service does not locate the centre of {object}"
        )))
    }
    /// The rectangle of least area enclosing `object`'s footprint. A
    /// service that does not orient footprints refuses by default, never
    /// answering with the footprint's axis-aligned box.
    fn measure_rectangle(&self, object: &ObjectId) -> Result<PlanRectangle, PlanSpanError> {
        Err(PlanSpanError::Unavailable(format!(
            "this plan-span service does not orient the footprint of {object}"
        )))
    }
    /// The recesses of `object`'s footprint: the pockets between its outer
    /// boundary and its convex hull. A service that does not find recesses
    /// refuses by default, never answering that there are none.
    fn measure_recesses(&self, object: &ObjectId) -> Result<PlanRecesses, PlanSpanError> {
        Err(PlanSpanError::Unavailable(format!(
            "this plan-span service does not measure the recesses of {object}"
        )))
    }
    /// The plan section `objects` share: the intersection of their
    /// footprints, its area and its minimum-area rectangle. A service that
    /// does not intersect footprints refuses by default.
    fn measure_section(&self, objects: &[ObjectId]) -> Result<PlanSection, PlanSpanError> {
        Err(PlanSpanError::Unavailable(format!(
            "this plan-span service does not measure the section of {} objects",
            objects.len()
        )))
    }
}

/// Registry handle for a [`PlanSpanService`].
#[derive(Clone)]
pub struct PlanSpanServiceHandle(Arc<dyn PlanSpanService>);

impl PlanSpanServiceHandle {
    /// Wraps a trusted plan-span service.
    #[must_use]
    pub fn new(service: Arc<dyn PlanSpanService>) -> Self {
        Self(service)
    }

    /// The longest plan diagonal of `object`'s footprint.
    pub fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        self.0.measure_diameter(object)
    }

    /// The span between two footprints; one object twice is refused rather
    /// than measured against itself.
    pub fn measure_span(
        &self,
        first: &ObjectId,
        second: &ObjectId,
        between: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        if first == second {
            return Err(PlanSpanError::Unavailable(format!(
                "a span needs two objects, not {first} twice"
            )));
        }
        self.0.measure_span(first, second, between)
    }

    /// The centre of `object`'s footprint; a centre naming another object
    /// is refused.
    pub fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        let centre = self.0.measure_centre(object)?;
        if centre.object() != object {
            return Err(PlanSpanError::Unavailable(format!(
                "a centre of {} was returned for {object}",
                centre.object()
            )));
        }
        Ok(centre)
    }

    /// The least-area rectangle enclosing `object`'s footprint; a rectangle
    /// naming another object is refused.
    pub fn measure_rectangle(&self, object: &ObjectId) -> Result<PlanRectangle, PlanSpanError> {
        let rectangle = self.0.measure_rectangle(object)?;
        if rectangle.object() != object {
            return Err(PlanSpanError::Unavailable(format!(
                "a rectangle of {} was returned for {object}",
                rectangle.object()
            )));
        }
        Ok(rectangle)
    }

    /// The recesses of `object`'s footprint; recesses naming another object
    /// are refused.
    pub fn measure_recesses(&self, object: &ObjectId) -> Result<PlanRecesses, PlanSpanError> {
        let recesses = self.0.measure_recesses(object)?;
        if recesses.object() != object {
            return Err(PlanSpanError::Unavailable(format!(
                "the recesses of {} were returned for {object}",
                recesses.object()
            )));
        }
        Ok(recesses)
    }

    /// The section `objects` share. No object is refused, and so is a
    /// section naming other objects than those asked for.
    pub fn measure_section(&self, objects: &[ObjectId]) -> Result<PlanSection, PlanSpanError> {
        let mut asked = objects.to_vec();
        asked.sort();
        asked.dedup();
        if asked.is_empty() {
            return Err(PlanSpanError::Unavailable(
                "a section needs at least one object".into(),
            ));
        }
        let section = self.0.measure_section(&asked)?;
        let mut answered = section.objects().to_vec();
        answered.sort();
        answered.dedup();
        if answered != asked {
            return Err(PlanSpanError::Unavailable(
                "a section of other objects was returned".into(),
            ));
        }
        Ok(section)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        CentrePlacement, PlanCentre, PlanLength, PlanRecess, PlanRecesses, PlanRectangle,
        PlanSection, PlanSpan, PlanSpanError, PlanSpanService, PlanSpanServiceHandle,
        RectangleOrientation,
    };
    use axioval_ir::{Evidence, ObjectId, SourceId};

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    /// Answers every centre about object `a`, and nothing else.
    struct AlwaysA;

    impl PlanSpanService for AlwaysA {
        fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
            Err(PlanSpanError::Unavailable("unused".into()))
        }
        fn measure_span(
            &self,
            _: &ObjectId,
            _: &ObjectId,
            _: PlanSpan,
        ) -> Result<PlanLength, PlanSpanError> {
            Err(PlanSpanError::Unavailable("unused".into()))
        }
        fn measure_centre(&self, _: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
            PlanCentre::try_new(
                id("a"),
                [1.0, 2.0],
                0.0,
                CentrePlacement::Inside,
                Evidence::exact(SourceId::new("cad", "m").unwrap(), "plan-centre:a"),
            )
        }
    }

    /// Measures nothing, and locates no centre.
    struct Silent;

    impl PlanSpanService for Silent {
        fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
            Err(PlanSpanError::Unavailable("unused".into()))
        }
        fn measure_span(
            &self,
            _: &ObjectId,
            _: &ObjectId,
            _: PlanSpan,
        ) -> Result<PlanLength, PlanSpanError> {
            Err(PlanSpanError::Unavailable("unused".into()))
        }
    }

    /// A rectangle of `a` with half extents `half` along the axes, turned
    /// `turn` radians from the x-axis.
    fn rectangle(
        half: [(f64, f64); 2],
        turn: f64,
        orientation: RectangleOrientation,
    ) -> Result<PlanRectangle, PlanSpanError> {
        #[allow(clippy::float_cmp)]
        let exact = half.iter().all(|(low, high)| low == high)
            && orientation == RectangleOrientation::Unique;
        let (sin, cos) = turn.sin_cos();
        PlanRectangle::try_new(
            id("a"),
            [0.0, 0.0],
            0.0,
            [[cos, sin], [-sin, cos]],
            0.0,
            half,
            orientation,
            Evidence {
                source: SourceId::new("cad", "m").unwrap(),
                locator: "plan-rectangle:a".into(),
                exact,
            },
        )
    }

    #[test]
    fn a_rectangle_has_a_long_axis_only_when_one_side_is_surely_longer() {
        let unique = RectangleOrientation::Unique;
        let bay = rectangle([(1.25, 1.25), (2.5, 2.5)], 0.0, unique).unwrap();
        assert_eq!(bay.long_axis(), Ok(1));
        assert_eq!(bay.width_and_length(), Ok([(2.5, 2.5), (5.0, 5.0)]));
        let square = rectangle([(1.5, 1.5), (1.5, 1.5)], 0.0, unique).unwrap();
        assert!(square.long_axis().is_err());
        assert_eq!(square.width_and_length(), Ok([(3.0, 3.0), (3.0, 3.0)]));
        // Sides whose intervals overlap cannot be ordered.
        let close = rectangle([(1.0, 1.2), (1.1, 1.3)], 0.0, unique).unwrap();
        assert!(close.long_axis().is_err());
        assert_eq!(close.width_and_length(), Ok([(2.0, 2.4), (2.2, 2.6)]));
        for orientation in [RectangleOrientation::Tied, RectangleOrientation::Unproven] {
            let other = rectangle([(1.25, 1.25), (2.5, 2.5)], 0.0, orientation).unwrap();
            assert!(other.long_axis().is_err());
            assert!(other.width_and_length().is_err());
        }
    }

    #[test]
    fn long_axes_meet_at_an_acute_angle_interval() {
        let unique = RectangleOrientation::Unique;
        let along = rectangle([(2.5, 2.5), (1.0, 1.0)], 0.0, unique).unwrap();
        let turned = rectangle([(1.0, 1.0), (2.5, 2.5)], 30.0_f64.to_radians(), unique).unwrap();
        // The turned one's long axis is its second: 120 degrees, so 60.
        let (low, high) = along.long_axis_angle(&turned).unwrap();
        assert!(
            low <= 60.0 && 60.0 <= high && high - low < 1e-6,
            "{low} {high}"
        );
        let (low, _) = along.long_axis_angle(&along).unwrap();
        assert!(low.abs() < 1e-12);
    }

    #[test]
    fn a_rectangle_must_be_valid_and_honest_about_its_exactness() {
        let unique = RectangleOrientation::Unique;
        // A first axis outside [0, 90) degrees has another spelling.
        assert_eq!(
            rectangle([(1.0, 1.0), (2.0, 2.0)], 100.0_f64.to_radians(), unique),
            Err(PlanSpanError::InvalidMeasurement)
        );
        assert_eq!(
            rectangle([(2.0, 1.0), (2.0, 2.0)], 0.0, unique),
            Err(PlanSpanError::InvalidMeasurement)
        );
        let mut evidence = Evidence::exact(SourceId::new("cad", "m").unwrap(), "r");
        evidence.exact = true;
        assert_eq!(
            PlanRectangle::try_new(
                id("a"),
                [0.0, 0.0],
                0.0,
                [[1.0, 0.0], [0.0, 1.0]],
                0.0,
                [(1.0, 1.0), (2.0, 2.0)],
                RectangleOrientation::Unproven,
                evidence,
            ),
            Err(PlanSpanError::InexactEvidence)
        );
        let handle = PlanSpanServiceHandle::new(Arc::new(Silent));
        assert!(matches!(
            handle.measure_rectangle(&id("a")),
            Err(PlanSpanError::Unavailable(_))
        ));
    }

    #[test]
    fn recesses_and_sections_are_bound_to_their_request_and_refused_by_default() {
        struct Wrong;
        impl PlanSpanService for Wrong {
            fn measure_diameter(&self, _: &ObjectId) -> Result<PlanLength, PlanSpanError> {
                Err(PlanSpanError::Unavailable("unused".into()))
            }
            fn measure_span(
                &self,
                _: &ObjectId,
                _: &ObjectId,
                _: PlanSpan,
            ) -> Result<PlanLength, PlanSpanError> {
                Err(PlanSpanError::Unavailable("unused".into()))
            }
            fn measure_recesses(&self, _: &ObjectId) -> Result<PlanRecesses, PlanSpanError> {
                PlanRecesses::try_new(id("a"), Vec::new(), exact())
            }
            fn measure_section(&self, _: &[ObjectId]) -> Result<PlanSection, PlanSpanError> {
                PlanSection::try_new(vec![id("a")], (0.0, 0.0), None, exact())
            }
        }
        let handle = PlanSpanServiceHandle::new(Arc::new(Wrong));
        assert!(handle.measure_recesses(&id("a")).is_ok());
        assert!(handle.measure_recesses(&id("b")).is_err());
        assert!(handle.measure_section(&[id("a"), id("a")]).is_ok());
        assert!(handle.measure_section(&[id("a"), id("b")]).is_err());
        assert!(handle.measure_section(&[]).is_err());
        let silent = PlanSpanServiceHandle::new(Arc::new(Silent));
        assert!(silent.measure_recesses(&id("a")).is_err());
        assert!(silent.measure_section(&[id("a")]).is_err());
    }

    #[test]
    fn a_section_has_a_rectangle_exactly_when_it_is_not_empty() {
        let side = |value| PlanLength::try_new(value, value, exact()).unwrap();
        let section = |area: (f64, f64), sides| {
            PlanSection::try_new(vec![id("a")], area, sides, {
                let mut evidence = exact();
                #[allow(clippy::float_cmp)]
                {
                    evidence.exact = area.0 == area.1;
                }
                evidence
            })
        };
        assert!(section((0.0, 0.0), None).is_ok());
        assert!(section((4.0, 4.0), Some((side(2.0), side(2.0)))).is_ok());
        assert!(section((1.0, 2.0), Some((side(1.0), side(2.0)))).is_ok());
        assert_eq!(
            section((4.0, 4.0), None),
            Err(PlanSpanError::InvalidMeasurement)
        );
        assert_eq!(
            section((0.0, 0.0), Some((side(1.0), side(1.0)))),
            Err(PlanSpanError::InvalidMeasurement)
        );
        assert_eq!(
            section((4.0, 4.0), Some((side(3.0), side(2.0)))),
            Err(PlanSpanError::InvalidMeasurement)
        );
        assert_eq!(
            PlanSection::try_new(vec![id("a")], (1.0, 2.0), None, exact()),
            Err(PlanSpanError::InexactEvidence)
        );
        assert_eq!(
            PlanRecess::try_new([[f64::NAN, 0.0], [1.0, 0.0]], side(1.0), side(1.0)),
            Err(PlanSpanError::InvalidMeasurement)
        );
    }

    #[test]
    fn a_centre_is_bound_to_its_object_and_refused_by_default() {
        let handle = PlanSpanServiceHandle::new(Arc::new(AlwaysA));
        let centre = handle.measure_centre(&id("a")).unwrap();
        assert_eq!(centre.object(), &id("a"));
        assert_eq!(centre.placement(), CentrePlacement::Inside);
        assert!(matches!(
            handle.measure_centre(&id("b")),
            Err(PlanSpanError::Unavailable(_))
        ));
        let silent = PlanSpanServiceHandle::new(Arc::new(Silent));
        assert!(matches!(
            silent.measure_centre(&id("a")),
            Err(PlanSpanError::Unavailable(_))
        ));
    }

    #[test]
    fn a_centre_is_exact_exactly_when_its_radius_is_zero() {
        let evidence = |exact| Evidence {
            source: SourceId::new("cad", "m").unwrap(),
            locator: "plan-centre:a".into(),
            exact,
        };
        let centre = |radius, exact| {
            PlanCentre::try_new(
                id("a"),
                [0.0, 0.0],
                radius,
                CentrePlacement::Undecided,
                evidence(exact),
            )
        };
        assert!(centre(0.0, true).is_ok());
        assert!(centre(0.1, false).is_ok());
        assert_eq!(centre(0.1, true), Err(PlanSpanError::InexactEvidence));
        assert_eq!(centre(0.0, false), Err(PlanSpanError::InexactEvidence));
        assert_eq!(centre(-0.1, false), Err(PlanSpanError::InvalidMeasurement));
        assert_eq!(
            PlanCentre::try_new(
                id("a"),
                [f64::NAN, 0.0],
                0.0,
                CentrePlacement::Inside,
                evidence(true)
            ),
            Err(PlanSpanError::InvalidMeasurement)
        );
    }

    fn exact() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "plan-diameter:a")
    }

    #[test]
    fn exactness_and_bounds_must_agree() {
        assert!(PlanLength::try_new(2.0, 2.0, exact()).is_ok());
        assert_eq!(
            PlanLength::try_new(1.0, 2.0, exact()),
            Err(PlanSpanError::InexactEvidence)
        );
        let mut approximate = exact();
        approximate.exact = false;
        assert!(PlanLength::try_new(1.0, 2.0, approximate.clone()).is_ok());
        assert_eq!(
            PlanLength::try_new(2.0, 2.0, approximate),
            Err(PlanSpanError::InexactEvidence)
        );
    }

    #[test]
    fn reversed_negative_or_non_finite_bounds_are_refused() {
        for (lower, upper) in [
            (2.0, 1.0),
            (-1.0, 1.0),
            (0.0, f64::NAN),
            (0.0, f64::INFINITY),
        ] {
            assert_eq!(
                PlanLength::try_new(lower, upper, exact()),
                Err(PlanSpanError::InvalidMeasurement),
                "{lower} {upper}"
            );
        }
    }
}
