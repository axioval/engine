//! Source-neutral surface-contact evidence.
//!
//! ADR 0004: a service returns what was *measured*; a capability decides what
//! it means. This is the measurement half of the slab-contact decomposition.
//!
//! Two things deliberately do **not** cross this seam:
//!
//! - **No verdict state.** The source provider returned a four-state enum in
//!   which `IgnoredSmall` was a threshold decision and `Skipped` a scope
//!   decision. Both are policy. Scope belongs to the selector, and a small
//!   contact area is just a small measured number.
//! - **No rounding.** The source rounded the contact ratio to two decimals
//!   before the rule compared it, so a value could round *up* across the
//!   declared minimum and pass. Rounding is a presentation choice and cannot
//!   be allowed to change a verdict, so the measurement stays exact.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::services::reviewable_exact_evidence;

/// Why a contact measurement could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContactError {
    /// Areas are negative, non-finite, or the contact exceeds the whole.
    #[error("contact areas must be finite, non-negative and contained")]
    InvalidAreas,
    /// The evidence backing the measurement was not exact and reviewable.
    #[error("contact evidence must be exact and reviewable")]
    InexactEvidence,
    /// The adapter cannot measure contact for this object.
    #[error("contact measurement is unavailable for the requested scope")]
    Unavailable,
    /// The object's body has no direction the adapter can orient against.
    #[error("object body has no checkable orientation")]
    UncheckableOrientation,
}

/// Which side of the subject the contacting surface must lie on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactSide {
    Above,
    Below,
}

/// Tolerances describing what counts as touching.
///
/// These are measurement inputs, not thresholds: they define contact, rather
/// than judging whether enough of it exists.
#[derive(Clone, Copy, Debug, PartialEq)]
// The shared unit suffix is the point: these are lengths and an area in fixed
// units, and naming the unit on each field is what stops a millimetre or a
// square metre being passed where metres are meant.
#[allow(clippy::struct_field_names)]
pub struct ContactTolerance {
    maximum_gap_metres: f64,
    maximum_intersection_metres: f64,
    minimum_polygon_area_square_metres: f64,
}

impl ContactTolerance {
    pub fn try_new(
        maximum_gap_metres: f64,
        maximum_intersection_metres: f64,
        minimum_polygon_area_square_metres: f64,
    ) -> Result<Self, ContactError> {
        let ok = |v: f64| v.is_finite() && v >= 0.0;
        if !ok(maximum_gap_metres)
            || !ok(maximum_intersection_metres)
            || !ok(minimum_polygon_area_square_metres)
        {
            return Err(ContactError::InvalidAreas);
        }
        Ok(Self {
            maximum_gap_metres,
            maximum_intersection_metres,
            minimum_polygon_area_square_metres,
        })
    }
    pub fn maximum_gap_metres(&self) -> f64 {
        self.maximum_gap_metres
    }
    pub fn maximum_intersection_metres(&self) -> f64 {
        self.maximum_intersection_metres
    }
    pub fn minimum_polygon_area_square_metres(&self) -> f64 {
        self.minimum_polygon_area_square_metres
    }
}

/// A request for the contact measurement of one object.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactRequest {
    subject: ObjectId,
    side: ContactSide,
    tolerance: ContactTolerance,
}

impl ContactRequest {
    pub fn new(subject: ObjectId, side: ContactSide, tolerance: ContactTolerance) -> Self {
        Self {
            subject,
            side,
            tolerance,
        }
    }
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }
    pub fn side(&self) -> ContactSide {
        self.side
    }
    pub fn tolerance(&self) -> ContactTolerance {
        self.tolerance
    }
}

/// How much of a subject's face is in contact, and with what.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactEvidence {
    request: ContactRequest,
    whole_area_square_metres: f64,
    contact_area_square_metres: f64,
    nearest_distance_metres: Option<f64>,
    touching: Vec<ObjectId>,
    evidence: Evidence,
}

