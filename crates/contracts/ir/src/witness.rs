//! Where a measurement between two bodies happens (#388).
//!
//! Evidence of a proximity measurement names the two bodies it measured and
//! a locator, but a reviewer, a viewer or an issue tracker also needs the
//! spot: the closest pair of points of two bodies apart, the deepest point
//! of one body found inside the other and where it leaves the other, and
//! for an overlap the box the shared region spans.
//!
//! A witness is geometry evidence and is held to the same rules as a
//! measured value:
//!
//! - **Points carry their deviation.** Each point lies on (or, for a
//!   penetration's deep point, in) the measured geometry of its body, and
//!   the true geometry lies within [`WitnessPoint::deviation_metres`] of it:
//!   zero for exact geometry, the chord deviation for a tessellation, widened
//!   by any rounding of the conversion into model coordinates. A witness is
//!   never an estimate presented as exact.
//! - **Extents are brackets.** [`WitnessExtent::outer`] is a box sure to
//!   contain the shared region; [`WitnessExtent::inner`] a box spanned by
//!   points witnessed in it, so the region's own box contains it. An
//!   unwitnessed inner box is absent rather than guessed.
//! - **No geometry, no witness.** An adapter that cannot find the spot
//!   (a degenerate triangle, a refused kernel query) leaves the witness out;
//!   it never places one at the objects' centres.
//!
//! Coordinates are the source's model coordinates in metres, as every
//! other position the engine reports: a local frame the geometry was
//! measured in has already been undone. The closest pair of two bodies is
//! not unique (two parallel faces have infinitely many), so a witness is one
//! pair realizing the measured value, not the only one.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a witness was refused.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum WitnessError {
    /// A coordinate or deviation is not finite, or a deviation is negative.
    #[error("a witness coordinate or deviation is not finite and non-negative")]
    NotFinite,
    /// A box's minimum exceeds its maximum on an axis.
    #[error("a witness box's minimum exceeds its maximum")]
    Inverted,
    /// An extent's inner box reaches outside its outer box.
    #[error("a witness extent's inner box reaches outside its outer box")]
    InnerOutside,
}

/// What a [`Witness`]'s two points stand for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessKind {
    /// The closest pair: `point_a` on the subject's surface, `point_b` on
    /// the counterpart's, as far apart as the measured separation.
    Separation,
    /// `point_a` is the deepest point of the subject found inside the
    /// counterpart, `point_b` the nearest point of the counterpart's
    /// surface, where the subject leaves it. Their distance is the
    /// witnessed penetration depth, a lower bound on the true depth.
    PenetrationIntoCounterpart,
    /// `point_b` is the deepest point of the counterpart found inside the
    /// subject, `point_a` the nearest point of the subject's surface.
    PenetrationIntoSubject,
}

impl WitnessKind {
    /// The kind's spelling on the wire and in messages.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Separation => "separation",
            Self::PenetrationIntoCounterpart => "penetration_into_counterpart",
            Self::PenetrationIntoSubject => "penetration_into_subject",
        }
    }
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite())
}

/// A point in model coordinates, in metres, with how far the true geometry
/// it stands for may lie from it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PointWire")]
pub struct WitnessPoint {
    metres: [f64; 3],
    deviation_metres: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PointWire {
    metres: [f64; 3],
    deviation_metres: f64,
}

impl TryFrom<PointWire> for WitnessPoint {
    type Error = WitnessError;
    fn try_from(wire: PointWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.metres, wire.deviation_metres)
    }
}

impl WitnessPoint {
    /// Refuses a non-finite coordinate and a non-finite or negative
    /// deviation.
    pub fn try_new(metres: [f64; 3], deviation_metres: f64) -> Result<Self, WitnessError> {
        if !finite(&metres) || !deviation_metres.is_finite() || deviation_metres < 0.0 {
            return Err(WitnessError::NotFinite);
        }
        Ok(Self {
            metres,
            deviation_metres,
        })
    }
    /// The point, `[x, y, z]` in metres.
    #[must_use]
    pub fn metres(&self) -> [f64; 3] {
        self.metres
    }
    /// How far the true geometry the point stands for may lie from it;
    /// zero when the point is exact.
    #[must_use]
    pub fn deviation_metres(&self) -> f64 {
        self.deviation_metres
    }
    /// Whether the point is exact.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.deviation_metres == 0.0
    }
    /// The distance to `other`.
    #[must_use]
    pub fn distance(&self, other: &Self) -> f64 {
        (0..3)
            .map(|axis| (self.metres[axis] - other.metres[axis]).powi(2))
            .sum::<f64>()
            .sqrt()
    }
}

