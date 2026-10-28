//! Source-neutral space-validation evidence.
//!
//! ADR 0004: a service returns what was *measured*; a capability decides what
//! it means. A space is validated from several independent measurements --
//! clear height, duplicate bodies, boundary gaps, overlaps, and cap coverage --
//! and each is requested separately.
//!
//! That separation is the fix. The source bundled all seven aspects into one
//! fact struct, each behind a `SpaceValidationBranch<T>` that was `Option` by
//! another name, so a rule asking for one aspect discovered only at use-time
//! that a *different* aspect was missing, and every aspect failed together.
//! One request per aspect makes an unavailable measurement explicit and local.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::expression::Interval;
use crate::services::reviewable_exact_evidence;

/// Why a space measurement could not be produced.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpaceError {
    /// A reported quantity is negative, non-finite, or incoherent.
    #[error("space quantities must be finite and non-negative")]
    InvalidQuantity,
    /// The evidence backing the measurement was not exact and reviewable.
    #[error("space evidence must be exact and reviewable")]
    InexactEvidence,
    /// The adapter cannot measure this aspect for this space.
    #[error("space measurement is unavailable for the requested aspect")]
    Unavailable,
    /// The bodies are held, but the geometry kernel refused the
    /// measurement on them, for the reason given (the plan overlay
    /// refusing a footprint, say). Distinct from [`Self::Unavailable`] so
    /// a report tells a kernel refusal from a body that could not be read;
    /// never a measured zero.
    #[error("the geometry kernel refused the space measurement: {0}")]
    Refused(&'static str),
    /// Objects whose bodies could not be measured could change the
    /// measurement, so it is refused rather than taken without them.
    #[error("{0}")]
    Unmeasured(Box<UnmeasuredObjects>),
}

impl SpaceError {
    /// Refuses `aspect` for the unmeasured `objects` that could change it.
    #[must_use]
    pub fn unmeasured(aspect: SpaceAspect, objects: Vec<ObjectId>) -> Self {
        Self::Unmeasured(Box::new(UnmeasuredObjects::new(aspect, objects)))
    }
}

/// One space measurement, as a refusal names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceAspect {
    /// [`SpaceService::measure_duplicates`].
    Duplicates,
    /// [`SpaceService::measure_clear_height`].
    ClearHeight,
    /// [`SpaceService::measure_boundary_gaps`].
    BoundaryGaps,
    /// [`SpaceService::measure_overlaps`].
    Overlaps,
    /// [`SpaceService::measure_cap_coverage`] of one cap.
    CapCoverage(Cap),
    /// [`SpaceService::measure_unallocated_regions`].
    UnallocatedRegions,
}

impl std::fmt::Display for SpaceAspect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Duplicates => "duplicate spaces",
            Self::ClearHeight => "clear height",
            Self::BoundaryGaps => "boundary gaps",
            Self::Overlaps => "overlaps",
            Self::CapCoverage(Cap::Top) => "top cap coverage",
            Self::CapCoverage(Cap::Bottom) => "bottom cap coverage",
            Self::UnallocatedRegions => "unallocated regions",
        })
    }
}

/// How many unmeasured objects a refusal names before it counts the rest.
const NAMED_UNMEASURED: usize = 5;

/// The unmeasured objects that refused one space measurement.
///
/// Named so a reviewer can open what blocks the measurement: one slab the
/// host could not mesh must not leave a building's spaces unjudged without
/// saying which.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnmeasuredObjects {
    aspect: SpaceAspect,
    objects: Vec<ObjectId>,
}

impl UnmeasuredObjects {
    /// The objects that refused `aspect`, sorted and deduplicated.
    #[must_use]
    pub fn new(aspect: SpaceAspect, objects: Vec<ObjectId>) -> Self {
        Self {
            aspect,
            objects: canonical(objects),
        }
    }
    /// The refused measurement.
    #[must_use]
    pub fn aspect(&self) -> SpaceAspect {
        self.aspect
    }
    /// Every unmeasured object that could change it, in identity order.
    #[must_use]
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }
}

impl std::fmt::Display for UnmeasuredObjects {
    /// The aspect and the first objects by name, then a count of the rest.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "space {} is unavailable: {} unmeasured object(s) could change it (",
            self.aspect,
            self.objects.len()
        )?;
        for (index, object) in self.objects.iter().take(NAMED_UNMEASURED).enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{object}")?;
        }
        let more = self.objects.len().saturating_sub(NAMED_UNMEASURED);
        if more > 0 {
            write!(f, ", +{more} more")?;
        }
        f.write_str(")")
    }
}

fn finite_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// The clear height of a space, in metres: a point, or the interval a
/// tessellated space's chord deviation leaves it in (#305). The evidence is
/// exact either way: the true height surely lies in the interval.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearHeightEvidence {
    space: ObjectId,
    lower: f64,
    upper: f64,
    evidence: Evidence,
}

