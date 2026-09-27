//! Plan-projected areas: an object's footprint, and two footprints' overlap.
//!
//! ADR 0004: this seam measures. How much floor area a storey's spaces
//! cover, or whether a space lies within a fire compartment, is a rule's
//! judgement over these measurements.
//!
//! An area is an interval. A mesh that is the object's exact shape measures
//! exactly; one that approximates curved faces measures within a bound the
//! adapter derives from its declared chord deviation, and a rule must decide
//! from the whole interval, never from its midpoint.
//!
//! The uncovered area of a footprint is what remains of it once the union of
//! other footprints, each grown in plan by a stated length, is taken away:
//! how much of an architectural wall no structural wall stands under.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to measure a plan area.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PlanAreaError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The geometry could not be measured, for example a mesh that fails its
    /// health audit or an overlay that cannot be computed.
    #[error("plan area unavailable: {0}")]
    Unavailable(String),
    /// A measurement is non-finite, negative, or its bounds are reversed.
    #[error("plan area measurement is invalid")]
    InvalidMeasurement,
    /// An interval was reported as exact, or evidence as inexact, inconsistently.
    #[error("plan area evidence does not match its exactness")]
    InexactEvidence,
}

/// A measured plan area in square metres, with its evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanArea {
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl PlanArea {
    /// An area known to lie in `[lower, upper]`.
    ///
    /// The evidence is exact exactly when the bounds coincide: an interval
    /// cannot be exact evidence, and a point cannot be approximate.
    pub fn try_new(lower: f64, upper: f64, evidence: Evidence) -> Result<Self, PlanAreaError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanAreaError::InexactEvidence);
        }
        Ok(Self {
            lower,
            upper,
            evidence,
        })
    }

    /// Smallest area the object can have, in square metres.
    #[must_use]
    pub fn lower_square_metres(&self) -> f64 {
        self.lower
    }

    /// Largest area the object can have, in square metres.
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

/// Measures plan-projected areas of model objects.
pub trait PlanAreaService: Send + Sync + 'static {
    /// The area of `object`'s footprint: its geometry projected onto the
    /// horizontal plane, overlapping parts counted once.
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError>;
    /// The area where the footprints of `first` and `second` overlap.
    fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError>;

    /// The area of `object`'s footprint outside the union of the footprints
    /// of `cover`, each grown by `growth_metres` in every plan direction (the
    /// set of points within that distance of it).
    ///
    /// An empty `cover` leaves the whole footprint uncovered. A service that
    /// does not measure uncovered areas refuses; it never answers with the
    /// footprint or with zero.
    fn measure_uncovered_area(
        &self,
        object: &ObjectId,
        cover: &[ObjectId],
        growth_metres: f64,
    ) -> Result<PlanArea, PlanAreaError> {
        let _ = (object, cover, growth_metres);
        Err(PlanAreaError::Unavailable(
            "this plan-area service does not measure uncovered areas".into(),
        ))
    }
}

/// Registry handle for a [`PlanAreaService`].
#[derive(Clone)]
pub struct PlanAreaServiceHandle(Arc<dyn PlanAreaService>);

impl PlanAreaServiceHandle {
    /// Wraps a trusted plan-area service.
    #[must_use]
    pub fn new(service: Arc<dyn PlanAreaService>) -> Self {
        Self(service)
    }