// Coordinates and deviations are finite by construction, so equality is
// reflexive.
impl Eq for WitnessPoint {}

/// An axis-aligned box in model coordinates, in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "BoxWire")]
pub struct WitnessBox {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoxWire {
    min: [f64; 3],
    max: [f64; 3],
}

impl TryFrom<BoxWire> for WitnessBox {
    type Error = WitnessError;
    fn try_from(wire: BoxWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.min, wire.max)
    }
}

impl WitnessBox {
    /// Refuses a non-finite corner and one whose minimum exceeds its
    /// maximum on an axis.
    pub fn try_new(min: [f64; 3], max: [f64; 3]) -> Result<Self, WitnessError> {
        if !finite(&min) || !finite(&max) {
            return Err(WitnessError::NotFinite);
        }
        if (0..3).any(|axis| min[axis] > max[axis]) {
            return Err(WitnessError::Inverted);
        }
        Ok(Self { min, max })
    }
    /// The lowest corner.
    #[must_use]
    pub fn min(&self) -> [f64; 3] {
        self.min
    }
    /// The highest corner.
    #[must_use]
    pub fn max(&self) -> [f64; 3] {
        self.max
    }
    /// The box's length along each axis.
    #[must_use]
    pub fn size(&self) -> [f64; 3] {
        [0, 1, 2].map(|axis| self.max[axis] - self.min[axis])
    }
    /// Whether `other` lies within this box.
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        (0..3).all(|axis| self.min[axis] <= other.min[axis] && other.max[axis] <= self.max[axis])
    }
}

impl Eq for WitnessBox {}

/// The axis-aligned extent of the region two bodies share, as a bracket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ExtentWire")]
pub struct WitnessExtent {
    #[serde(skip_serializing_if = "Option::is_none")]
    inner: Option<WitnessBox>,
    outer: WitnessBox,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtentWire {
    #[serde(default)]
    inner: Option<WitnessBox>,
    outer: WitnessBox,
}

impl TryFrom<ExtentWire> for WitnessExtent {
    type Error = WitnessError;
    fn try_from(wire: ExtentWire) -> Result<Self, Self::Error> {
        Self::try_new(wire.inner, wire.outer)
    }
}

impl WitnessExtent {
    /// Refuses an inner box reaching outside the outer one.
    pub fn try_new(inner: Option<WitnessBox>, outer: WitnessBox) -> Result<Self, WitnessError> {
        if inner.is_some_and(|inner| !outer.contains(&inner)) {
            return Err(WitnessError::InnerOutside);
        }
        Ok(Self { inner, outer })
    }
    /// A box spanned by points witnessed in the shared region, so the
    /// region's own box contains it; `None` when no such point was found.
    #[must_use]
    pub fn inner(&self) -> Option<WitnessBox> {
        self.inner
    }
    /// A box sure to contain the shared region.
    #[must_use]
    pub fn outer(&self) -> WitnessBox {
        self.outer
    }
    /// Whether the bracket is closed: the inner box is the outer one, so
    /// the region's box is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.inner == Some(self.outer)
    }
}

/// Where a measurement between two bodies happens: its two witness points
/// and, for an overlap, the extent of the shared region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    kind: WitnessKind,
    point_a: WitnessPoint,
    point_b: WitnessPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extent: Option<WitnessExtent>,
}

