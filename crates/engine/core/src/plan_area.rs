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
//!
//! The covered area of a footprint is the union of several sources' effect
//! areas clipped to it ([`crate::CoverageRequest`]): how much of a room the
//! devices placed in it reach.
//!
//! The uncovered elevation area ([`ElevationRequest`]) is the same question
//! asked in a vertical plane of the object's own: the object and its cover
//! projected onto the plane through a stated plan axis and the vertical,
//! each cover grown by a length along the axis and another in height. It is
//! how much of a wall's face no structural wall stands behind, where plan
//! and height checked apart pass a wall under a full-height counterpart on
//! one half and a half-height one on the other.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::coverage::{CoverageEvidence, CoverageRequest, check_answer};

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

/// The length of a footprint's boundary in plan, holes included, in
/// metres, with its evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct FootprintPerimeter {
    object: ObjectId,
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl FootprintPerimeter {
    /// A perimeter of `object` known to lie in `[lower, upper]`; the
    /// evidence is exact exactly when the bounds coincide.
    ///
    /// # Errors
    ///
    /// [`PlanAreaError::InvalidMeasurement`] for bounds not finite,
    /// negative or reversed; [`PlanAreaError::InexactEvidence`] for evidence
    /// that does not match them.
    pub fn try_new(
        object: ObjectId,
        lower: f64,
        upper: f64,
        evidence: Evidence,
    ) -> Result<Self, PlanAreaError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanAreaError::InexactEvidence);
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

    /// The least length the boundary may have, in metres.
    #[must_use]
    pub fn lower_metres(&self) -> f64 {
        self.lower
    }

    /// The greatest length the boundary may have, in metres.
    #[must_use]
    pub fn upper_metres(&self) -> f64 {
        self.upper
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// The band between two footprints facing each other along a direction.
///
/// It is the convex hull of the two footprints, cut to the positions along
/// `direction` that both footprints reach: between two parallel walls, the
/// strip between them over the length they share, walls included. Where
/// their reaches along the direction do not overlap, the band is empty.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanBand {
    first: ObjectId,
    second: ObjectId,
    direction: [f64; 2],
}

impl PlanBand {
    /// The band between `first` and `second` along `direction`, a plan
    /// vector normalised here. The two are ordered, so one band has one
    /// spelling.
    pub fn try_new(
        first: ObjectId,
        second: ObjectId,
        direction: [f64; 2],
    ) -> Result<Self, PlanAreaError> {
        let length = direction[0].hypot(direction[1]);
        if first == second {
            return Err(PlanAreaError::Unavailable(format!(
                "a band needs two objects, not {first} twice"
            )));
        }
        if !length.is_finite() || length <= f64::EPSILON {
            return Err(PlanAreaError::Unavailable(format!(
                "a band needs a plan direction, not {direction:?}"
            )));
        }
        let (first, second) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        Ok(Self {
            first,
            second,
            direction: [direction[0] / length, direction[1] / length],
        })
    }

    /// The two objects, in order.
    #[must_use]
    pub fn objects(&self) -> [&ObjectId; 2] {
        [&self.first, &self.second]
    }

    /// The unit plan direction along which the band is cut.
    #[must_use]
    pub fn direction(&self) -> [f64; 2] {
        self.direction
    }
}

/// A request for the area of an object's elevation that its cover leaves
/// uncovered.
///
/// The elevation is the object projected onto the vertical plane through
/// `axis` (a plan direction, normalised here): positions along the axis
/// against heights. Only the part of each cover object within the object's
/// own depth across the axis, widened by `along_growth_metres` on both
/// sides, is projected; that projection is grown by `along_growth_metres`
/// along the axis and `vertical_growth_metres` in height.
///
/// `frame` objects cover by the frame they form, not by their bodies: the
/// convex hull of their projected parts (a column at each end and a beam
/// over them enclose the bay between), grown the same way. An empty frame
/// adds nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct ElevationRequest {
    object: ObjectId,
    axis: [f64; 2],
    cover: Vec<ObjectId>,
    frame: Vec<ObjectId>,
    along_growth: f64,
    vertical_growth: f64,
}

