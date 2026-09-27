//! Vertical extents: the elevations of an object's bottom and top.
//!
//! ADR 0004: this seam measures. Whether two stacked slabs are far enough
//! apart, or equally spaced, is a rule's judgement over these measurements.
//!
//! An elevation is an interval. A mesh that is the object's exact shape
//! measures exactly; one that approximates curved faces measures within its
//! declared chord deviation, and a rule must decide from the whole interval,
//! never from its midpoint.
//!
//! The same seam measures an object's extent along any direction
//! ([`DirectionalExtent`]): its lowest and highest positions projected onto
//! that direction. Along an object's own placement axis this is the body's
//! depth in that axis, such as a straight wall's thickness. Vertical is the
//! special case the rest of this module names.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::MetricDirection;

/// Failure to measure a vertical extent.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VerticalExtentError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The geometry could not be measured, for example a body the host could
    /// not mesh, or an object declared to have no body.
    #[error("vertical extent unavailable: {0}")]
    Unavailable(String),
    /// An elevation is non-finite, its bounds are reversed, the bottom lies
    /// above the top, or the extent names another object.
    #[error("vertical extent measurement is invalid")]
    InvalidMeasurement,
    /// Evidence reported as exact for an interval, or as inexact for points.
    #[error("vertical extent evidence does not match its exactness")]
    InexactEvidence,
}

/// An elevation in metres, known to lie in `[lower, upper]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationInterval {
    lower: f64,
    upper: f64,
}

impl ElevationInterval {
    /// An elevation interval; bounds must be finite and ordered.
    pub fn try_new(lower: f64, upper: f64) -> Result<Self, VerticalExtentError> {
        if !lower.is_finite() || !upper.is_finite() || lower > upper {
            return Err(VerticalExtentError::InvalidMeasurement);
        }
        Ok(Self { lower, upper })
    }

    /// An elevation known exactly.
    pub fn exact(metres: f64) -> Result<Self, VerticalExtentError> {
        Self::try_new(metres, metres)
    }

    /// Lowest possible elevation, in metres.
    #[must_use]
    pub fn lower_metres(&self) -> f64 {
        self.lower
    }

    /// Highest possible elevation, in metres.
    #[must_use]
    pub fn upper_metres(&self) -> f64 {
        self.upper
    }

    /// Whether the elevation is a single value.
    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn is_exact(&self) -> bool {
        self.lower == self.upper
    }
}

/// The elevations of one object's lowest and highest points, with evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct VerticalExtent {
    object: ObjectId,
    bottom: ElevationInterval,
    top: ElevationInterval,
    evidence: Evidence,
}

