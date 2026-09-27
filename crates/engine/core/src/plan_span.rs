//! Plan spans: how far an object's footprint reaches across itself, how far
//! apart two footprints lie between their centres or their farthest points,
//! and where a footprint's centre lies.
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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        CentrePlacement, PlanCentre, PlanLength, PlanSpan, PlanSpanError, PlanSpanService,
        PlanSpanServiceHandle,
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
