//! Facade areas: how much of an object's surface faces the outside.
//!
//! ADR 0004: this seam measures. Whether a storey has enough facade, or too
//! much of it glazed, is a rule's judgement over these measurements.
//!
//! An object's facade area is the area of its steep faces that face away
//! from the interior: neither against another body nor looking into a
//! space. Which objects are spaces is semantic, so the host declares them;
//! whether an object is external is the rule's selection, never this seam's.
//!
//! An area is an interval. A mesh that is the object's exact shape measures
//! exactly; one that approximates curved faces measures within a bound the
//! adapter derives from its declared chord deviation, and a rule must decide
//! from the whole interval, never from its midpoint.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to measure a facade area.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FacadeAreaError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The geometry could not be measured, for example an object without a
    /// body, a mesh that fails its audit, or an unmeasured body that could
    /// cover the object's faces.
    #[error("facade area unavailable: {0}")]
    Unavailable(String),
    /// A measurement is non-finite, negative, its bounds are reversed, or it
    /// names another object.
    #[error("facade area measurement is invalid")]
    InvalidMeasurement,
    /// An interval was reported as exact, or evidence as inexact, inconsistently.
    #[error("facade area evidence does not match its exactness")]
    InexactEvidence,
}

/// The facade area of one object in square metres, with its evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct FacadeArea {
    object: ObjectId,
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl FacadeArea {
    /// The facade area of `object`, known to lie in `[lower, upper]`.
    ///
    /// The evidence is exact exactly when the bounds coincide: an interval
    /// cannot be exact evidence, and a point cannot be approximate.
    pub fn try_new(
        object: ObjectId,
        lower: f64,
        upper: f64,
        evidence: Evidence,
    ) -> Result<Self, FacadeAreaError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(FacadeAreaError::InexactEvidence);
        }
        Ok(Self {
            object,
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

    /// Smallest area the facade can have, in square metres.
    #[must_use]
    pub fn lower_square_metres(&self) -> f64 {
        self.lower
    }

    /// Largest area the facade can have, in square metres.
    #[must_use]
    pub fn upper_square_metres(&self) -> f64 {
        self.upper
    }

    /// Whether the area is known exactly.
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

/// Measures the outward-facing surface area of model objects.
pub trait FacadeAreaService: Send + Sync + 'static {
    /// The area of `object`'s faces that face the outside.
    fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError>;
    /// The area of `object`'s largest plane face: all of its body's faces
    /// lying in one oriented plane, such as a wall's side less its openings
    /// or a slab's top. The answer is a [`FacadeArea`] of that face.
    ///
    /// The default refuses with [`FacadeAreaError::Unavailable`], so a
    /// service that does not measure faces fails closed.
    fn measure_face_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let _ = object;
        Err(FacadeAreaError::Unavailable(
            "face areas are not measured by this service".into(),
        ))
    }
}

/// Registry handle for a [`FacadeAreaService`].
#[derive(Clone)]
pub struct FacadeAreaServiceHandle(Arc<dyn FacadeAreaService>);

impl FacadeAreaServiceHandle {
    /// Wraps a trusted facade-area service.
    #[must_use]
    pub fn new(service: Arc<dyn FacadeAreaService>) -> Self {
        Self(service)
    }

    /// The facade area of `object`. An area naming another object answers a
    /// different question and is refused.
    pub fn measure_facade_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let area = self.0.measure_facade_area(object)?;
        if area.object() != object {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(area)
    }

    /// The area of `object`'s largest plane face. An area naming another
    /// object answers a different question and is refused.
    pub fn measure_face_area(&self, object: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
        let area = self.0.measure_face_area(object)?;
        if area.object() != object {
            return Err(FacadeAreaError::InvalidMeasurement);
        }
        Ok(area)
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
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "facade-area:a")
    }

    #[test]
    fn exactness_and_bounds_must_agree() {
        assert!(FacadeArea::try_new(id("a"), 2.0, 2.0, exact()).is_ok());
        assert_eq!(
            FacadeArea::try_new(id("a"), 1.0, 2.0, exact()),
            Err(FacadeAreaError::InexactEvidence)
        );
        let mut approximate = exact();
        approximate.exact = false;
        assert!(FacadeArea::try_new(id("a"), 1.0, 2.0, approximate.clone()).is_ok());
        assert_eq!(
            FacadeArea::try_new(id("a"), 2.0, 2.0, approximate),
            Err(FacadeAreaError::InexactEvidence)
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
                FacadeArea::try_new(id("a"), lower, upper, exact()),
                Err(FacadeAreaError::InvalidMeasurement),
                "{lower} {upper}"
            );
        }
    }

    struct Other;
    impl FacadeAreaService for Other {
        fn measure_facade_area(&self, _: &ObjectId) -> Result<FacadeArea, FacadeAreaError> {
            FacadeArea::try_new(id("b"), 1.0, 1.0, exact())
        }
    }

    #[test]
    fn an_area_of_another_object_is_refused() {
        let handle = FacadeAreaServiceHandle::new(Arc::new(Other));
        assert_eq!(
            handle.measure_facade_area(&id("a")),
            Err(FacadeAreaError::InvalidMeasurement)
        );
    }
}
