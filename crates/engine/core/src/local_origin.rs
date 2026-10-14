//! Local frames geometry is measured in (#372).
//!
//! A source placed in map coordinates puts its bodies millions of metres
//! from the origin, where a coordinate rounds to about 1e-9 m: a mesh built
//! there is not the solid, however exact its construction. A host may
//! therefore measure such a source in a local frame, its model coordinates
//! less a horizontal translation, the source's [`LocalOrigin`]: every mesh
//! the geometry services hold and every frame and door leaf the object-frame
//! service answers ([`crate::EvidenceSession::with_local_origins`]) are then
//! stated in it. Heights are never moved, so levels, elevations and vertical
//! extents read alike in both frames.
//!
//! Distances, areas and volumes do not change with a translation. What
//! does, a position, is brought into the local frame where context stated
//! in model or map coordinates is compared with it (context zones, the
//! terrain), and back into model coordinates where it is reported. Both
//! conversions round, so a comparison widens by [`LocalOrigin::rounding`].
//!
//! A source without an origin is measured in its model coordinates: the
//! origin is zero and nothing is converted.

use std::collections::BTreeMap;

use axioval_ir::SourceId;

/// The horizontal translation from a source's local frame to its model
/// coordinates, in metres: a model point is the local point plus it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LocalOrigin {
    metres: [f64; 2],
}

impl LocalOrigin {
    /// The origin at `metres` (east, north) in model coordinates; `None`
    /// unless both are finite.
    #[must_use]
    pub fn new(metres: [f64; 2]) -> Option<Self> {
        metres
            .iter()
            .all(|value| value.is_finite())
            .then_some(Self { metres })
    }

    /// Its position in model coordinates, in metres.
    #[must_use]
    pub fn metres(&self) -> [f64; 2] {
        self.metres
    }

    /// Whether it is the model origin, so nothing is converted.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.metres == [0.0, 0.0]
    }

    /// A local plan point in model coordinates.
    #[must_use]
    pub fn to_model(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        if self.is_zero() {
            return [x, y];
        }
        [x + self.metres[0], y + self.metres[1]]
    }

    /// A local plan point in model coordinates, and how far the model point
    /// it was read from may lie from it: zero on an axis where the origin is
    /// zero or the point lies well within a factor of two of the origin, so
    /// that both differences are exact (Sterbenz), the rounding of a
    /// conversion otherwise.
    #[must_use]
    pub fn to_model_bounded(&self, point: [f64; 2]) -> ([f64; 2], f64) {
        let model = self.to_model(point);
        let near = (0..2).all(|axis| {
            let origin = self.metres[axis];
            origin == 0.0 || (0.75..=1.5).contains(&(model[axis] / origin))
        });
        let rounding = if near {
            0.0
        } else {
            self.rounding(model[0].abs().max(model[1].abs()))
        };
        (model, rounding)
    }

    /// The projection of the origin onto `direction`, a unit vector (its
    /// height ignored): what a position measured along that direction in
    /// the local frame lacks of its model position. It and the sum round
    /// by no more than [`Self::rounding`] of the sum's magnitude.
    #[must_use]
    pub fn along(&self, direction: [f64; 3]) -> f64 {
        self.metres[0] * direction[0] + self.metres[1] * direction[1]
    }

    /// A local point in model coordinates, its height unchanged.
    #[must_use]
    pub fn point_to_model(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let [x, y] = self.to_model([x, y]);
        [x, y, z]
    }

    /// A model plan point in the local frame.
    #[must_use]
    pub fn to_local(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        if self.is_zero() {
            return [x, y];
        }
        [x - self.metres[0], y - self.metres[1]]
    }

    /// A model point in the local frame, its height unchanged.
    #[must_use]
    pub fn point_to_local(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let [x, y] = self.to_local([x, y]);
        [x, y, z]
    }

    /// The model plan point `point` in the local frame, and whether no
    /// coordinate rounded on the way.
    #[must_use]
    pub fn to_local_exactly(&self, point: [f64; 2]) -> ([f64; 2], bool) {
        let local = self.to_local(point);
        let exact =
            self.is_zero() || (0..2).all(|axis| exact_difference(point[axis], self.metres[axis]));
        (local, exact)
    }

    /// How far, in metres, converting a plan point of `magnitude` metres
    /// (its largest coordinate, in either frame) from one frame to the
    /// other may move it: the rounding of the sum, generously, plus that
    /// of the origin itself, which a host derived from the coordinates its
    /// source states. Zero at the model origin.
    #[must_use]
    pub fn rounding(&self, magnitude: f64) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let largest = self.metres[0].abs().max(self.metres[1].abs());
        (16.0 * f64::EPSILON * (magnitude.abs() + largest)).next_up()
    }
}

/// Whether `a - b` is exact in floating point (Knuth's two-sum error is
/// zero).
fn exact_difference(a: f64, b: f64) -> bool {
    let b = -b;
    let sum = a + b;
    let b_virtual = sum - a;
    let a_virtual = sum - b_virtual;
    let error = (a - a_virtual) + (b - b_virtual);
    sum.is_finite() && error == 0.0
}

/// The local origin of every source a host measures in a local frame.
///
/// A source it does not name is measured in its model coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalOrigins {
    origins: BTreeMap<SourceId, LocalOrigin>,
}

impl LocalOrigins {
    /// The origins of `origins`, the model origin left out.
    #[must_use]
    pub fn new(origins: impl IntoIterator<Item = (SourceId, LocalOrigin)>) -> Self {
        Self {
            origins: origins
                .into_iter()
                .filter(|(_, origin)| !origin.is_zero())
                .collect(),
        }
    }

    /// The origin `source` is measured about, zero when it has none.
    #[must_use]
    pub fn of(&self, source: &SourceId) -> LocalOrigin {
        self.origins.get(source).copied().unwrap_or_default()
    }

    /// Whether no source is measured in a local frame.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// Every source with its origin, in source order.
    pub fn iter(&self) -> impl Iterator<Item = (&SourceId, &LocalOrigin)> {
        self.origins.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_goes_to_the_local_frame_and_back() {
        let origin = LocalOrigin::new([643_692.128_712_773, 5_649_156.756_632_48]).unwrap();
        let local = origin.to_local([643_700.0, 5_649_150.0]);
        assert!((local[0] - 7.871_287_227).abs() < 1e-6);
        let back = origin.to_model(local);
        assert!((back[0] - 643_700.0).abs() <= origin.rounding(1e7));
        assert!((back[1] - 5_649_150.0).abs() <= origin.rounding(1e7));
        // Near the origin the difference is exact (Sterbenz).
        let (_, exact) = origin.to_local_exactly([643_692.0, 5_649_156.0]);
        assert!(exact);
        // Far from it, it rounds.
        let (_, exact) = origin.to_local_exactly([0.1, 0.1]);
        assert!(!exact);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn the_model_origin_converts_nothing() {
        let origin = LocalOrigin::default();
        assert!(origin.is_zero());
        assert_eq!(origin.to_local([0.1, 0.2]), [0.1, 0.2]);
        assert_eq!(origin.rounding(1e7), 0.0);
        assert_eq!(origin.to_local_exactly([0.1, 0.2]), ([0.1, 0.2], true));
        let origins = LocalOrigins::new([(SourceId::new("ifc", "a.ifc").unwrap(), origin)]);
        assert!(origins.is_empty());
        assert!(LocalOrigin::new([f64::NAN, 0.0]).is_none());
    }
}
