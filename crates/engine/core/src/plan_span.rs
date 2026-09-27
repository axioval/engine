//! Plan spans: how far an object's footprint reaches across itself, and how
//! far apart two footprints lie between their centres or their farthest
//! points.
//!
//! ADR 0004: this seam measures. Whether a room's exits are far enough apart
//! for its size is a rule's judgement over these measurements. The closest
//! distance between two footprints is the proximity service's `horizontal`
//! projection; this seam does not repeat it.
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
}

#[cfg(test)]
mod tests {
    use super::{PlanLength, PlanSpanError};
    use axioval_ir::{Evidence, SourceId};

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
