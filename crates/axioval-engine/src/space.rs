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

use crate::services::reviewable_exact_evidence;

/// Why a space measurement could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
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
}

fn finite_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// The clear height of a space, in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearHeightEvidence {
    space: ObjectId,
    metres: f64,
    evidence: Evidence,
}

impl ClearHeightEvidence {
    pub fn try_new(space: ObjectId, metres: f64, evidence: Evidence) -> Result<Self, SpaceError> {
        if !finite_non_negative(metres) {
            return Err(SpaceError::InvalidQuantity);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(SpaceError::InexactEvidence);
        }
        Ok(Self {
            space,
            metres,
            evidence,
        })
    }
    pub fn space(&self) -> &ObjectId {
        &self.space
    }
    pub fn metres(&self) -> f64 {
        self.metres
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

/// A measured overlap between a space and another body.
#[derive(Clone, Debug, PartialEq)]
pub struct SpaceOverlap {
    other: ObjectId,
    other_is_space: bool,
    area_square_metres: f64,
    height_metres: f64,
    containment: Containment,
}

impl SpaceOverlap {
    pub fn try_new(
        other: ObjectId,
        other_is_space: bool,
        area_square_metres: f64,
        height_metres: f64,
        containment: Containment,
    ) -> Result<Self, SpaceError> {
        if !finite_non_negative(area_square_metres) || !finite_non_negative(height_metres) {
            return Err(SpaceError::InvalidQuantity);
        }
        Ok(Self {
            other,
            other_is_space,
            area_square_metres,
            height_metres,
            containment,
        })
    }
    pub fn other(&self) -> &ObjectId {
        &self.other
    }
    /// Whether the overlapping body is itself a space.
    pub fn other_is_space(&self) -> bool {
        self.other_is_space
    }
    pub fn area_square_metres(&self) -> f64 {
        self.area_square_metres
    }
    pub fn height_metres(&self) -> f64 {
        self.height_metres
    }
    pub fn containment(&self) -> Containment {
        self.containment
    }
}

/// How much of a space's horizontal cap is covered by elements.
#[derive(Clone, Debug, PartialEq)]
pub struct CapCoverage {
    whole_area_square_metres: f64,
    covered_area_square_metres: f64,
    elements: Vec<ObjectId>,
}

impl CapCoverage {
    pub fn try_new(
        whole_area_square_metres: f64,
        covered_area_square_metres: f64,
        mut elements: Vec<ObjectId>,
    ) -> Result<Self, SpaceError> {
        if !finite_non_negative(whole_area_square_metres)
            || !finite_non_negative(covered_area_square_metres)
            || whole_area_square_metres <= 0.0
            // A cap cannot be covered over more than its own area.
            || covered_area_square_metres > whole_area_square_metres
        {
            return Err(SpaceError::InvalidQuantity);
        }
        elements.sort();
        elements.dedup();
        Ok(Self {
            whole_area_square_metres,
            covered_area_square_metres,
            elements,
        })
    }
    pub fn whole_area_square_metres(&self) -> f64 {
        self.whole_area_square_metres
    }
    pub fn covered_area_square_metres(&self) -> f64 {
        self.covered_area_square_metres
    }
    pub fn elements(&self) -> &[ObjectId] {
        &self.elements
    }
    /// Fraction of the cap that is covered, computed exactly.
    ///
    /// The divisor is validated positive in [`Self::try_new`].
    pub fn covered_ratio(&self) -> f64 {
        self.covered_area_square_metres / self.whole_area_square_metres
    }
}

/// Floor area on a storey that belongs to no space.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreyResidual {
    storey: ObjectId,
    area_square_metres: f64,
    elements: Vec<ObjectId>,
}

impl StoreyResidual {
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
        })
    }
    pub fn storey(&self) -> &ObjectId {
        &self.storey
    }
    pub fn area_square_metres(&self) -> f64 {
        self.area_square_metres
    }
    pub fn elements(&self) -> &[ObjectId] {
        &self.elements
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

/// Measures the geometry a space-validation policy reasons about.
///
/// ADR 0004: every method returns a measurement. None returns a finding, and
/// each aspect fails independently.
pub trait SpaceService: Send + Sync + 'static {
    /// Spaces whose body coincides with `space`.
    fn measure_duplicates(&self, space: &ObjectId) -> Result<Vec<ObjectId>, SpaceError>;
    /// The clear height of `space`.
    fn measure_clear_height(&self, space: &ObjectId) -> Result<ClearHeightEvidence, SpaceError>;
    /// Uncovered runs of the space boundary.
    fn measure_boundary_gaps(&self, space: &ObjectId) -> Result<Vec<BoundaryGap>, SpaceError>;
    /// Bodies overlapping `space`.
    fn measure_overlaps(&self, space: &ObjectId) -> Result<Vec<SpaceOverlap>, SpaceError>;
    /// Coverage of one horizontal cap of `space`.
    fn measure_cap_coverage(&self, space: &ObjectId, cap: Cap) -> Result<CapCoverage, SpaceError>;
    /// Floor area belonging to no space, per storey.
    fn measure_storey_residuals(&self) -> Result<Vec<StoreyResidual>, SpaceError>;
    /// Counts of the elements that can form horizontal caps.
    fn measure_support_counts(&self) -> Result<SupportCounts, SpaceError>;
    /// Evidence backing this service's measurements.
    fn evidence(&self) -> Evidence;
}

/// Registry handle for a [`SpaceService`].
#[derive(Clone)]
pub struct SpaceServiceHandle(Arc<dyn SpaceService>);

impl SpaceServiceHandle {
    pub fn new(service: Arc<dyn SpaceService>) -> Self {
        Self(service)
    }
    pub fn get(&self) -> &dyn SpaceService {
        self.0.as_ref()
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
        assert!(SpaceOverlap::try_new(oid("o"), false, -1.0, 1.0, Containment::Partial).is_err());
        assert!(StoreyResidual::try_new(oid("st"), f64::NAN, Vec::new()).is_err());
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
                },
            ),
            Err(SpaceError::InexactEvidence)
        );
    }
}