    /// The footprint area of `object`.
    pub fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
        self.0.measure_footprint(object)
    }

    /// The overlap of two footprints, never larger than either footprint's
    /// upper bound would allow; a larger answer is refused.
    pub fn measure_plan_overlap(
        &self,
        first: &ObjectId,
        second: &ObjectId,
    ) -> Result<PlanArea, PlanAreaError> {
        self.0.measure_plan_overlap(first, second)
    }

    /// The area of `object`'s footprint that the footprints of `cover`, grown
    /// by `growth_metres`, leave uncovered.
    ///
    /// A growth that is negative or not finite, or an object covering itself,
    /// is refused rather than measured; `cover` reaches the service sorted
    /// and without repeats.
    pub fn measure_uncovered_area(
        &self,
        object: &ObjectId,
        cover: &[ObjectId],
        growth_metres: f64,
    ) -> Result<PlanArea, PlanAreaError> {
        if !growth_metres.is_finite() || growth_metres < 0.0 {
            return Err(PlanAreaError::Unavailable(format!(
                "a growth of {growth_metres} m is not a non-negative length"
            )));
        }
        if cover.contains(object) {
            return Err(PlanAreaError::Unavailable(format!(
                "{object} cannot cover its own footprint"
            )));
        }
        let mut cover = cover.to_vec();
        cover.sort();
        cover.dedup();
        self.0.measure_uncovered_area(object, &cover, growth_metres)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{PlanArea, PlanAreaError, PlanAreaService, PlanAreaServiceHandle};
    use axioval_ir::{Evidence, ObjectId, SourceId};

    /// Measures only footprints, and records the cover it was asked about.
    #[derive(Default)]
    struct FootprintsOnly(Mutex<Vec<Vec<ObjectId>>>);

    impl PlanAreaService for FootprintsOnly {
        fn measure_footprint(&self, _: &ObjectId) -> Result<PlanArea, PlanAreaError> {
            PlanArea::try_new(1.0, 1.0, exact())
        }
        fn measure_plan_overlap(
            &self,
            _: &ObjectId,
            _: &ObjectId,
        ) -> Result<PlanArea, PlanAreaError> {
            PlanArea::try_new(0.0, 0.0, exact())
        }
    }

    struct Recording(Arc<FootprintsOnly>);

    impl PlanAreaService for Recording {
        fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError> {
            self.0.measure_footprint(object)
        }
        fn measure_plan_overlap(
            &self,
            first: &ObjectId,
            second: &ObjectId,
        ) -> Result<PlanArea, PlanAreaError> {
            self.0.measure_plan_overlap(first, second)
        }
        fn measure_uncovered_area(
            &self,
            _: &ObjectId,
            cover: &[ObjectId],
            _: f64,
        ) -> Result<PlanArea, PlanAreaError> {
            self.0.0.lock().unwrap().push(cover.to_vec());
            PlanArea::try_new(0.5, 0.5, exact())
        }
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }

    #[test]
    fn a_service_without_uncovered_areas_refuses_rather_than_answering() {
        let handle = PlanAreaServiceHandle::new(Arc::new(FootprintsOnly::default()));
        assert!(matches!(
            handle.measure_uncovered_area(&id("a"), &[id("b")], 0.0),
            Err(PlanAreaError::Unavailable(_))
        ));
    }

    #[test]
    fn the_handle_refuses_a_bad_growth_or_self_cover_and_orders_the_cover() {
        let log = Arc::new(FootprintsOnly::default());
        let handle = PlanAreaServiceHandle::new(Arc::new(Recording(log.clone())));
        for growth in [-0.01, f64::NAN, f64::INFINITY] {
            assert!(
                handle
                    .measure_uncovered_area(&id("a"), &[id("b")], growth)
                    .is_err(),
                "{growth}"
            );
        }
        assert!(
            handle
                .measure_uncovered_area(&id("a"), &[id("b"), id("a")], 0.0)
                .is_err()
        );
        assert!(log.0.lock().unwrap().is_empty(), "refused before measuring");
        handle
            .measure_uncovered_area(&id("a"), &[id("c"), id("b"), id("c")], 0.1)
            .unwrap();
        assert_eq!(*log.0.lock().unwrap(), vec![vec![id("b"), id("c")]]);
    }

    fn exact() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "footprint:a")
    }

    #[test]
    fn exactness_and_bounds_must_agree() {
        assert!(PlanArea::try_new(2.0, 2.0, exact()).is_ok());
        assert_eq!(
            PlanArea::try_new(1.0, 2.0, exact()),
            Err(PlanAreaError::InexactEvidence)
        );
        let mut approximate = exact();
        approximate.exact = false;
        assert!(PlanArea::try_new(1.0, 2.0, approximate.clone()).is_ok());
        assert_eq!(
            PlanArea::try_new(2.0, 2.0, approximate),
            Err(PlanAreaError::InexactEvidence)
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
                PlanArea::try_new(lower, upper, exact()),
                Err(PlanAreaError::InvalidMeasurement),
                "{lower} {upper}"
            );
        }
    }
}