impl ClearHeightEvidence {
    pub fn try_new(space: ObjectId, metres: f64, evidence: Evidence) -> Result<Self, SpaceError> {
        Self::try_bracketed(space, metres, metres, evidence)
    }
    /// A clear height known to lie between `lower` and `upper` metres.
    pub fn try_bracketed(
        space: ObjectId,
        lower: f64,
        upper: f64,
        evidence: Evidence,
    ) -> Result<Self, SpaceError> {
        if !finite_non_negative(lower) || !finite_non_negative(upper) || lower > upper {
            return Err(SpaceError::InvalidQuantity);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(SpaceError::InexactEvidence);
        }
        Ok(Self {
            space,
            lower,
            upper,
            evidence,
        })
    }
    pub fn space(&self) -> &ObjectId {
        &self.space
    }
    /// The clear height; its lower bound when bracketed
    /// ([`Self::bounds_metres`]).
    pub fn metres(&self) -> f64 {
        self.lower
    }
    /// The clear height's `(lower, upper)` bounds, equal for a point.
    pub fn bounds_metres(&self) -> (f64, f64) {
        (self.lower, self.upper)
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// A contiguous run of space boundary that no element covers.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryGap {
    length_metres: f64,
    elements: Vec<ObjectId>,
}

impl BoundaryGap {
    pub fn try_new(length_metres: f64, mut elements: Vec<ObjectId>) -> Result<Self, SpaceError> {
        if !finite_non_negative(length_metres) {
            return Err(SpaceError::InvalidQuantity);
        }
        elements.sort();
        elements.dedup();
        Ok(Self {
            length_metres,
            elements,
        })
    }
    pub fn length_metres(&self) -> f64 {
        self.length_metres
    }
    pub fn elements(&self) -> &[ObjectId] {
        &self.elements
    }
}

/// How one body sits inside another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Containment {
    /// Neither body contains the other; they merely overlap.
    Partial,
    /// The measured space lies inside the other body.
    SubjectInsideOther,
    /// The other body lies inside the measured space.
    OtherInsideSubject,
}

/// Whether the plan region an overlap shares holds a disc wider than
/// `width_metres` (#425).
///
/// An adapter measures it at the width the request carried
/// ([`OverlapRequest::contact_width_metres`]), and a reader trusts it only
/// toward that width: a deeper region holds a disc at least as wide at any
/// narrower tolerance, a region that is not deeper holds none wider at any
/// wider one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanDepth {
    width_metres: f64,
    deeper: bool,
}

impl PlanDepth {
    pub fn try_new(width_metres: f64, deeper: bool) -> Result<Self, SpaceError> {
        if !finite_non_negative(width_metres) {
            return Err(SpaceError::InvalidQuantity);
        }
        Ok(Self {
            width_metres: width_metres + 0.0,
            deeper,
        })
    }
    /// The disc width, in metres, the depth was measured at.
    pub fn width_metres(&self) -> f64 {
        self.width_metres
    }
    /// Whether the shared plan region holds a disc wider than
    /// [`Self::width_metres`].
    pub fn deeper(&self) -> bool {
        self.deeper
    }
}

/// How a measured overlap reads at a tolerance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlapReading {
    /// The bodies intersect.
    Intersection,
    /// The bodies only touch, within the tolerance.
    Contact,
    /// The measurement leaves open which of the two it is.
    Undecided,
}

/// The reason a reader gives for an overlap it cannot read
/// ([`SpaceOverlap::intersections`]).
pub const UNDECIDED_OVERLAP: &str = "an overlap's measured plan area, height or depth leaves \
     undecided whether it is an intersection at the tolerance";

/// A measured overlap between a space and another body.
#[derive(Clone, Debug, PartialEq)]
pub struct SpaceOverlap {
    other: ObjectId,
    other_is_space: bool,
    area_square_metres: (f64, f64),
    height_metres: f64,
    containment: Containment,
    depth: Option<PlanDepth>,
}