impl VerticalExtent {
    /// A vertical extent of `object`.
    ///
    /// The bottom may not lie above the top at either bound, and the evidence
    /// is exact exactly when both elevations are: an interval cannot be exact
    /// evidence, and points cannot be approximate.
    pub fn try_new(
        object: ObjectId,
        bottom: ElevationInterval,
        top: ElevationInterval,
        evidence: Evidence,
    ) -> Result<Self, VerticalExtentError> {
        if bottom.lower > top.lower || bottom.upper > top.upper {
            return Err(VerticalExtentError::InvalidMeasurement);
        }
        let exact = bottom.is_exact() && top.is_exact();
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(VerticalExtentError::InexactEvidence);
        }
        Ok(Self {
            object,
            bottom,
            top,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// Elevation of the object's lowest point.
    #[must_use]
    pub fn bottom(&self) -> ElevationInterval {
        self.bottom
    }

    /// Elevation of the object's highest point.
    #[must_use]
    pub fn top(&self) -> ElevationInterval {
        self.top
    }

    /// Whether both elevations are known exactly.
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

/// The lowest and highest positions of one object's body projected onto a
/// direction, in metres along it, with evidence.
///
/// Positions are coordinates along `direction` from the canonical origin, so
/// only their difference, the extent, means anything on its own.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectionalExtent {
    object: ObjectId,
    direction: MetricDirection,
    lower: ElevationInterval,
    upper: ElevationInterval,
    evidence: Evidence,
}

impl DirectionalExtent {
    /// The extent of `object` along `direction`.
    ///
    /// As for a [`VerticalExtent`], the lowest position may not lie above
    /// the highest at either bound, and the evidence is exact exactly when
    /// both positions are points.
    pub fn try_new(
        object: ObjectId,
        direction: MetricDirection,
        lower: ElevationInterval,
        upper: ElevationInterval,
        evidence: Evidence,
    ) -> Result<Self, VerticalExtentError> {
        if lower.lower > upper.lower || lower.upper > upper.upper {
            return Err(VerticalExtentError::InvalidMeasurement);
        }
        let exact = lower.is_exact() && upper.is_exact();
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(VerticalExtentError::InexactEvidence);
        }
        Ok(Self {
            object,
            direction,
            lower,
            upper,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The direction the body was projected onto.
    #[must_use]
    pub fn direction(&self) -> MetricDirection {
        self.direction
    }

    /// Position of the body's lowest point along the direction.
    #[must_use]
    pub fn lower(&self) -> ElevationInterval {
        self.lower
    }

    /// Position of the body's highest point along the direction.
    #[must_use]
    pub fn upper(&self) -> ElevationInterval {
        self.upper
    }

    /// The extent, highest less lowest position, as `(lower, upper)` metres
    /// sure to hold the exact difference of the two positions.
    #[must_use]
    pub fn length_metres(&self) -> (f64, f64) {
        let (low, _) = difference(self.upper.lower, self.lower.upper);
        let (_, high) = difference(self.upper.upper, self.lower.lower);
        (low.max(0.0), high.max(0.0))
    }

    /// Whether both positions are known exactly.
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

/// `minuend - subtrahend` as an interval sure to hold the exact difference:
/// the rounded difference, widened by one step where rounding moved it.
fn difference(minuend: f64, subtrahend: f64) -> (f64, f64) {
    let rounded = minuend - subtrahend;
    // Two-sum: the exact difference is `rounded + error`.
    let back = rounded - minuend;
    let error = (minuend - (rounded - back)) + (-subtrahend - back);
    if error > 0.0 {
        (rounded, rounded.next_up())
    } else if error < 0.0 {
        (rounded.next_down(), rounded)
    } else {
        (rounded, rounded)
    }
}

/// Measures the vertical extents of model objects.
pub trait VerticalExtentService: Send + Sync + 'static {
    /// The elevations of `object`'s lowest and highest points.
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError>;

    /// The lowest and highest positions of `object`'s body along
    /// `direction`.
    ///
    /// A service that measures elevations only refuses; it never answers
    /// with the vertical extent.
    fn measure_directional_extent(
        &self,
        object: &ObjectId,
        direction: MetricDirection,
    ) -> Result<DirectionalExtent, VerticalExtentError> {
        let _ = (object, direction);
        Err(VerticalExtentError::Unavailable(
            "this service measures vertical extents only".into(),
        ))
    }
}

/// Registry handle for a [`VerticalExtentService`].
#[derive(Clone)]
pub struct VerticalExtentServiceHandle(Arc<dyn VerticalExtentService>);

impl VerticalExtentServiceHandle {
    /// Wraps a trusted vertical-extent service.
    #[must_use]
    pub fn new(service: Arc<dyn VerticalExtentService>) -> Self {
        Self(service)
    }

    /// The vertical extent of `object`. An extent naming another object
    /// answers a different question and is refused.
    pub fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError> {
        let extent = self.0.measure_vertical_extent(object)?;
        if extent.object() != object {
            return Err(VerticalExtentError::InvalidMeasurement);
        }
        Ok(extent)
    }

    /// The extent of `object` along `direction`. An extent naming another
    /// object or another direction answers a different question and is
    /// refused.
    pub fn measure_directional_extent(
        &self,
        object: &ObjectId,
        direction: MetricDirection,
    ) -> Result<DirectionalExtent, VerticalExtentError> {
        let extent = self.0.measure_directional_extent(object, direction)?;
        if extent.object() != object || extent.direction() != direction {
            return Err(VerticalExtentError::InvalidMeasurement);
        }
        Ok(extent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    fn exact() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "vertical-extent:a")
    }

    fn point(value: f64) -> ElevationInterval {
        ElevationInterval::exact(value).unwrap()
    }

    #[test]
    fn exactness_and_intervals_must_agree() {
        assert!(VerticalExtent::try_new(id("a"), point(0.0), point(0.2), exact()).is_ok());
        let widened = ElevationInterval::try_new(0.199, 0.201).unwrap();
        assert_eq!(
            VerticalExtent::try_new(id("a"), point(0.0), widened, exact()),
            Err(VerticalExtentError::InexactEvidence)
        );
        let mut approximate = exact();
        approximate.exact = false;
        assert!(VerticalExtent::try_new(id("a"), point(0.0), widened, approximate.clone()).is_ok());
        assert_eq!(
            VerticalExtent::try_new(id("a"), point(0.0), point(0.2), approximate),
            Err(VerticalExtentError::InexactEvidence)
        );
    }

    #[test]
    fn incoherent_elevations_are_refused() {
        assert_eq!(
            ElevationInterval::try_new(1.0, 0.0),
            Err(VerticalExtentError::InvalidMeasurement)
        );
        assert_eq!(
            ElevationInterval::try_new(f64::NAN, 0.0),
            Err(VerticalExtentError::InvalidMeasurement)
        );
        assert_eq!(
            VerticalExtent::try_new(id("a"), point(1.0), point(0.0), exact()),
            Err(VerticalExtentError::InvalidMeasurement)
        );
    }

    struct Other;
    impl VerticalExtentService for Other {
        fn measure_vertical_extent(
            &self,
            _: &ObjectId,
        ) -> Result<VerticalExtent, VerticalExtentError> {
            VerticalExtent::try_new(id("b"), point(0.0), point(1.0), exact())
        }
    }

    #[test]
    fn an_extent_of_another_object_is_refused() {
        let handle = VerticalExtentServiceHandle::new(Arc::new(Other));
        assert_eq!(
            handle.measure_vertical_extent(&id("a")),
            Err(VerticalExtentError::InvalidMeasurement)
        );
    }

    fn along(vector: [f64; 3]) -> MetricDirection {
        MetricDirection::try_new(vector).unwrap()
    }

    #[test]
    fn a_service_measuring_elevations_only_refuses_directions() {
        let handle = VerticalExtentServiceHandle::new(Arc::new(Other));
        assert!(matches!(
            handle.measure_directional_extent(&id("a"), along([0.0, 1.0, 0.0])),
            Err(VerticalExtentError::Unavailable(_))
        ));
    }

    #[test]
    fn directional_exactness_and_order_must_agree() {
        let y = along([0.0, 1.0, 0.0]);
        let extent =
            DirectionalExtent::try_new(id("a"), y, point(0.1), point(0.4), exact()).unwrap();
        let (low, high) = extent.length_metres();
        // 0.4 - 0.1 rounds, so the interval holds the exact difference.
        assert!(low <= 0.3 && 0.3 <= high && high - low <= 2.0 * f64::EPSILON);
        assert_eq!(
            DirectionalExtent::try_new(id("a"), y, point(0.4), point(0.1), exact()),
            Err(VerticalExtentError::InvalidMeasurement)
        );
        let widened = ElevationInterval::try_new(0.39, 0.41).unwrap();
        assert_eq!(
            DirectionalExtent::try_new(id("a"), y, point(0.1), widened, exact()),
            Err(VerticalExtentError::InexactEvidence)
        );
        let exact_length = DirectionalExtent::try_new(id("a"), y, point(1.0), point(1.5), exact())
            .unwrap()
            .length_metres();
        assert_eq!(exact_length, (0.5, 0.5));
    }

    struct Sideways;
    impl VerticalExtentService for Sideways {
        fn measure_vertical_extent(
            &self,
            object: &ObjectId,
        ) -> Result<VerticalExtent, VerticalExtentError> {
            VerticalExtent::try_new(object.clone(), point(0.0), point(1.0), exact())
        }
        fn measure_directional_extent(
            &self,
            object: &ObjectId,
            _: MetricDirection,
        ) -> Result<DirectionalExtent, VerticalExtentError> {
            DirectionalExtent::try_new(
                object.clone(),
                along([1.0, 0.0, 0.0]),
                point(0.0),
                point(1.0),
                exact(),
            )
        }
    }

    #[test]
    fn an_extent_along_another_direction_is_refused() {
        let handle = VerticalExtentServiceHandle::new(Arc::new(Sideways));
        assert!(
            handle
                .measure_directional_extent(&id("a"), along([1.0, 0.0, 0.0]))
                .is_ok()
        );
        assert_eq!(
            handle.measure_directional_extent(&id("a"), along([0.0, 1.0, 0.0])),
            Err(VerticalExtentError::InvalidMeasurement)
        );
    }
}