impl ElevationRequest {
    /// The request, its cover and frame sorted and without repeats.
    ///
    /// Refuses an axis that is not a finite plan direction, a growth that is
    /// negative or not finite, and an object in its own cover or frame.
    pub fn try_new(
        object: ObjectId,
        axis: [f64; 2],
        cover: &[ObjectId],
        frame: &[ObjectId],
        along_growth_metres: f64,
        vertical_growth_metres: f64,
    ) -> Result<Self, PlanAreaError> {
        let length = axis[0].hypot(axis[1]);
        if !length.is_finite() || length <= f64::EPSILON {
            return Err(PlanAreaError::Unavailable(format!(
                "an elevation needs a plan axis, not {axis:?}"
            )));
        }
        for growth in [along_growth_metres, vertical_growth_metres] {
            if !growth.is_finite() || growth < 0.0 {
                return Err(PlanAreaError::Unavailable(format!(
                    "a growth of {growth} m is not a non-negative length"
                )));
            }
        }
        if cover.contains(&object) || frame.contains(&object) {
            return Err(PlanAreaError::Unavailable(format!(
                "{object} cannot cover its own elevation"
            )));
        }
        let sorted = |objects: &[ObjectId]| {
            let mut objects = objects.to_vec();
            objects.sort();
            objects.dedup();
            objects
        };
        Ok(Self {
            object,
            axis: [axis[0] / length, axis[1] / length],
            cover: sorted(cover),
            frame: sorted(frame),
            along_growth: along_growth_metres,
            vertical_growth: vertical_growth_metres,
        })
    }

    /// The object whose elevation is measured.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The unit plan direction the elevation runs along.
    #[must_use]
    pub fn axis(&self) -> [f64; 2] {
        self.axis
    }

    /// The objects covering by their bodies, sorted.
    #[must_use]
    pub fn cover(&self) -> &[ObjectId] {
        &self.cover
    }

    /// The objects covering by the frame they form, sorted.
    #[must_use]
    pub fn frame(&self) -> &[ObjectId] {
        &self.frame
    }

    /// Growth along the axis, and the depth added across it on each side.
    #[must_use]
    pub fn along_growth_metres(&self) -> f64 {
        self.along_growth
    }

    /// Growth in height.
    #[must_use]
    pub fn vertical_growth_metres(&self) -> f64 {
        self.vertical_growth
    }
}

/// The elevation area of an object and the part of it its cover leaves
/// uncovered, each in square metres as an interval.
#[derive(Clone, Debug, PartialEq)]
pub struct ElevationCover {
    object: ObjectId,
    area: (f64, f64),
    uncovered: (f64, f64),
    evidence: Evidence,
}

