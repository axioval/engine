//! Leaf glazing: how much of each leaf of a door (or panel of a window) is
//! glazed, measured from the geometry of its body.
//!
//! ADR 0004: this seam measures areas. Whether a leaf is glazed enough is a
//! rule's judgement over them.
//!
//! The leaves are the ones the source states ([`crate::DoorLeaves`], from
//! [`crate::ObjectFrameService::leaves`]); the request carries them, so
//! the geometry is measured against the leaves the rule read. A service
//! answers per leaf, in the leaves' order:
//!
//! - the glazed area: the representation items of the body that are
//!   glazed (styled as, or associated with, a transparent or glass
//!   material), each assigned to the leaf whose extent along the door's
//!   width holds it, projected onto the plane of the closed leaf (`along`,
//!   `up`), overlaps counted once;
//! - the leaf's area: its stated width times the body's height along `up`.
//!
//! Both are intervals. A tessellated body is never exact: the interval
//! holds the chord deviation and the rounding of a projection off the
//! coordinate axes. A leaf is undecided ([`LeafGlazing::Undecided`]) where
//! an item that could be glazed reaches into it but cannot be told apart
//! from the frame (its material is undecided) or assigned to one leaf (it
//! straddles the leaf's edge); never zero, never a default.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId, SourceId};
use thiserror::Error;

use crate::{DoorLeaves, SourceSnapshot};

/// Failure to measure a door's leaf glazing.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LeafGlazingError {
    /// The service does not cover the door's source.
    #[error("leaf-glazing service does not cover source `{0}`")]
    UncoveredSource(SourceId),
    /// The service holds no geometry for the door.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The door has no body to measure its glazing from.
    #[error("`{0}` has no body")]
    NoBody(ObjectId),
    /// The door's body, or its items, could not be measured.
    #[error("leaf glazing unavailable: {0}")]
    Unavailable(String),
    /// The measurement is malformed: an empty or reversed interval, a
    /// leaf count that does not match the request, or evidence without a
    /// locator.
    #[error("leaf glazing is invalid")]
    InvalidMeasurement,
    /// The service answered for another door.
    #[error("leaf-glazing service answered for another door")]
    ResponseRequestMismatch,
}

/// The glazing of one leaf.
#[derive(Clone, Debug, PartialEq)]
pub enum LeafGlazing {
    /// Measured: the glazed area and the leaf's area, each an interval in
    /// square metres, and how many glazed items the leaf holds.
    Measured {
        /// The glazed area's bounds, lower first.
        glazed: (f64, f64),
        /// The leaf's area's bounds, lower first; above zero.
        leaf: (f64, f64),
        /// How many glazed items were assigned to the leaf.
        items: usize,
    },
    /// An item that could be glazed reaches into the leaf, but its
    /// material or its leaf cannot be decided: why, in plain words.
    Undecided(String),
}

impl LeafGlazing {
    /// The glazed share of the leaf, `glazed / leaf`, as bounds; `None`
    /// for an undecided leaf.
    #[must_use]
    pub fn ratio(&self) -> Option<(f64, f64)> {
        match self {
            Self::Measured { glazed, leaf, .. } => Some((glazed.0 / leaf.1, glazed.1 / leaf.0)),
            Self::Undecided(_) => None,
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Self::Measured { glazed, leaf, .. } => {
                let interval = |(lower, upper): (f64, f64)| {
                    lower.is_finite() && upper.is_finite() && 0.0 <= lower && lower <= upper
                };
                interval(*glazed) && interval(*leaf) && leaf.0 > 0.0
            }
            Self::Undecided(why) => !why.trim().is_empty(),
        }
    }
}

/// The glazing of every leaf of one door, in the leaves' order, with
/// evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorGlazing {
    door: ObjectId,
    leaves: Vec<LeafGlazing>,
    evidence: Evidence,
}