impl ContactEvidence {
    /// Rejects incoherent areas and unreviewable evidence, so an adapter
    /// cannot launder an estimate into the engine as fact.
    pub fn try_new(
        request: ContactRequest,
        whole_area_square_metres: f64,
        contact_area_square_metres: f64,
        nearest_distance_metres: Option<f64>,
        mut touching: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, ContactError> {
        let finite_non_negative = |v: f64| v.is_finite() && v >= 0.0;
        if !finite_non_negative(whole_area_square_metres)
            || !finite_non_negative(contact_area_square_metres)
            || whole_area_square_metres <= 0.0
            // A face cannot touch over more than its own area; if it appears
            // to, the measurement is wrong and must not reach a rule.
            || contact_area_square_metres > whole_area_square_metres
        {
            return Err(ContactError::InvalidAreas);
        }
        if nearest_distance_metres.is_some_and(|d| !finite_non_negative(d)) {
            return Err(ContactError::InvalidAreas);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(ContactError::InexactEvidence);
        }
        touching.sort();
        touching.dedup();
        Ok(Self {
            request,
            whole_area_square_metres,
            contact_area_square_metres,
            nearest_distance_metres,
            touching,
            evidence,
        })
    }

    pub fn request(&self) -> &ContactRequest {
        &self.request
    }
    pub fn whole_area_square_metres(&self) -> f64 {
        self.whole_area_square_metres
    }
    pub fn contact_area_square_metres(&self) -> f64 {
        self.contact_area_square_metres
    }
    /// Distance to the nearest candidate when nothing is touching.
    pub fn nearest_distance_metres(&self) -> Option<f64> {
        self.nearest_distance_metres
    }
    /// The objects found in contact, sorted and deduplicated.
    pub fn touching(&self) -> &[ObjectId] {
        &self.touching
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// Fraction of the face in contact, computed exactly.
    ///
    /// The divisor is validated positive in [`Self::try_new`], so this cannot
    /// divide by zero.
    pub fn contact_ratio(&self) -> f64 {
        self.contact_area_square_metres / self.whole_area_square_metres
    }
}

/// Measures surface contact between model objects.
///
/// ADR 0004: every method returns a measurement. None returns a finding.
pub trait ContactService: Send + Sync + 'static {
    fn measure_contact(&self, request: &ContactRequest) -> Result<ContactEvidence, ContactError>;
}

/// Registry handle for a [`ContactService`].
#[derive(Clone)]
pub struct ContactServiceHandle(Arc<dyn ContactService>);

impl ContactServiceHandle {
    pub fn new(service: Arc<dyn ContactService>) -> Self {
        Self(service)
    }
    pub fn measure_contact(
        &self,
        request: &ContactRequest,
    ) -> Result<ContactEvidence, ContactError> {
        self.0.measure_contact(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("cad", "m").unwrap(), local).unwrap()
    }
    fn request() -> ContactRequest {
        ContactRequest::new(
            id("wall"),
            ContactSide::Above,
            ContactTolerance::try_new(0.01, 0.01, 0.001).unwrap(),
        )
    }
    fn evidence() -> Evidence {
        Evidence::exact(SourceId::new("cad", "m").unwrap(), "contact:wall")
    }

    /// Contact larger than the face is physically impossible; accepting it
    /// would let a ratio above 1.0 satisfy any minimum.
    #[test]
    fn contact_exceeding_the_whole_face_is_refused() {
        assert_eq!(
            ContactEvidence::try_new(request(), 10.0, 11.0, None, Vec::new(), evidence()),
            Err(ContactError::InvalidAreas)
        );
    }

    #[test]
    fn zero_or_non_finite_whole_area_is_refused() {
        for whole in [0.0, f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(
                ContactEvidence::try_new(request(), whole, 0.0, None, Vec::new(), evidence()),
                Err(ContactError::InvalidAreas)
            );
        }
    }

    /// The ratio is exact. Rounding it here would let a value below a declared
    /// minimum round up and silently pass.
    #[test]
    fn contact_ratio_is_exact_and_unrounded() {
        let measured =
            ContactEvidence::try_new(request(), 3.0, 1.0, None, Vec::new(), evidence()).unwrap();
        assert!((measured.contact_ratio() - 1.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn touching_objects_are_sorted_and_deduplicated() {
        let measured = ContactEvidence::try_new(
            request(),
            4.0,
            2.0,
            None,
            vec![id("slab-b"), id("slab-a"), id("slab-b")],
            evidence(),
        )
        .unwrap();
        assert_eq!(measured.touching(), &[id("slab-a"), id("slab-b")]);
    }
}