impl SpaceOverlap {
    /// An overlap whose plan area lies in `area_square_metres` (a point, or
    /// an interval where slivers leave it open) and whose shared plan region
    /// has the measured `depth`, or `None` where none was measured.
    pub fn try_new(
        other: ObjectId,
        other_is_space: bool,
        area_square_metres: (f64, f64),
        height_metres: f64,
        containment: Containment,
        depth: Option<PlanDepth>,
    ) -> Result<Self, SpaceError> {
        let (lower, upper) = area_square_metres;
        if !finite_non_negative(lower)
            || !finite_non_negative(upper)
            || lower > upper
            || !finite_non_negative(height_metres)
        {
            return Err(SpaceError::InvalidQuantity);
        }
        Ok(Self {
            other,
            other_is_space,
            // A kernel can hand back -0.0; it renders as "-0.0".
            area_square_metres: (lower + 0.0, upper + 0.0),
            height_metres: height_metres + 0.0,
            containment,
            depth,
        })
    }
    pub fn other(&self) -> &ObjectId {
        &self.other
    }
    /// Whether the overlapping body is itself a space.
    pub fn other_is_space(&self) -> bool {
        self.other_is_space
    }
    /// The lower bound of the plan area.
    pub fn area_square_metres(&self) -> f64 {
        self.area_square_metres.0
    }
    /// The plan area as a lower and an upper bound.
    pub fn area_bounds(&self) -> (f64, f64) {
        self.area_square_metres
    }
    pub fn height_metres(&self) -> f64 {
        self.height_metres
    }
    pub fn containment(&self) -> Containment {
        self.containment
    }
    /// The measured depth of the shared plan region, or `None` where none
    /// was measured.
    pub fn depth(&self) -> Option<PlanDepth> {
        self.depth
    }
    /// How the overlap reads at `tolerance_metres`: either body containing
    /// the other is always an intersection; a partial overlap is one only
    /// when higher than the tolerance, over a plan area of at least
    /// `OVERLAP_AREA_EPSILON_M2` at both ends, and sharing a plan region that
    /// holds a disc wider than the tolerance. It is contact when it is no
    /// higher, when its area surely falls below the floor, or when its depth
    /// surely holds no such disc, and undecided otherwise (a depth not
    /// measured, or measured at a narrower width than the tolerance).
    pub fn reading(&self, tolerance_metres: f64) -> OverlapReading {
        match self.containment {
            Containment::SubjectInsideOther | Containment::OtherInsideSubject => {
                OverlapReading::Intersection
            }
            Containment::Partial => {
                let (lower, upper) = self.area_square_metres;
                if self.height_metres <= tolerance_metres || upper < OVERLAP_AREA_EPSILON_M2 {
                    return OverlapReading::Contact;
                }
                match self.depth {
                    Some(depth) if !depth.deeper && depth.width_metres <= tolerance_metres => {
                        OverlapReading::Contact
                    }
                    Some(depth) if depth.deeper && depth.width_metres >= tolerance_metres => {
                        if lower >= OVERLAP_AREA_EPSILON_M2 {
                            OverlapReading::Intersection
                        } else {
                            OverlapReading::Undecided
                        }
                    }
                    _ => OverlapReading::Undecided,
                }
            }
        }
    }
    /// The overlaps that are intersections at `tolerance_metres`, in order:
    /// the one reading `space-validation` judges and `axioval:measured`
    /// `intersection_count` counts. Refuses with [`UNDECIDED_OVERLAP`] when
    /// any overlap reads undecided, so a reader never counts around it.
    pub fn intersections(
        overlaps: Vec<Self>,
        tolerance_metres: f64,
    ) -> Result<Vec<Self>, SpaceError> {
        let mut kept = Vec::new();
        for overlap in overlaps {
            match overlap.reading(tolerance_metres) {
                OverlapReading::Intersection => kept.push(overlap),
                OverlapReading::Contact => {}
                OverlapReading::Undecided => return Err(SpaceError::Refused(UNDECIDED_OVERLAP)),
            }
        }
        Ok(kept)
    }
}

/// The plan area, in square metres, below which a partial overlap is
/// contact, not intersection ([`SpaceOverlap::reading`]).
pub const OVERLAP_AREA_EPSILON_M2: f64 = 1.0e-8;

/// How much of a space's horizontal cap is covered by elements.
///
/// Both areas are points, or intervals where a tessellated space or cap
/// element leaves them within its chord deviation (#305); the true areas
/// surely lie in them.
#[derive(Clone, Debug, PartialEq)]
pub struct CapCoverage {
    whole: (f64, f64),
    covered: (f64, f64),
    elements: Vec<ObjectId>,
}

