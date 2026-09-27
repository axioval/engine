//! Vertical extents: the elevations of an object's bottom and top.
//!
//! ADR 0004: this seam measures. Whether two stacked slabs are far enough
//! apart, or equally spaced, is a rule's judgement over these measurements.
//!
//! An elevation is an interval. A mesh that is the object's exact shape
//! measures exactly; one that approximates curved faces measures within its
//! declared chord deviation, and a rule must decide from the whole interval,
//! never from its midpoint.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

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

/// Measures the vertical extents of model objects.
pub trait VerticalExtentService: Send + Sync + 'static {
    /// The elevations of `object`'s lowest and highest points.
    fn measure_vertical_extent(
        &self,
        object: &ObjectId,
    ) -> Result<VerticalExtent, VerticalExtentError>;
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
}