impl ElevationCover {
    /// The elevation of `object` with area in `area` and uncovered area in
    /// `uncovered`, each `(lower, upper)`.
    ///
    /// Refuses bounds that are not finite, negative or reversed, an
    /// uncovered area surely larger than the elevation, and evidence that
    /// is exact unless both are points (or a point claimed inexact).
    pub fn try_new(
        object: ObjectId,
        area: (f64, f64),
        uncovered: (f64, f64),
        evidence: Evidence,
    ) -> Result<Self, PlanAreaError> {
        let valid = |(lower, upper): (f64, f64)| {
            lower.is_finite() && upper.is_finite() && lower >= 0.0 && lower <= upper
        };
        if !valid(area) || !valid(uncovered) || uncovered.0 > area.1 {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        #[allow(clippy::float_cmp)]
        let exact = area.0 == area.1 && uncovered.0 == uncovered.1;
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(PlanAreaError::InexactEvidence);
        }
        Ok(Self {
            object,
            area,
            uncovered,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The elevation's area, `(lower, upper)` square metres.
    #[must_use]
    pub fn area_square_metres(&self) -> (f64, f64) {
        self.area
    }

    /// The uncovered part of it, `(lower, upper)` square metres.
    #[must_use]
    pub fn uncovered_square_metres(&self) -> (f64, f64) {
        self.uncovered
    }

    /// Whether both areas are known exactly.
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
    // gate: measures area
    fn measure_footprint(&self, object: &ObjectId) -> Result<PlanArea, PlanAreaError>;
    /// The length of the boundary of `object`'s footprint, holes included.
    ///
    /// A service that does not measure perimeters refuses.
    // gate: measures length
    fn measure_footprint_perimeter(
        &self,
        object: &ObjectId,
    ) -> Result<FootprintPerimeter, PlanAreaError> {
        let _ = object;
        Err(PlanAreaError::Unavailable(
            "this service does not measure perimeters".into(),
        ))
    }
    /// The area where the footprints of `first` and `second` overlap.
    // gate: measures area
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
    // gate: measures area
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

    /// The area of `object`'s footprint outside the union of `bands`.
    ///
    /// An empty set of bands leaves the whole footprint outside. A service
    /// that does not measure bands refuses; it never answers with the
    /// footprint or with zero.
    // gate: measures area
    fn measure_outside_bands(
        &self,
        object: &ObjectId,
        bands: &[PlanBand],
    ) -> Result<PlanArea, PlanAreaError> {
        let _ = (object, bands);
        Err(PlanAreaError::Unavailable(
            "this plan-area service does not measure bands".into(),
        ))
    }

    /// How much of the request's subject footprint the union of its
    /// sources' effect areas covers.
    ///
    /// A service that does not measure coverage refuses; it never answers
    /// with an empty or a whole cover.
    // gate: measures area, number
    fn measure_coverage(
        &self,
        request: &CoverageRequest,
    ) -> Result<CoverageEvidence, PlanAreaError> {
        Err(PlanAreaError::Unavailable(format!(
            "this plan-area service does not measure the coverage of {}",
            request.subject()
        )))
    }

    /// The elevation of the request's object and the part of it the
    /// request's cover and frame, grown, leave uncovered.
    ///
    /// A service that does not measure elevations refuses; it never answers
    /// with the whole elevation or with zero.
    // gate: measures area, length
    fn measure_elevation_cover(
        &self,
        request: &ElevationRequest,
    ) -> Result<ElevationCover, PlanAreaError> {
        Err(PlanAreaError::Unavailable(format!(
            "this plan-area service does not measure the elevation of {}",
            request.object()
        )))
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
    pub fn measure_footprint_perimeter(
        &self,
        object: &ObjectId,
    ) -> Result<FootprintPerimeter, PlanAreaError> {
        let perimeter = self.0.measure_footprint_perimeter(object)?;
        if perimeter.object() != object {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        Ok(perimeter)
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

    /// The area of `object`'s footprint outside the union of `bands`.
    ///
    /// A band bounded by `object` itself is refused rather than measured;
    /// `bands` reach the service sorted and without repeats.
    pub fn measure_outside_bands(
        &self,
        object: &ObjectId,
        bands: &[PlanBand],
    ) -> Result<PlanArea, PlanAreaError> {
        if bands.iter().any(|band| band.objects().contains(&object)) {
            return Err(PlanAreaError::Unavailable(format!(
                "{object} cannot bound a band over its own footprint"
            )));
        }
        let mut bands = bands.to_vec();
        bands.sort_by(|a, b| {
            (
                a.objects(),
                a.direction[0].to_bits(),
                a.direction[1].to_bits(),
            )
                .cmp(&(
                    b.objects(),
                    b.direction[0].to_bits(),
                    b.direction[1].to_bits(),
                ))
        });
        bands.dedup();
        self.0.measure_outside_bands(object, &bands)
    }

    /// How much of the request's subject footprint its sources cover.
    ///
    /// An answer about another subject, or not listing exactly the
    /// requested sources in order, is refused.
    pub fn measure_coverage(
        &self,
        request: &CoverageRequest,
    ) -> Result<CoverageEvidence, PlanAreaError> {
        let answer = self.0.measure_coverage(request)?;
        check_answer(request, &answer)?;
        Ok(answer)
    }

    /// The elevation of the request's object and its uncovered part.
    ///
    /// An answer about another object is refused.
    pub fn measure_elevation_cover(
        &self,
        request: &ElevationRequest,
    ) -> Result<ElevationCover, PlanAreaError> {
        let answer = self.0.measure_elevation_cover(request)?;
        if answer.object() != request.object() {
            return Err(PlanAreaError::InvalidMeasurement);
        }
        Ok(answer)
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
        let band = super::PlanBand::try_new(id("b"), id("c"), [1.0, 0.0]).unwrap();
        assert!(matches!(
            handle.measure_outside_bands(&id("a"), &[band]),
            Err(PlanAreaError::Unavailable(_))
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn a_band_is_ordered_normalised_and_never_bounded_by_its_subject() {
        let band = super::PlanBand::try_new(id("c"), id("b"), [0.0, 2.0]).unwrap();
        assert_eq!(band.objects(), [&id("b"), &id("c")]);
        assert_eq!(band.direction(), [0.0, 1.0]);
        assert!(super::PlanBand::try_new(id("b"), id("b"), [1.0, 0.0]).is_err());
        assert!(super::PlanBand::try_new(id("b"), id("c"), [0.0, 0.0]).is_err());
        assert!(super::PlanBand::try_new(id("b"), id("c"), [f64::NAN, 1.0]).is_err());
        let handle = PlanAreaServiceHandle::new(Arc::new(FootprintsOnly::default()));
        assert!(matches!(
            handle.measure_outside_bands(&id("b"), &[band]),
            Err(PlanAreaError::Unavailable(message)) if message.contains("its own footprint")
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

    #[test]
    #[allow(clippy::float_cmp)]
    fn an_elevation_request_is_normalised_ordered_and_never_covers_itself() {
        use super::ElevationRequest;
        let request = ElevationRequest::try_new(
            id("a"),
            [0.0, 2.0],
            &[id("c"), id("b"), id("c")],
            &[],
            0.1,
            0.2,
        )
        .unwrap();
        assert_eq!(request.axis(), [0.0, 1.0]);
        assert_eq!(request.cover(), [id("b"), id("c")]);
        assert!(request.frame().is_empty());
        assert_eq!(
            (
                request.along_growth_metres(),
                request.vertical_growth_metres()
            ),
            (0.1, 0.2)
        );
        for (axis, along, vertical) in [
            ([0.0, 0.0], 0.0, 0.0),
            ([f64::NAN, 1.0], 0.0, 0.0),
            ([1.0, 0.0], -0.1, 0.0),
            ([1.0, 0.0], 0.0, f64::INFINITY),
        ] {
            assert!(
                ElevationRequest::try_new(id("a"), axis, &[], &[], along, vertical).is_err(),
                "{axis:?} {along} {vertical}"
            );
        }
        assert!(ElevationRequest::try_new(id("a"), [1.0, 0.0], &[id("a")], &[], 0.0, 0.0).is_err());
        assert!(ElevationRequest::try_new(id("a"), [1.0, 0.0], &[], &[id("a")], 0.0, 0.0).is_err());
    }

    #[test]
    fn an_elevation_cover_must_be_coherent_and_about_the_requested_object() {
        use super::ElevationCover;
        let mut approximate = exact();
        approximate.exact = false;
        assert!(ElevationCover::try_new(id("a"), (2.0, 2.0), (1.0, 1.0), exact()).is_ok());
        assert!(
            ElevationCover::try_new(id("a"), (2.0, 2.0), (1.0, 1.5), approximate.clone()).is_ok()
        );
        assert_eq!(
            ElevationCover::try_new(id("a"), (2.0, 2.0), (1.0, 1.5), exact()),
            Err(PlanAreaError::InexactEvidence)
        );
        assert_eq!(
            ElevationCover::try_new(id("a"), (2.0, 2.0), (1.0, 1.0), approximate.clone()),
            Err(PlanAreaError::InexactEvidence)
        );
        for (area, uncovered) in [
            ((2.0, 1.0), (0.0, 0.0)),
            ((1.0, 1.0), (1.5, 1.5)),
            ((1.0, 1.0), (-0.5, 0.5)),
            ((1.0, f64::NAN), (0.0, 0.0)),
        ] {
            assert_eq!(
                ElevationCover::try_new(id("a"), area, uncovered, approximate.clone()),
                Err(PlanAreaError::InvalidMeasurement),
                "{area:?} {uncovered:?}"
            );
        }
    }

    #[test]
    fn an_elevation_about_another_object_or_unmeasured_is_refused() {
        use super::{ElevationCover, ElevationRequest};
        struct Elsewhere;
        impl PlanAreaService for Elsewhere {
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
            fn measure_elevation_cover(
                &self,
                _: &ElevationRequest,
            ) -> Result<ElevationCover, PlanAreaError> {
                ElevationCover::try_new(id("b"), (1.0, 1.0), (0.0, 0.0), exact())
            }
        }
        let request = ElevationRequest::try_new(id("a"), [1.0, 0.0], &[], &[], 0.0, 0.0).unwrap();
        assert_eq!(
            PlanAreaServiceHandle::new(Arc::new(Elsewhere)).measure_elevation_cover(&request),
            Err(PlanAreaError::InvalidMeasurement)
        );
        assert!(matches!(
            PlanAreaServiceHandle::new(Arc::new(FootprintsOnly::default()))
                .measure_elevation_cover(&request),
            Err(PlanAreaError::Unavailable(_))
        ));
    }
}