impl CapCoverage {
    pub fn try_new(
        whole_area_square_metres: f64,
        covered_area_square_metres: f64,
        elements: Vec<ObjectId>,
    ) -> Result<Self, SpaceError> {
        Self::try_bracketed(
            (whole_area_square_metres, whole_area_square_metres),
            (covered_area_square_metres, covered_area_square_metres),
            elements,
        )
    }
    /// A cap whose whole and covered areas lie within the `(lower, upper)`
    /// bounds given, in square metres.
    pub fn try_bracketed(
        whole: (f64, f64),
        covered: (f64, f64),
        mut elements: Vec<ObjectId>,
    ) -> Result<Self, SpaceError> {
        if ![whole.0, whole.1, covered.0, covered.1]
            .into_iter()
            .all(finite_non_negative)
            || whole.0 <= 0.0
            || whole.0 > whole.1
            || covered.0 > covered.1
            // A cap cannot be covered over more than its own area.
            || covered.1 > whole.1
        {
            return Err(SpaceError::InvalidQuantity);
        }
        elements.sort();
        elements.dedup();
        Ok(Self {
            whole,
            // A geometry kernel can return -0.0 for an empty intersection.
            // It compares equal to 0.0 but renders as "-0.0", so a cap with no
            // coverage would report "-0.0% covered". Normalise at the boundary.
            covered: (covered.0 + 0.0, covered.1 + 0.0),
            elements,
        })
    }
    /// The cap's area; its lower bound when bracketed.
    pub fn whole_area_square_metres(&self) -> f64 {
        self.whole.0
    }
    /// The covered area; its lower bound when bracketed.
    pub fn covered_area_square_metres(&self) -> f64 {
        self.covered.0
    }
    /// The cap's area as `(lower, upper)` bounds.
    pub fn whole_bounds(&self) -> (f64, f64) {
        self.whole
    }
    /// The covered area as `(lower, upper)` bounds.
    pub fn covered_bounds(&self) -> (f64, f64) {
        self.covered
    }
    pub fn elements(&self) -> &[ObjectId] {
        &self.elements
    }
    /// Fraction of the cap that is covered, computed exactly; its lower
    /// bound when bracketed ([`Self::covered_ratio_bounds`]).
    ///
    /// The divisor is validated positive in [`Self::try_bracketed`].
    pub fn covered_ratio(&self) -> f64 {
        self.covered_ratio_bounds().0
    }
    /// The covered fraction's `(lower, upper)` bounds: the least covered
    /// area over the largest whole, and the most over the least (at most
    /// one).
    // A point is a point bit for bit: its ratio is divided once.
    #[allow(clippy::float_cmp)]
    pub fn covered_ratio_bounds(&self) -> (f64, f64) {
        if self.whole.0 == self.whole.1 && self.covered.0 == self.covered.1 {
            let ratio = self.covered.0 / self.whole.0;
            return (ratio, ratio);
        }
        (
            self.covered.0 / self.whole.1,
            (self.covered.1 / self.whole.0).min(1.0),
        )
    }
}

/// One connected region of storey floor that belongs to no space.
///
/// Each region is measured on its own, so a rule can judge a large hole apart
/// from small shafts on the same storey: a storey total would hide which one
/// misses the allowance.
#[derive(Clone, Debug, PartialEq)]
pub struct UnallocatedRegion {
    storey: ObjectId,
    area_square_metres: f64,
    elements: Vec<ObjectId>,
    floor_area_square_metres: Option<f64>,
}

impl UnallocatedRegion {
    /// A region of `area_square_metres` on `storey`, surrounded by
    /// `elements` (the bodies whose footprint meets its boundary).
    pub fn try_new(
        storey: ObjectId,
        area_square_metres: f64,
        mut elements: Vec<ObjectId>,
    ) -> Result<Self, SpaceError> {
        if !finite_non_negative(area_square_metres) {
            return Err(SpaceError::InvalidQuantity);
        }
        elements.sort();
        elements.dedup();
        Ok(Self {
            storey,
            area_square_metres,
            elements,
            floor_area_square_metres: None,
        })
    }
    /// States the storey's gross floor area the region is part of, so a
    /// rule can judge the region's share of it. A gross area smaller than
    /// the region, negative or non-finite is refused.
    pub fn with_floor_area(mut self, square_metres: f64) -> Result<Self, SpaceError> {
        if !finite_non_negative(square_metres) || square_metres < self.area_square_metres {
            return Err(SpaceError::InvalidQuantity);
        }
        self.floor_area_square_metres = Some(square_metres);
        Ok(self)
    }
    pub fn storey(&self) -> &ObjectId {
        &self.storey
    }
    pub fn area_square_metres(&self) -> f64 {
        self.area_square_metres
    }
    /// The storey's gross floor area, when the service states it.
    pub fn floor_area_square_metres(&self) -> Option<f64> {
        self.floor_area_square_metres
    }
    /// The elements surrounding the region, in canonical order.
    pub fn elements(&self) -> &[ObjectId] {
        &self.elements
    }
    /// The share of their storey's gross floor area that one storey's
    /// `regions` cover together: their summed area over the gross area the
    /// service states, the same on every region. `None` when the regions
    /// are none, do not state one positive gross area alike, or sum to no
    /// finite area: the share is then undefined, never zero. The one
    /// reading `space-validation`'s `maximum_unallocated_share` judges and
    /// `axioval:measured` `unallocated_share` answers.
    pub fn storey_share(regions: &[&Self]) -> Option<UnallocatedShare> {
        let gross = regions.first()?.floor_area_square_metres.filter(|gross| {
            *gross > 0.0
                && regions
                    .iter()
                    .all(|region| region.floor_area_square_metres == Some(*gross))
        })?;
        // Summed and divided outward, so both hold the exact values.
        let area = regions
            .iter()
            .try_fold(Interval::point(0.0), |total, region| {
                total.plus(Interval::point(region.area_square_metres)).ok()
            })?;
        let share = area.divided_by(Interval::point(gross)).ok()?;
        Some(UnallocatedShare {
            area,
            gross_floor_area_square_metres: gross,
            share,
        })
    }
}

