//! Space-boundary coverage: how much of a space's body surface the space
//! boundaries its source declares for it cover, where they leave gaps and
//! where they overlap.
//!
//! ADR 0004: this seam measures. Whether a space's boundaries are complete
//! enough, or overlap too much, is a rule's judgement over these areas.
//!
//! A space boundary is the source's statement that a surface bounds a
//! space: its identity (the stating relationship, source-qualified), the
//! element on its other side when the source names one, and its connection
//! surface. Which boundaries a space has is a source fact the host
//! registers, never the rule's selection; a boundary whose surface the host
//! could not read leaves the space unmeasured, never measured without it.
//!
//! Areas lie on the body's surface: each boundary surface lying on a face
//! plane of the body, within the request's `plane_tolerance`, counts on that
//! plane; one lying on no face plane is reported
//! ([`BoundaryPlacement::OffSurface`]) and covers nothing. The covered share
//! is derived here from the covered and surface areas, never accepted from
//! an adapter. The evidence is exact only when every input is exact and
//! planar and nothing was rounded on the way; then every interval is a point.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Failure to measure a space's boundary coverage.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BoundaryCoverageError {
    /// The service holds no space boundaries for this object: it is not a
    /// space the host registered.
    #[error("no space boundaries registered for `{0}`")]
    UnknownSpace(ObjectId),
    /// The space occupies no volume, so it has no surface to cover.
    #[error("space `{0}` has no body")]
    NoBody(ObjectId),
    /// The space's body or one of its boundary surfaces could not be
    /// measured, for example a surface the host could not read, a curved
    /// body or an overlay that cannot be computed.
    #[error("boundary coverage unavailable: {0}")]
    Unavailable(String),
    /// The request is malformed.
    #[error("invalid boundary coverage request: {0}")]
    InvalidRequest(String),
    /// The service does not measure boundary coverage.
    #[error("the geometry service does not measure space-boundary coverage")]
    Unsupported,
    /// The answer is malformed, inconsistent or bound to another request.
    #[error("boundary coverage measurement is invalid")]
    InvalidMeasurement,
}

/// How much of `space`'s body surface its declared boundaries cover.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryCoverageRequest {
    space: ObjectId,
    plane_tolerance: f64,
}

impl BoundaryCoverageRequest {
    /// Coverage of `space`, counting a boundary surface on a face plane when
    /// every point of it lies within `plane_tolerance_metres` of the plane.
    ///
    /// # Errors
    ///
    /// Refuses a negative or non-finite tolerance.
    pub fn try_new(
        space: ObjectId,
        plane_tolerance_metres: f64,
    ) -> Result<Self, BoundaryCoverageError> {
        if !plane_tolerance_metres.is_finite() || plane_tolerance_metres < 0.0 {
            return Err(BoundaryCoverageError::InvalidRequest(format!(
                "plane tolerance must be a finite length of at least zero, not \
                 {plane_tolerance_metres}"
            )));
        }
        Ok(Self {
            space,
            plane_tolerance: plane_tolerance_metres,
        })
    }

    /// The space whose surface is measured.
    #[must_use]
    pub fn space(&self) -> &ObjectId {
        &self.space
    }

    /// How far from a face plane a boundary surface may lie and still count
    /// on it, in metres.
    #[must_use]
    pub fn plane_tolerance_metres(&self) -> f64 {
        self.plane_tolerance
    }
}

/// An area on a surface, in square metres, known to lie in `[lower, upper]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceAreaInterval {
    lower: f64,
    upper: f64,
}

impl SurfaceAreaInterval {
    /// An area in `[lower, upper]`.
    ///
    /// # Errors
    ///
    /// Refuses non-finite or negative bounds and reversed ones.
    pub fn try_new(lower: f64, upper: f64) -> Result<Self, BoundaryCoverageError> {
        if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || lower > upper {
            return Err(BoundaryCoverageError::InvalidMeasurement);
        }
        Ok(Self { lower, upper })
    }

    /// An area known exactly.
    ///
    /// # Errors
    ///
    /// Refuses a non-finite or negative area.
    pub fn exact(square_metres: f64) -> Result<Self, BoundaryCoverageError> {
        Self::try_new(square_metres, square_metres)
    }

    /// The smallest the area can be.
    #[must_use]
    pub fn lower_square_metres(&self) -> f64 {
        self.lower
    }

    /// The largest the area can be.
    #[must_use]
    pub fn upper_square_metres(&self) -> f64 {
        self.upper
    }

