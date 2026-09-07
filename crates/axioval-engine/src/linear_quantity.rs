//! Source-neutral linear-quantity evidence.
//!
//! ADR 0004: a service returns what was *measured*; a capability decides what
//! it means. This is the measurement half of the shelf-capacity decomposition:
//! the adapter reports how many running metres of shelving a space contains,
//! and never whether that satisfies a requirement.
//!
//! The quantity is an interval rather than a scalar so an adapter can report
//! honest bounds when its measurement is approximate. A capability that needs
//! exactness asks for it explicitly via [`LinearInterval::is_exact`].

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};

use crate::services::reviewable_exact_evidence;

/// Why a linear quantity could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinearQuantityError {
    /// The bounds are negative, non-finite, or inverted.
    #[error("linear interval must be finite, non-negative and ordered")]
    InvalidInterval,
    /// The evidence backing the measurement was not exact and reviewable.
    #[error("linear quantity evidence must be exact and reviewable")]
    InexactEvidence,
    /// The adapter cannot measure this quantity for this object.
    #[error("linear quantity is unavailable for the requested scope")]
    Unavailable,
    /// The requested arrangement is not physically realisable.
    #[error("shelf geometry must be positive, finite and ordered")]
    InvalidGeometry,
}

/// A measured length, in metres, bounded below and above.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearInterval {
    lower_metres: f64,
    upper_metres: f64,
}

impl LinearInterval {
    /// Bounds for an approximate measurement.
    pub fn try_new(lower: f64, upper: f64) -> Result<Self, LinearQuantityError> {
        let valid = |v: f64| v.is_finite() && v >= 0.0;
        if !valid(lower) || !valid(upper) || lower > upper {
            return Err(LinearQuantityError::InvalidInterval);
        }
        Ok(Self {
            lower_metres: lower,
            upper_metres: upper,
        })
    }

    /// A measurement the adapter can vouch for exactly.
    pub fn exact(metres: f64) -> Result<Self, LinearQuantityError> {
        Self::try_new(metres, metres)
    }

    pub fn lower_metres(&self) -> f64 {
        self.lower_metres
    }

    pub fn upper_metres(&self) -> f64 {
        self.upper_metres
    }

    /// True when the interval collapses to a single value.
    ///
    /// Exact bit-equality is the intended test: the bounds are only equal when
    /// an adapter constructed them from one measurement via [`Self::exact`].
    /// A tolerance here would let a genuine range masquerade as exact.
    #[allow(clippy::float_cmp)]
    pub fn is_exact(&self) -> bool {
        self.lower_metres == self.upper_metres
    }

    /// Whether the whole interval clears `minimum`.
    ///
    /// Comparison lives with the caller, but the *interval* semantics live
    /// here: a partially-clearing interval is not a pass, and saying so once
    /// stops each capability inventing its own rounding.
    pub fn definitely_at_least(&self, minimum: f64) -> bool {
        self.lower_metres >= minimum
    }

    /// Whether no part of the interval clears `minimum`.
    pub fn definitely_below(&self, minimum: f64) -> bool {
        self.upper_metres < minimum
    }
}

/// The physical shelving arrangement whose run length is being measured.
///
/// These are *geometry inputs*, not thresholds: they describe the shelf being
/// measured, so they belong to the measurement request. The minimum a space
/// must provide is policy and stays with the capability. Keeping the two apart
/// is what stops a threshold drifting back behind the evidence seam.
#[derive(Clone, Copy, Debug, PartialEq)]
// The shared `_metres` suffix is the point: every field is a length in the
// same unit, and naming it on each one is what stops a millimetre value being
// passed where metres are meant. Dropping the suffix would trade a real
// safety property for brevity.
#[allow(clippy::struct_field_names)]
pub struct ShelfGeometry {
    depth_metres: f64,
    horizontal_spacing_metres: f64,
    vertical_spacing_metres: f64,
    bottom_elevation_metres: f64,
    top_elevation_metres: f64,
    door_clearance_metres: f64,
}