/// One storey's unallocated floor against its gross floor area
/// ([`UnallocatedRegion::storey_share`]), each an interval sure to hold the
/// exact value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnallocatedShare {
    area: Interval,
    gross_floor_area_square_metres: f64,
    share: Interval,
}

impl UnallocatedShare {
    /// The regions' summed area, `(lower, upper)` in square metres.
    pub fn area_square_metres(&self) -> (f64, f64) {
        (self.area.lower, self.area.upper)
    }
    /// The storey's gross floor area the service states.
    pub fn gross_floor_area_square_metres(&self) -> f64 {
        self.gross_floor_area_square_metres
    }
    /// The summed area's share of the gross floor area, `(lower, upper)`;
    /// incoherent regions summing past the gross area give a share above 1.
    pub fn share(&self) -> (f64, f64) {
        (self.share.lower, self.share.upper)
    }
}

/// Which horizontal caps a model actually has elements for.
///
/// Counts, not a decision about which checks to run: the capability decides
/// that a cap check without any slab is not worth reporting per space.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportCounts {
    slabs: usize,
    roofs: usize,
    buildings: Vec<ObjectId>,
}

impl SupportCounts {
    pub fn new(slabs: usize, roofs: usize, mut buildings: Vec<ObjectId>) -> Self {
        buildings.sort();
        buildings.dedup();
        Self {
            slabs,
            roofs,
            buildings,
        }
    }
    pub fn slabs(&self) -> usize {
        self.slabs
    }
    pub fn roofs(&self) -> usize {
        self.roofs
    }
    pub fn buildings(&self) -> &[ObjectId] {
        &self.buildings
    }
}

/// Which horizontal cap of a space is being measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Top,
    Bottom,
}

/// A request for the coverage of one cap of a space.
///
/// Which elements may form the cap is a policy choice, so a rule can state it:
/// [`Self::with_elements`] carries the rule's selection, and the service then
/// considers exactly those elements. Without it the service falls back to the
/// cap elements its host declared (slabs for the bottom, slabs and roofs for
/// the top).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapRequest {
    cap: Cap,
    elements: Option<Vec<ObjectId>>,
}

impl CapRequest {
    /// Asks for `cap`, bounded by the host-declared cap elements.
    #[must_use]
    pub fn new(cap: Cap) -> Self {
        Self {
            cap,
            elements: None,
        }
    }
    /// Asks for `cap` bounded by exactly these elements, in canonical order.
    #[must_use]
    pub fn with_elements(mut self, mut elements: Vec<ObjectId>) -> Self {
        elements.sort();
        elements.dedup();
        self.elements = Some(elements);
        self
    }
    pub fn cap(&self) -> Cap {
        self.cap
    }
    /// The elements the caller chose, or `None` to use the host's declaration.
    pub fn elements(&self) -> Option<&[ObjectId]> {
        self.elements.as_deref()
    }
}

/// A request for the uncovered runs of a space's boundary.
///
/// Which elements bound a space is a policy choice: [`Self::with_elements`]
/// carries the rule's selection, and the service then counts exactly those
/// elements as covering. Without it the service counts every body that is
/// not a space.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundaryRequest {
    elements: Option<Vec<ObjectId>>,
}

impl BoundaryRequest {
    /// Asks for the gaps no body other than a space covers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Asks for the gaps none of these elements covers, in canonical order.
    #[must_use]
    pub fn with_elements(mut self, elements: Vec<ObjectId>) -> Self {
        self.elements = Some(canonical(elements));
        self
    }
    /// The elements the caller chose, or `None` for the host's default.
    pub fn elements(&self) -> Option<&[ObjectId]> {
        self.elements.as_deref()
    }
}

/// A request for the bodies a space overlaps.
///
/// Which elements a space must not intersect is a policy choice:
/// [`Self::with_elements`] carries the rule's selection, and the service then
/// measures overlaps with exactly those elements. Without it every other body
/// is measured.
///
/// [`Self::with_contact_width`] carries the tolerance the overlaps are read
/// at, so the service measures the depth of each shared plan region at it
/// (#425). Without it the width is zero: any overlap region is deeper.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlapRequest {
    elements: Option<Vec<ObjectId>>,
    contact_width_metres: f64,
}

impl OverlapRequest {
    /// Asks for overlaps with every other body.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Asks for overlaps with exactly these elements, in canonical order.
    #[must_use]
    pub fn with_elements(mut self, elements: Vec<ObjectId>) -> Self {
        self.elements = Some(canonical(elements));
        self
    }
    /// The elements the caller chose, or `None` for every other body.
    pub fn elements(&self) -> Option<&[ObjectId]> {
        self.elements.as_deref()
    }
    /// Asks for each shared plan region's depth at this contact width. The
    /// service refuses a width that is negative or not finite.
    #[must_use]
    pub fn with_contact_width(mut self, metres: f64) -> Self {
        self.contact_width_metres = metres;
        self
    }
    /// The width, in metres, the depth is measured at.
    pub fn contact_width_metres(&self) -> f64 {
        self.contact_width_metres
    }
}