impl DoorGlazing {
    /// The glazing of `door`'s leaves. Every measured interval must be
    /// ordered, finite and non-negative, every leaf area above zero, an
    /// undecided leaf must say why, and the evidence must name where the
    /// measurement comes from; it is exact only when the body is the
    /// door's exact shape.
    pub fn try_new(
        door: ObjectId,
        leaves: Vec<LeafGlazing>,
        evidence: Evidence,
    ) -> Result<Self, LeafGlazingError> {
        if evidence.locator.trim().is_empty()
            || evidence.source != door.source
            || !leaves.iter().all(LeafGlazing::is_valid)
        {
            return Err(LeafGlazingError::InvalidMeasurement);
        }
        Ok(Self {
            door,
            leaves,
            evidence,
        })
    }

    /// The door.
    #[must_use]
    pub fn door(&self) -> &ObjectId {
        &self.door
    }

    /// Each leaf's glazing, in the order of the leaves measured against.
    #[must_use]
    pub fn leaves(&self) -> &[LeafGlazing] {
        &self.leaves
    }

    /// Whether the measurement is exact: the body is the door's exact
    /// shape and every projection exact.
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

/// Measures the glazing of doors' leaves from their bodies.
pub trait LeafGlazingService: Send + Sync + 'static {
    /// Exact source snapshots this service answers for.
    // gate: reads
    fn source_snapshots(&self) -> &[SourceSnapshot];
    /// The glazed area and area of each of `leaves`, measured from the
    /// body of the door they belong to.
    // gate: measures area
    fn measure_leaf_glazing(&self, leaves: &DoorLeaves) -> Result<DoorGlazing, LeafGlazingError>;
}

/// Registry handle for a [`LeafGlazingService`].
#[derive(Clone)]
pub struct LeafGlazingServiceHandle(Arc<dyn LeafGlazingService>);

impl LeafGlazingServiceHandle {
    /// Wraps a trusted leaf-glazing service.
    #[must_use]
    pub fn new(service: Arc<dyn LeafGlazingService>) -> Self {
        Self(service)
    }

    /// The glazing of each of `leaves`. A door of an uncovered source is
    /// refused, and so is an answer for another door or for another
    /// number of leaves.
    pub fn measure_leaf_glazing(
        &self,
        leaves: &DoorLeaves,
    ) -> Result<DoorGlazing, LeafGlazingError> {
        let door = leaves.door();
        if !self
            .0
            .source_snapshots()
            .iter()
            .any(|snapshot| *snapshot.source() == door.source)
        {
            return Err(LeafGlazingError::UncoveredSource(door.source.clone()));
        }
        let glazing = crate::timing::host("leaf-glazing", || self.0.measure_leaf_glazing(leaves))?;
        if glazing.door() != door {
            return Err(LeafGlazingError::ResponseRequestMismatch);
        }
        if glazing.leaves().len() != leaves.leaves().len() {
            return Err(LeafGlazingError::InvalidMeasurement);
        }
        Ok(glazing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn evidence() -> Evidence {
        Evidence::exact(source(), "leaf-glazing:d")
    }

    #[test]
    fn a_measurement_is_checked() {
        let measured = LeafGlazing::Measured {
            glazed: (0.5, 0.5),
            leaf: (2.0, 2.0),
            items: 1,
        };
        assert_eq!(measured.ratio(), Some((0.25, 0.25)));
        assert!(DoorGlazing::try_new(id("d"), vec![measured], evidence()).is_ok());
        let reversed = LeafGlazing::Measured {
            glazed: (0.6, 0.5),
            leaf: (2.0, 2.0),
            items: 1,
        };
        assert_eq!(
            DoorGlazing::try_new(id("d"), vec![reversed], evidence()),
            Err(LeafGlazingError::InvalidMeasurement)
        );
        let empty = LeafGlazing::Measured {
            glazed: (0.0, 0.0),
            leaf: (0.0, 1.0),
            items: 0,
        };
        assert_eq!(
            DoorGlazing::try_new(id("d"), vec![empty], evidence()),
            Err(LeafGlazingError::InvalidMeasurement)
        );
        assert_eq!(
            DoorGlazing::try_new(
                id("d"),
                vec![LeafGlazing::Undecided(" ".into())],
                evidence()
            ),
            Err(LeafGlazingError::InvalidMeasurement)
        );
        let unlocated = Evidence::exact(source(), " ");
        assert_eq!(
            DoorGlazing::try_new(id("d"), Vec::new(), unlocated),
            Err(LeafGlazingError::InvalidMeasurement)
        );
    }
}