    /// Whether the interval is a single value.
    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn is_point(&self) -> bool {
        self.lower == self.upper
    }
}

/// A share in `[0, 1]`, known to lie in `[lower, upper]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShareInterval {
    lower: f64,
    upper: f64,
}

impl ShareInterval {
    /// The smallest the share can be.
    #[must_use]
    pub fn lower(&self) -> f64 {
        self.lower
    }

    /// The largest the share can be.
    #[must_use]
    pub fn upper(&self) -> f64 {
        self.upper
    }
}

/// Where a boundary surface lies against the space's body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryPlacement {
    /// On the face planes of the body; `area` is the boundary's own area
    /// there, overlaps with other boundaries and parts beyond the faces
    /// included.
    OnSurface {
        /// The boundary's area projected into the face planes it lies on.
        area: SurfaceAreaInterval,
    },
    /// On no face plane of the body within the tolerance: it covers nothing.
    OffSurface,
}

/// One declared boundary of the space, as measured.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredBoundary {
    boundary: ObjectId,
    element: Option<ObjectId>,
    placement: BoundaryPlacement,
}

impl MeasuredBoundary {
    /// `boundary` (the source's boundary identity), bounding the space
    /// against `element` when the source names one.
    #[must_use]
    pub fn new(
        boundary: ObjectId,
        element: Option<ObjectId>,
        placement: BoundaryPlacement,
    ) -> Self {
        Self {
            boundary,
            element,
            placement,
        }
    }

    /// The boundary's source-qualified identity.
    #[must_use]
    pub fn boundary(&self) -> &ObjectId {
        &self.boundary
    }

    /// The element on the boundary's other side, when the source names one.
    #[must_use]
    pub fn element(&self) -> Option<&ObjectId> {
        self.element.as_ref()
    }

    /// Where the boundary lies against the body.
    #[must_use]
    pub fn placement(&self) -> BoundaryPlacement {
        self.placement
    }
}

/// Two boundaries covering the same part of the surface.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryOverlap {
    first: ObjectId,
    second: ObjectId,
    area: SurfaceAreaInterval,
}

impl BoundaryOverlap {
    /// `first` and `second` overlapping by `area`; the pair is ordered here,
    /// so one overlap has one spelling.
    ///
    /// # Errors
    ///
    /// Refuses a boundary overlapping itself.
    pub fn try_new(
        first: ObjectId,
        second: ObjectId,
        area: SurfaceAreaInterval,
    ) -> Result<Self, BoundaryCoverageError> {
        if first == second {
            return Err(BoundaryCoverageError::InvalidMeasurement);
        }
        let (first, second) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        Ok(Self {
            first,
            second,
            area,
        })
    }

    /// The first boundary of the pair, in identity order.
    #[must_use]
    pub fn first(&self) -> &ObjectId {
        &self.first
    }

    /// The second boundary of the pair, in identity order.
    #[must_use]
    pub fn second(&self) -> &ObjectId {
        &self.second
    }

    /// The area both cover.
    #[must_use]
    pub fn area(&self) -> SurfaceAreaInterval {
        self.area
    }
}

/// The areas a boundary-coverage answer reports, in square metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverageAreas {
    /// The body's surface area.
    pub surface: SurfaceAreaInterval,
    /// The part of the surface at least one boundary covers.
    pub covered: SurfaceAreaInterval,
    /// The part of the surface no boundary covers.
    pub uncovered: SurfaceAreaInterval,
    /// The part of the surface two or more boundaries cover.
    pub overlap: SurfaceAreaInterval,
}

/// How much of a space's body surface its declared boundaries cover, bound
/// to the request it answers.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryCoverage {
    request: BoundaryCoverageRequest,
    areas: CoverageAreas,
    share: ShareInterval,
    boundaries: Vec<MeasuredBoundary>,
    overlaps: Vec<BoundaryOverlap>,
    evidence: Evidence,
}