impl ShelfGeometry {
    /// Rejects a physically impossible arrangement.
    pub fn try_new(
        depth_metres: f64,
        horizontal_spacing_metres: f64,
        vertical_spacing_metres: f64,
        bottom_elevation_metres: f64,
        top_elevation_metres: f64,
        door_clearance_metres: f64,
    ) -> Result<Self, LinearQuantityError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let non_negative = |v: f64| v.is_finite() && v >= 0.0;
        if !positive(depth_metres)
            || !positive(horizontal_spacing_metres)
            || !positive(vertical_spacing_metres)
            || !non_negative(bottom_elevation_metres)
            || !non_negative(door_clearance_metres)
            || !top_elevation_metres.is_finite()
            || top_elevation_metres <= bottom_elevation_metres
        {
            return Err(LinearQuantityError::InvalidGeometry);
        }
        Ok(Self {
            depth_metres,
            horizontal_spacing_metres,
            vertical_spacing_metres,
            bottom_elevation_metres,
            top_elevation_metres,
            door_clearance_metres,
        })
    }
    pub fn depth_metres(&self) -> f64 {
        self.depth_metres
    }
    pub fn horizontal_spacing_metres(&self) -> f64 {
        self.horizontal_spacing_metres
    }
    pub fn vertical_spacing_metres(&self) -> f64 {
        self.vertical_spacing_metres
    }
    pub fn bottom_elevation_metres(&self) -> f64 {
        self.bottom_elevation_metres
    }
    pub fn top_elevation_metres(&self) -> f64 {
        self.top_elevation_metres
    }
    pub fn door_clearance_metres(&self) -> f64 {
        self.door_clearance_metres
    }
}

/// What linear quantity is being asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LinearQuantityKind {
    /// Total running length of shelving fitting the given arrangement.
    ShelfRunningLength(ShelfGeometry),
}

impl LinearQuantityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinearQuantityKind::ShelfRunningLength(_) => "shelf-running-length",
        }
    }
}

/// A request for one linear measurement of one object.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearQuantityRequest {
    scope: ObjectId,
    kind: LinearQuantityKind,
}

impl LinearQuantityRequest {
    pub fn new(scope: ObjectId, kind: LinearQuantityKind) -> Self {
        Self { scope, kind }
    }
    pub fn scope(&self) -> &ObjectId {
        &self.scope
    }
    pub fn kind(&self) -> LinearQuantityKind {
        self.kind
    }
}

/// A measured linear quantity with the evidence that supports it.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearQuantityEvidence {
    request: LinearQuantityRequest,
    measured: LinearInterval,
    evidence: Evidence,
}

impl LinearQuantityEvidence {
    /// Rejects evidence that is not exact and reviewable, so an adapter
    /// cannot launder an estimate into the engine as fact.
    pub fn try_new(
        request: LinearQuantityRequest,
        measured: LinearInterval,
        evidence: Evidence,
    ) -> Result<Self, LinearQuantityError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(LinearQuantityError::InexactEvidence);
        }
        Ok(Self {
            request,
            measured,
            evidence,
        })
    }
    pub fn request(&self) -> &LinearQuantityRequest {
        &self.request
    }
    pub fn measured(&self) -> LinearInterval {
        self.measured
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures linear quantities of model objects.
///
/// ADR 0004: every method returns a measurement. None returns a finding.
pub trait LinearQuantityService: Send + Sync + 'static {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError>;
}

/// Registry handle for a [`LinearQuantityService`].
#[derive(Clone)]
pub struct LinearQuantityServiceHandle(Arc<dyn LinearQuantityService>);

impl LinearQuantityServiceHandle {
    pub fn new(service: Arc<dyn LinearQuantityService>) -> Self {
        Self(service)
    }
    pub fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        self.0.measure_linear_quantity(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_rejects_inverted_negative_and_non_finite_bounds() {
        assert!(LinearInterval::try_new(2.0, 1.0).is_err());
        assert!(LinearInterval::try_new(-1.0, 1.0).is_err());
        assert!(LinearInterval::try_new(0.0, f64::NAN).is_err());
        assert!(LinearInterval::try_new(0.0, f64::INFINITY).is_err());
        assert!(LinearInterval::try_new(0.0, 0.0).is_ok());
    }

    /// An approximate interval straddling the minimum is neither a pass nor a
    /// definite failure. Collapsing that to a boolean is how an estimate turns
    /// into a false verdict.
    #[test]
    fn straddling_interval_is_neither_pass_nor_definite_failure() {
        let straddles = LinearInterval::try_new(9.0, 11.0).unwrap();
        assert!(!straddles.definitely_at_least(10.0));
        assert!(!straddles.definitely_below(10.0));
        assert!(!straddles.is_exact());

        let clears = LinearInterval::exact(10.0).unwrap();
        assert!(clears.definitely_at_least(10.0));
        assert!(!clears.definitely_below(10.0));
        assert!(clears.is_exact());
    }
}