fn canonical(mut elements: Vec<ObjectId>) -> Vec<ObjectId> {
    elements.sort();
    elements.dedup();
    elements
}

/// Measures the geometry a space-validation policy reasons about.
///
/// ADR 0004: every method returns a measurement. None returns a finding, and
/// each aspect fails independently.
pub trait SpaceService: Send + Sync + 'static {
    /// Spaces whose body coincides with `space`.
    // gate: measures number
    fn measure_duplicates(&self, space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError>;
    /// The clear height of `space`.
    // gate: measures length
    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError>;
    /// Uncovered runs of the space boundary, covered only by the elements
    /// `request` names or, when it names none, by any body but a space.
    // gate: measures length, area
    fn measure_boundary_gaps(
        &self,
        space: &ObjectId,
        request: &BoundaryRequest,
    ) -> Result<Vec<BoundaryGap>, SpaceError>;
    /// Bodies overlapping `space`: the elements `request` names or, when it
    /// names none, every other body. A partial overlap higher than
    /// `request.contact_width_metres()` whose upper area reaches
    /// [`OVERLAP_AREA_EPSILON_M2`] carries a [`PlanDepth`] measured at that
    /// width; without one it reads as undecided.
    // gate: measures volume, area
    fn measure_overlaps(
        &self,
        space: &ObjectId,
        request: &OverlapRequest,
    ) -> Result<Vec<SpaceOverlap>, SpaceError>;
    /// Coverage of one horizontal cap of `space`, by the elements `request`
    /// names or, when it names none, by the host-declared cap elements.
    // gate: measures area, number
    fn measure_cap_coverage(
        &self,
        space: &ObjectId,
        request: &CapRequest,
    ) -> Result<CapCoverage, SpaceError>;
    /// Floor area belonging to no space, one entry per connected region of
    /// each storey.
    // gate: measures area, number
    fn measure_unallocated_regions(&self) -> Result<Vec<UnallocatedRegion>, SpaceError>;
    /// Counts of the elements that can form horizontal caps.
    // gate: measures number
    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError>;
    /// Evidence backing this service's measurements.
    // gate: reads
    fn evidence(&self) -> Evidence;
}

/// Registry handle for a [`SpaceService`].
#[derive(Clone)]
pub struct SpaceServiceHandle(Arc<dyn SpaceService>);

impl SpaceServiceHandle {
    pub fn new(service: Arc<dyn SpaceService>) -> Self {
        Self(Arc::new(Timed(service)))
    }
    pub fn get(&self) -> &dyn SpaceService {
        self.0.as_ref()
    }
}

/// The host's service with every measurement counted against the rule
/// being timed ([`crate::timing::host`]). The handle hands out the service
/// itself, so its calls are timed here rather than in the handle.
struct Timed(Arc<dyn SpaceService>);