impl BoundaryCoverage {
    /// An answer to `request`: the `areas`, every declared boundary of the
    /// space as measured and the pairs of boundaries that overlap.
    ///
    /// The covered share is derived from the covered and surface areas.
    ///
    /// # Errors
    ///
    /// Refuses an unlocated evidence, a surface that may be empty, covered
    /// and uncovered areas that cannot add up to the surface, a boundary
    /// listed twice, an overlap naming a boundary not on the surface, and
    /// exact evidence over an interval that is not a point.
    pub fn try_new(
        request: BoundaryCoverageRequest,
        areas: CoverageAreas,
        mut boundaries: Vec<MeasuredBoundary>,
        mut overlaps: Vec<BoundaryOverlap>,
        evidence: Evidence,
    ) -> Result<Self, BoundaryCoverageError> {
        let invalid = || BoundaryCoverageError::InvalidMeasurement;
        if evidence.locator.trim().is_empty() {
            return Err(invalid());
        }
        let CoverageAreas {
            surface,
            covered,
            uncovered,
            overlap,
        } = areas;
        if surface.lower <= 0.0 {
            return Err(invalid());
        }
        // Covered and uncovered partition the surface; allow the rounding of
        // adding two measured areas.
        let slack = 1e-9 * surface.upper.max(1.0);
        if covered.lower + uncovered.lower > surface.upper + slack
            || covered.upper + uncovered.upper < surface.lower - slack
            || covered.lower > surface.upper
            || uncovered.lower > surface.upper
            || overlap.lower > covered.upper + slack
        {
            return Err(invalid());
        }
        boundaries.sort_by(|a, b| a.boundary.cmp(&b.boundary));
        if boundaries
            .windows(2)
            .any(|pair| pair[0].boundary == pair[1].boundary)
        {
            return Err(invalid());
        }
        let on_surface = |id: &ObjectId| {
            boundaries
                .binary_search_by(|boundary| boundary.boundary.cmp(id))
                .is_ok_and(|index| {
                    matches!(
                        boundaries[index].placement,
                        BoundaryPlacement::OnSurface { .. }
                    )
                })
        };
        if overlaps
            .iter()
            .any(|pair| !on_surface(&pair.first) || !on_surface(&pair.second))
        {
            return Err(invalid());
        }
        overlaps.sort_by(|a, b| (&a.first, &a.second).cmp(&(&b.first, &b.second)));
        if overlaps
            .windows(2)
            .any(|pair| (&pair[0].first, &pair[0].second) == (&pair[1].first, &pair[1].second))
        {
            return Err(invalid());
        }
        if evidence.exact {
            let points = [surface, covered, uncovered, overlap]
                .iter()
                .chain(overlaps.iter().map(|pair| &pair.area))
                .all(SurfaceAreaInterval::is_point)
                && boundaries.iter().all(|boundary| match boundary.placement {
                    BoundaryPlacement::OnSurface { area } => area.is_point(),
                    BoundaryPlacement::OffSurface => true,
                });
            if !points {
                return Err(invalid());
            }
        }
        let share = ShareInterval {
            lower: (covered.lower / surface.upper).clamp(0.0, 1.0),
            upper: (covered.upper / surface.lower).clamp(0.0, 1.0),
        };
        Ok(Self {
            request,
            areas,
            share,
            boundaries,
            overlaps,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &BoundaryCoverageRequest {
        &self.request
    }

    /// The measured space.
    #[must_use]
    pub fn space(&self) -> &ObjectId {
        &self.request.space
    }

    /// The body's surface area.
    #[must_use]
    pub fn surface_area(&self) -> SurfaceAreaInterval {
        self.areas.surface
    }

    /// The part of the surface at least one boundary covers.
    #[must_use]
    pub fn covered_area(&self) -> SurfaceAreaInterval {
        self.areas.covered
    }

    /// The part of the surface no boundary covers.
    #[must_use]
    pub fn uncovered_area(&self) -> SurfaceAreaInterval {
        self.areas.uncovered
    }

    /// The part of the surface two or more boundaries cover.
    #[must_use]
    pub fn overlap_area(&self) -> SurfaceAreaInterval {
        self.areas.overlap
    }

    /// The covered share of the surface: the covered area over the surface
    /// area, bounded by the least covered over the largest surface and the
    /// most covered over the smallest.
    #[must_use]
    pub fn covered_share(&self) -> ShareInterval {
        self.share
    }

    /// Every declared boundary of the space, in identity order.
    #[must_use]
    pub fn boundaries(&self) -> &[MeasuredBoundary] {
        &self.boundaries
    }

    /// The boundaries lying on no face plane of the body.
    pub fn off_surface(&self) -> impl Iterator<Item = &MeasuredBoundary> {
        self.boundaries
            .iter()
            .filter(|boundary| boundary.placement == BoundaryPlacement::OffSurface)
    }

    /// The pairs of boundaries covering a common part of the surface, in
    /// identity order.
    #[must_use]
    pub fn overlaps(&self) -> &[BoundaryOverlap] {
        &self.overlaps
    }

    /// Whether every area is known exactly.
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

/// Measures how much of a space's body surface its declared boundaries cover.
pub trait BoundaryCoverageService: Send + Sync + 'static {
    /// The coverage `request` asks for. The default refuses: a service that
    /// does not measure boundaries must never answer with an empty coverage.
    ///
    /// # Errors
    ///
    /// Returns [`BoundaryCoverageError::Unsupported`] unless implemented.
    // gate: measures area, number
    fn measure_boundary_coverage(
        &self,
        request: &BoundaryCoverageRequest,
    ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
        let _ = request;
        Err(BoundaryCoverageError::Unsupported)
    }
}

/// Registry handle for a [`BoundaryCoverageService`].
#[derive(Clone)]
pub struct BoundaryCoverageServiceHandle(Arc<dyn BoundaryCoverageService>);

impl BoundaryCoverageServiceHandle {
    /// Wraps a trusted boundary-coverage service.
    #[must_use]
    pub fn new(service: Arc<dyn BoundaryCoverageService>) -> Self {
        Self(service)
    }

    /// The coverage `request` asks for. An answer bound to another request
    /// is refused.
    ///
    /// # Errors
    ///
    /// Returns the service's refusal, or
    /// [`BoundaryCoverageError::InvalidMeasurement`] for an answer to another
    /// request.
    pub fn measure_boundary_coverage(
        &self,
        request: &BoundaryCoverageRequest,
    ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
        let answer = self.0.measure_boundary_coverage(request)?;
        if answer.request() != request {
            return Err(BoundaryCoverageError::InvalidMeasurement);
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn area(lower: f64, upper: f64) -> SurfaceAreaInterval {
        SurfaceAreaInterval::try_new(lower, upper).unwrap()
    }

    fn point(value: f64) -> SurfaceAreaInterval {
        SurfaceAreaInterval::exact(value).unwrap()
    }

    fn request() -> BoundaryCoverageRequest {
        BoundaryCoverageRequest::try_new(id("space"), 0.001).unwrap()
    }

    fn areas(surface: f64, covered: f64, overlap: f64) -> CoverageAreas {
        CoverageAreas {
            surface: point(surface),
            covered: point(covered),
            uncovered: point(surface - covered),
            overlap: point(overlap),
        }
    }

    fn on(boundary: &str, value: f64) -> MeasuredBoundary {
        MeasuredBoundary::new(
            id(boundary),
            Some(id("wall")),
            BoundaryPlacement::OnSurface { area: point(value) },
        )
    }

    fn exact() -> Evidence {
        Evidence::exact(source(), "boundary-coverage:space")
    }

    #[test]
    fn a_request_needs_a_finite_non_negative_tolerance() {
        assert!(BoundaryCoverageRequest::try_new(id("space"), 0.0).is_ok());
        for tolerance in [-0.1, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                BoundaryCoverageRequest::try_new(id("space"), tolerance),
                Err(BoundaryCoverageError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn intervals_refuse_reversed_negative_and_non_finite_bounds() {
        assert!(SurfaceAreaInterval::try_new(1.0, 2.0).is_ok());
        for (lower, upper) in [
            (2.0, 1.0),
            (-1.0, 1.0),
            (0.0, f64::INFINITY),
            (f64::NAN, 1.0),
        ] {
            assert_eq!(
                SurfaceAreaInterval::try_new(lower, upper),
                Err(BoundaryCoverageError::InvalidMeasurement)
            );
        }
    }

    #[test]
    fn the_share_is_derived_from_covered_and_surface_areas() {
        let coverage = BoundaryCoverage::try_new(
            request(),
            areas(10.0, 7.5, 0.0),
            vec![on("b", 5.0), on("a", 2.5)],
            vec![],
            exact(),
        )
        .unwrap();
        assert!((coverage.covered_share().lower() - 0.75).abs() < 1e-12);
        assert!((coverage.covered_share().upper() - 0.75).abs() < 1e-12);
        assert_eq!(coverage.boundaries()[0].boundary(), &id("a"));

        let approximate = BoundaryCoverage::try_new(
            request(),
            CoverageAreas {
                surface: area(9.0, 10.0),
                covered: area(4.5, 5.0),
                uncovered: area(4.5, 5.5),
                overlap: area(0.0, 0.1),
            },
            vec![],
            vec![],
            Evidence {
                exact: false,
                ..exact()
            },
        )
        .unwrap();
        assert!((approximate.covered_share().lower() - 0.45).abs() < 1e-12);
        assert!((approximate.covered_share().upper() - 5.0 / 9.0).abs() < 1e-12);
    }

    #[test]
    fn exact_evidence_needs_point_intervals() {
        let widened = CoverageAreas {
            surface: area(10.0, 10.1),
            ..areas(10.0, 5.0, 0.0)
        };
        assert_eq!(
            BoundaryCoverage::try_new(request(), widened, vec![], vec![], exact()),
            Err(BoundaryCoverageError::InvalidMeasurement)
        );
        let widened_boundary = MeasuredBoundary::new(
            id("a"),
            None,
            BoundaryPlacement::OnSurface {
                area: area(1.0, 1.1),
            },
        );
        assert_eq!(
            BoundaryCoverage::try_new(
                request(),
                areas(10.0, 1.0, 0.0),
                vec![widened_boundary],
                vec![],
                exact()
            ),
            Err(BoundaryCoverageError::InvalidMeasurement)
        );
    }

    #[test]
    fn inconsistent_areas_are_refused() {
        // Covered and uncovered adding up to more than the surface.
        let over = CoverageAreas {
            uncovered: point(6.0),
            ..areas(10.0, 5.0, 0.0)
        };
        // An empty surface has no share.
        let empty = areas(0.0, 0.0, 0.0);
        // More overlap than coverage.
        let overlapping = areas(10.0, 1.0, 2.0);
        for areas in [over, empty, overlapping] {
            assert_eq!(
                BoundaryCoverage::try_new(request(), areas, vec![], vec![], exact()),
                Err(BoundaryCoverageError::InvalidMeasurement)
            );
        }
    }

    #[test]
    fn boundaries_are_listed_once_and_overlaps_name_boundaries_on_the_surface() {
        assert_eq!(
            BoundaryCoverage::try_new(
                request(),
                areas(10.0, 5.0, 0.0),
                vec![on("a", 5.0), on("a", 5.0)],
                vec![],
                exact()
            ),
            Err(BoundaryCoverageError::InvalidMeasurement)
        );
        let off = MeasuredBoundary::new(id("c"), None, BoundaryPlacement::OffSurface);
        let with = |pair: BoundaryOverlap| {
            BoundaryCoverage::try_new(
                request(),
                areas(10.0, 5.0, 1.0),
                vec![on("a", 3.0), on("b", 3.0), off.clone()],
                vec![pair],
                exact(),
            )
        };
        let coverage =
            with(BoundaryOverlap::try_new(id("b"), id("a"), point(1.0)).unwrap()).unwrap();
        assert_eq!(coverage.overlaps()[0].first(), &id("a"));
        assert_eq!(coverage.off_surface().count(), 1);
        for pair in [
            BoundaryOverlap::try_new(id("a"), id("c"), point(1.0)).unwrap(),
            BoundaryOverlap::try_new(id("a"), id("z"), point(1.0)).unwrap(),
        ] {
            assert_eq!(with(pair), Err(BoundaryCoverageError::InvalidMeasurement));
        }
        assert_eq!(
            BoundaryOverlap::try_new(id("a"), id("a"), point(1.0)),
            Err(BoundaryCoverageError::InvalidMeasurement)
        );
    }

    struct Refusing;
    impl BoundaryCoverageService for Refusing {}

    struct Other;
    impl BoundaryCoverageService for Other {
        fn measure_boundary_coverage(
            &self,
            _: &BoundaryCoverageRequest,
        ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
            BoundaryCoverage::try_new(
                BoundaryCoverageRequest::try_new(id("space"), 0.5).unwrap(),
                areas(10.0, 10.0, 0.0),
                vec![],
                vec![],
                exact(),
            )
        }
    }

    #[test]
    fn the_default_refuses_and_the_handle_binds_answers_to_the_request() {
        let refusing = BoundaryCoverageServiceHandle::new(Arc::new(Refusing));
        assert_eq!(
            refusing.measure_boundary_coverage(&request()),
            Err(BoundaryCoverageError::Unsupported)
        );
        let other = BoundaryCoverageServiceHandle::new(Arc::new(Other));
        assert_eq!(
            other.measure_boundary_coverage(&request()),
            Err(BoundaryCoverageError::InvalidMeasurement)
        );
        let same = BoundaryCoverageRequest::try_new(id("space"), 0.5).unwrap();
        assert!(other.measure_boundary_coverage(&same).is_ok());
    }
}