impl Witness {
    /// A witness pair: `point_a` belongs to the measurement's subject,
    /// `point_b` to its counterpart, as `kind` says.
    #[must_use]
    pub fn new(kind: WitnessKind, point_a: WitnessPoint, point_b: WitnessPoint) -> Self {
        Self {
            kind,
            point_a,
            point_b,
            extent: None,
        }
    }
    /// The witness with the extent of the shared region.
    #[must_use]
    pub fn with_extent(mut self, extent: WitnessExtent) -> Self {
        self.extent = Some(extent);
        self
    }
    /// What the two points stand for.
    #[must_use]
    pub fn kind(&self) -> WitnessKind {
        self.kind
    }
    /// The point of the subject's side.
    #[must_use]
    pub fn point_a(&self) -> WitnessPoint {
        self.point_a
    }
    /// The point of the counterpart's side.
    #[must_use]
    pub fn point_b(&self) -> WitnessPoint {
        self.point_b
    }
    /// The extent of the shared region; `None` for bodies apart, or when
    /// it was not measured.
    #[must_use]
    pub fn extent(&self) -> Option<WitnessExtent> {
        self.extent
    }
    /// Whether every part of the witness is exact.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.point_a.is_exact()
            && self.point_b.is_exact()
            && self.extent.is_none_or(|extent| extent.is_exact())
    }
    /// The smallest box holding both points and the extent's outer box:
    /// what a viewer frames to show the spot.
    #[must_use]
    pub fn frame(&self) -> WitnessBox {
        let (a, b) = (self.point_a.metres, self.point_b.metres);
        let mut min = [0, 1, 2].map(|axis| a[axis].min(b[axis]));
        let mut max = [0, 1, 2].map(|axis| a[axis].max(b[axis]));
        if let Some(extent) = self.extent {
            for axis in 0..3 {
                min[axis] = min[axis].min(extent.outer.min[axis]);
                max[axis] = max[axis].max(extent.outer.max[axis]);
            }
        }
        WitnessBox { min, max }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Corners are compared exactly.
mod tests {
    use super::*;

    fn point(metres: [f64; 3]) -> WitnessPoint {
        WitnessPoint::try_new(metres, 0.0).unwrap()
    }

    #[test]
    fn a_witness_round_trips_and_omits_an_absent_extent() {
        let witness = Witness::new(
            WitnessKind::Separation,
            point([1.0, 0.0, 0.0]),
            WitnessPoint::try_new([1.5, 0.0, 0.0], 0.001).unwrap(),
        );
        let json = serde_json::to_value(witness).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "separation",
                "point_a": {"metres": [1.0, 0.0, 0.0], "deviation_metres": 0.0},
                "point_b": {"metres": [1.5, 0.0, 0.0], "deviation_metres": 0.001},
            })
        );
        assert_eq!(serde_json::from_value::<Witness>(json).unwrap(), witness);
        assert!(!witness.is_exact());
        assert!((witness.point_a().distance(&witness.point_b()) - 0.5).abs() < 1e-15);
    }

    #[test]
    fn incoherent_witnesses_are_refused_on_construction_and_on_reading() {
        assert_eq!(
            WitnessPoint::try_new([f64::NAN, 0.0, 0.0], 0.0),
            Err(WitnessError::NotFinite)
        );
        assert_eq!(
            WitnessPoint::try_new([0.0; 3], -1.0),
            Err(WitnessError::NotFinite)
        );
        assert_eq!(
            WitnessBox::try_new([1.0, 0.0, 0.0], [0.0; 3]),
            Err(WitnessError::Inverted)
        );
        let outer = WitnessBox::try_new([0.0; 3], [1.0; 3]).unwrap();
        let wider = WitnessBox::try_new([0.0; 3], [2.0; 3]).unwrap();
        assert_eq!(
            WitnessExtent::try_new(Some(wider), outer),
            Err(WitnessError::InnerOutside)
        );
        let inverted = serde_json::json!({"min": [1.0, 0.0, 0.0], "max": [0.0, 0.0, 0.0]});
        assert!(serde_json::from_value::<WitnessBox>(inverted).is_err());
        let unknown =
            serde_json::json!({"metres": [0.0, 0.0, 0.0], "deviation_metres": 0.0, "x": 1});
        assert!(serde_json::from_value::<WitnessPoint>(unknown).is_err());
    }

    #[test]
    fn the_frame_holds_the_points_and_the_outer_extent() {
        let outer = WitnessBox::try_new([0.0; 3], [1.0, 1.0, 0.5]).unwrap();
        let witness = Witness::new(
            WitnessKind::PenetrationIntoCounterpart,
            point([0.5, 0.5, 0.5]),
            point([0.5, 0.5, 2.0]),
        )
        .with_extent(WitnessExtent::try_new(Some(outer), outer).unwrap());
        assert!(witness.is_exact());
        assert_eq!(witness.frame().min(), [0.0; 3]);
        assert_eq!(witness.frame().max(), [1.0, 1.0, 2.0]);
    }
}