impl SpaceService for Timed {
    fn measure_duplicates(&self, space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError> {
        crate::timing::host("space", || self.0.measure_duplicates(space))
    }
    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError> {
        crate::timing::host("space", || self.0.measure_clear_height(space))
    }
    fn measure_boundary_gaps(
        &self,
        space: &ObjectId,
        request: &BoundaryRequest,
    ) -> Result<Vec<BoundaryGap>, SpaceError> {
        crate::timing::host("space", || self.0.measure_boundary_gaps(space, request))
    }
    fn measure_overlaps(
        &self,
        space: &ObjectId,
        request: &OverlapRequest,
    ) -> Result<Vec<SpaceOverlap>, SpaceError> {
        crate::timing::host("space", || self.0.measure_overlaps(space, request))
    }
    fn measure_cap_coverage(
        &self,
        space: &ObjectId,
        request: &CapRequest,
    ) -> Result<CapCoverage, SpaceError> {
        crate::timing::host("space", || self.0.measure_cap_coverage(space, request))
    }
    fn measure_unallocated_regions(&self) -> Result<Vec<UnallocatedRegion>, SpaceError> {
        crate::timing::host("space", || self.0.measure_unallocated_regions())
    }
    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError> {
        crate::timing::host("space", || self.0.measure_support_counts())
    }
    fn evidence(&self) -> Evidence {
        self.0.evidence()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }
    fn oid(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn overlap(
        area: (f64, f64),
        height: f64,
        containment: Containment,
        depth: Option<(f64, bool)>,
    ) -> SpaceOverlap {
        let depth = depth.map(|(width, deeper)| PlanDepth::try_new(width, deeper).unwrap());
        SpaceOverlap::try_new(oid("other"), true, area, height, containment, depth).unwrap()
    }

    /// Containment always intersects; a partial overlap is contact within the
    /// tolerance in height, area or depth.
    #[test]
    fn an_overlap_reads_contact_within_the_tolerance_in_height_area_or_depth() {
        use OverlapReading::{Contact, Intersection, Undecided};
        let t = 0.005;
        let reading =
            |area, height, depth| overlap(area, height, Containment::Partial, depth).reading(t);
        for containment in [
            Containment::SubjectInsideOther,
            Containment::OtherInsideSubject,
        ] {
            assert_eq!(
                overlap((0.0, 0.0), 0.0, containment, None).reading(t),
                Intersection
            );
        }
        assert_eq!(reading((1.0, 1.0), 0.01, Some((t, true))), Intersection);
        assert_eq!(reading((1.0, 1.0), t, Some((t, true))), Contact);
        assert_eq!(
            reading((1.0e-8, 1.0e-8), 1.0, Some((t, true))),
            Intersection
        );
        assert_eq!(reading((0.0, 0.99e-8), 1.0, Some((t, true))), Contact);
        assert_eq!(reading((5.0e-9, 2.0e-8), 1.0, Some((t, true))), Undecided);
        assert_eq!(reading((1.0, 1.0), 1.0, None), Undecided);
        assert_eq!(reading((1.0, 1.0), 1.0, Some((t, false))), Contact);
    }

    /// A depth speaks only toward the width it was measured at.
    #[test]
    fn a_depth_reads_only_toward_its_width() {
        use OverlapReading::{Contact, Intersection, Undecided};
        let reading = |deeper, t| {
            overlap((1.0, 1.0), 1.0, Containment::Partial, Some((0.01, deeper))).reading(t)
        };
        assert_eq!(reading(true, 0.005), Intersection);
        assert_eq!(reading(true, 0.02), Undecided);
        assert_eq!(reading(false, 0.02), Contact);
        assert_eq!(reading(false, 0.005), Undecided);
        assert_eq!(reading(true, f64::NAN), Undecided);
    }

    #[test]
    fn intersections_refuse_an_undecided_overlap() {
        let deep = overlap((1.0, 1.0), 1.0, Containment::Partial, Some((0.005, true)));
        let touching = overlap((1.0, 1.0), 1.0, Containment::Partial, Some((0.005, false)));
        let unknown = overlap((1.0, 1.0), 1.0, Containment::Partial, None);
        assert_eq!(
            SpaceOverlap::intersections(vec![touching.clone(), deep.clone()], 0.005),
            Ok(vec![deep.clone()])
        );
        assert_eq!(
            SpaceOverlap::intersections(vec![deep, touching, unknown], 0.005),
            Err(SpaceError::Refused(UNDECIDED_OVERLAP))
        );
    }

    /// A storey's share holds the exact one, and is undefined without one
    /// gross area every region states alike.
    #[test]
    #[allow(clippy::float_cmp)]
    fn a_storey_share_needs_one_gross_area() {
        let region = |area: f64, gross: Option<f64>| {
            let region = UnallocatedRegion::try_new(oid("storey"), area, Vec::new()).unwrap();
            match gross {
                Some(gross) => region.with_floor_area(gross).unwrap(),
                None => region,
            }
        };
        let (two, three) = (region(2.0, Some(100.0)), region(3.0, Some(100.0)));
        let share = UnallocatedRegion::storey_share(&[&two, &three]).unwrap();
        let (lower, upper) = share.share();
        assert!(lower <= 0.05 && 0.05 <= upper && upper - lower < 1e-15);
        assert_eq!(share.area_square_metres(), (5.0, 5.0));
        assert_eq!(share.gross_floor_area_square_metres(), 100.0);
        assert!(UnallocatedRegion::storey_share(&[]).is_none());
        assert!(UnallocatedRegion::storey_share(&[&region(2.0, None)]).is_none());
        let other = region(2.0, Some(90.0));
        assert!(UnallocatedRegion::storey_share(&[&two, &other]).is_none());
        assert!(UnallocatedRegion::storey_share(&[&region(0.0, Some(0.0))]).is_none());
    }

    /// A refusal names its aspect and the first unmeasured objects in
    /// identity order, then counts the rest.
    #[test]
    fn an_unmeasured_refusal_names_the_aspect_and_a_bounded_list() {
        let few = SpaceError::unmeasured(
            SpaceAspect::CapCoverage(Cap::Top),
            vec![oid("slab"), oid("roof"), oid("slab")],
        );
        assert_eq!(
            few.to_string(),
            "space top cap coverage is unavailable: 2 unmeasured object(s) could change it \
             (cad:m/roof, cad:m/slab)"
        );
        let many = SpaceError::unmeasured(
            SpaceAspect::Overlaps,
            (0..8).map(|i| oid(&format!("o{i}"))).collect(),
        );
        assert!(
            many.to_string()
                .ends_with("(cad:m/o0, cad:m/o1, cad:m/o2, cad:m/o3, cad:m/o4, +3 more)"),
            "{many}"
        );
        let SpaceError::Unmeasured(blocked) = many else {
            panic!("an unmeasured refusal");
        };
        assert_eq!(blocked.aspect(), SpaceAspect::Overlaps);
        assert_eq!(blocked.objects().len(), 8);
    }

    #[test]
    fn cap_covered_over_its_own_area_is_refused() {
        assert_eq!(
            CapCoverage::try_new(10.0, 11.0, Vec::new()),
            Err(SpaceError::InvalidQuantity)
        );
    }

    #[test]
    fn zero_cap_area_is_refused_so_the_ratio_cannot_divide_by_zero() {
        assert_eq!(
            CapCoverage::try_new(0.0, 0.0, Vec::new()),
            Err(SpaceError::InvalidQuantity)
        );
    }

    /// A geometry kernel can hand back -0.0 for an empty intersection. It
    /// compares equal to zero but renders as "-0.0", so an uncovered cap would
    /// be reported as "-0.0% covered".
    #[test]
    fn negative_zero_coverage_is_normalised() {
        let coverage = CapCoverage::try_new(10.0, -0.0, Vec::new()).unwrap();
        assert_eq!(format!("{:.1}", coverage.covered_ratio() * 100.0), "0.0");
    }

    #[test]
    fn cap_request_elements_are_canonical_and_absent_by_default() {
        assert_eq!(CapRequest::new(Cap::Top).elements(), None);
        let request =
            CapRequest::new(Cap::Bottom).with_elements(vec![oid("b"), oid("a"), oid("b")]);
        assert_eq!(request.cap(), Cap::Bottom);
        assert_eq!(request.elements(), Some(&[oid("a"), oid("b")][..]));
        // An explicit empty selection is a statement, not an absence.
        assert_eq!(
            CapRequest::new(Cap::Top)
                .with_elements(Vec::new())
                .elements(),
            Some(&[][..])
        );
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn boundary_and_overlap_requests_are_canonical_and_absent_by_default() {
        assert_eq!(BoundaryRequest::new().elements(), None);
        assert_eq!(OverlapRequest::new().elements(), None);
        assert_eq!(OverlapRequest::new().contact_width_metres(), 0.0);
        assert_eq!(
            OverlapRequest::new()
                .with_contact_width(0.005)
                .contact_width_metres(),
            0.005
        );
        let ids = vec![oid("b"), oid("a"), oid("b")];
        assert_eq!(
            BoundaryRequest::new().with_elements(ids.clone()).elements(),
            Some(&[oid("a"), oid("b")][..])
        );
        assert_eq!(
            OverlapRequest::new().with_elements(ids).elements(),
            Some(&[oid("a"), oid("b")][..])
        );
    }

    #[test]
    fn cap_ratio_is_exact() {
        let coverage = CapCoverage::try_new(4.0, 1.0, Vec::new()).unwrap();
        assert!((coverage.covered_ratio() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn element_lists_are_normalised() {
        let gap = BoundaryGap::try_new(1.0, vec![oid("w2"), oid("w1"), oid("w2")]).unwrap();
        assert_eq!(gap.elements(), &[oid("w1"), oid("w2")]);
    }

    #[test]
    fn non_finite_quantities_are_refused() {
        assert!(
            ClearHeightEvidence::try_new(oid("s"), f64::NAN, Evidence::exact(source(), "h"))
                .is_err()
        );
        assert!(BoundaryGap::try_new(f64::INFINITY, Vec::new()).is_err());
        let overlap = |area, depth| {
            SpaceOverlap::try_new(oid("o"), false, area, 1.0, Containment::Partial, depth)
        };
        assert!(overlap((-1.0, 1.0), None).is_err());
        assert!(overlap((2.0, 1.0), None).is_err());
        assert!(overlap((f64::NAN, 1.0), None).is_err());
        assert!(overlap((0.0, f64::INFINITY), None).is_err());
        assert!(PlanDepth::try_new(-1.0, true).is_err());
        assert!(PlanDepth::try_new(f64::NAN, false).is_err());
        assert!(UnallocatedRegion::try_new(oid("st"), f64::NAN, Vec::new()).is_err());
        let region = UnallocatedRegion::try_new(oid("st"), 5.0, Vec::new()).unwrap();
        assert_eq!(region.floor_area_square_metres(), None);
        // A gross area smaller than the region cannot hold it.
        assert!(region.clone().with_floor_area(4.0).is_err());
        assert!(region.clone().with_floor_area(f64::INFINITY).is_err());
        assert_eq!(
            region
                .with_floor_area(100.0)
                .unwrap()
                .floor_area_square_metres(),
            Some(100.0)
        );
    }

    #[test]
    fn inexact_evidence_is_refused() {
        assert_eq!(
            ClearHeightEvidence::try_new(
                oid("s"),
                2.5,
                Evidence {
                    source: source(),
                    locator: "h".into(),
                    exact: false,
                    witness: None,
                },
            ),
            Err(SpaceError::InexactEvidence)
        );
    }
}
