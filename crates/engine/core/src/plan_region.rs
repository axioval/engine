//! Convex plan regions: stated areas of the floor plan that are not an
//! object's body, such as the sector a door leaf sweeps.
//!
//! A region is a convex polygon in canonical metres, anticlockwise seen
//! from above. Its geometry is exact as stated; a region approximating a
//! curved area (a sector between an inscribed and a circumscribed polygon)
//! is the caller's bracket, not the region's.

use crate::{PlanRing, ProximityError};

/// Tolerance on convexity, relative to the edge lengths multiplied.
const CONVEXITY: f64 = 1.0e-12;

/// A convex plan polygon, anticlockwise, in canonical metres.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvexPlanRegion {
    ring: PlanRing,
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn length(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Distance from `point` to the segment `a`–`b`.
fn segment_distance(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let squared = dx * dx + dy * dy;
    let t = if squared == 0.0 {
        0.0
    } else {
        (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / squared).clamp(0.0, 1.0)
    };
    (point[0] - (a[0] + t * dx)).hypot(point[1] - (a[1] + t * dy))
}

impl ConvexPlanRegion {
    /// A region from its vertices, in order and without repeating the
    /// first. They must be finite, at least three, turn anticlockwise at
    /// every vertex (collinear vertices are accepted) and enclose a positive
    /// area.
    pub fn try_new(ring: PlanRing) -> Result<Self, ProximityError> {
        let count = ring.len();
        if count < 3 || ring.iter().flatten().any(|c| !c.is_finite()) {
            return Err(ProximityError::InvalidMeasurement);
        }
        let mut area = 0.0;
        for index in 0..count {
            let (a, b, c) = (
                ring[index],
                ring[(index + 1) % count],
                ring[(index + 2) % count],
            );
            let scale = length(a, b) * length(b, c);
            if cross(a, b, c) < -CONVEXITY * scale.max(f64::MIN_POSITIVE) {
                return Err(ProximityError::InvalidMeasurement);
            }
            area += a[0] * b[1] - b[0] * a[1];
        }
        if area <= 0.0 {
            return Err(ProximityError::InvalidMeasurement);
        }
        Ok(Self { ring })
    }

    /// The vertices, anticlockwise.
    #[must_use]
    pub fn ring(&self) -> &[[f64; 2]] {
        &self.ring
    }

    /// The least and greatest corner of the region's plan box.
    #[must_use]
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        self.ring.iter().fold(
            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
            |(low, high), [x, y]| {
                (
                    [low[0].min(*x), low[1].min(*y)],
                    [high[0].max(*x), high[1].max(*y)],
                )
            },
        )
    }

    /// The region's area in square metres.
    #[must_use]
    pub fn area_square_metres(&self) -> f64 {
        let count = self.ring.len();
        (0..count)
            .map(|index| {
                let (a, b) = (self.ring[index], self.ring[(index + 1) % count]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            / 2.0
    }

    fn edges(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        let count = self.ring.len();
        (0..count).map(move |index| (self.ring[index], self.ring[(index + 1) % count]))
    }

    /// Signed separation from `other`: the plan distance between the two
    /// regions when they are apart, zero when they touch, and minus the
    /// least depth one reaches into the other along an edge normal when
    /// they overlap with positive area.
    ///
    /// Both are convex, so they are apart exactly when an edge normal of
    /// one separates them, and their distance is then the least distance
    /// from a vertex of one to an edge of the other.
    #[must_use]
    pub fn separation(&self, other: &Self) -> f64 {
        let mut gap = f64::NEG_INFINITY;
        for (region, against) in [(self, other), (other, self)] {
            for (a, b) in region.edges() {
                let edge = length(a, b);
                if edge == 0.0 {
                    continue;
                }
                // Outward normal of an anticlockwise edge.
                let normal = [(b[1] - a[1]) / edge, (a[0] - b[0]) / edge];
                let reach = |point: &[f64; 2]| {
                    (point[0] - a[0]) * normal[0] + (point[1] - a[1]) * normal[1]
                };
                let nearest = against.ring.iter().map(reach).fold(f64::INFINITY, f64::min);
                gap = gap.max(nearest);
            }
        }
        if gap < 0.0 {
            return gap;
        }
        let mut distance = f64::INFINITY;
        for (region, against) in [(self, other), (other, self)] {
            for point in &region.ring {
                for (a, b) in against.edges() {
                    distance = distance.min(segment_distance(*point, a, b));
                }
            }
        }
        distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> ConvexPlanRegion {
        ConvexPlanRegion::try_new(vec![
            [x, y],
            [x + side, y],
            [x + side, y + side],
            [x, y + side],
        ])
        .unwrap()
    }

    #[test]
    fn a_region_is_convex_and_anticlockwise() {
        assert!(ConvexPlanRegion::try_new(vec![[0.0, 0.0], [1.0, 0.0]]).is_err());
        // Clockwise.
        assert!(
            ConvexPlanRegion::try_new(vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]])
                .is_err()
        );
        // A notch.
        assert!(
            ConvexPlanRegion::try_new(vec![
                [0.0, 0.0],
                [2.0, 0.0],
                [1.0, 0.5],
                [2.0, 2.0],
                [0.0, 2.0]
            ])
            .is_err()
        );
        // Collinear vertices are accepted.
        let collinear =
            ConvexPlanRegion::try_new(vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [1.0, 1.0]])
                .unwrap();
        assert!((collinear.area_square_metres() - 1.0).abs() < 1e-12);
        assert_eq!(square(1.0, 2.0, 1.0).bounds(), ([1.0, 2.0], [2.0, 3.0]));
    }

    #[test]
    fn separation_is_the_distance_apart_or_the_depth_of_overlap() {
        let a = square(0.0, 0.0, 1.0);
        assert!((a.separation(&square(3.0, 0.0, 1.0)) - 2.0).abs() < 1e-12);
        // Corner to corner.
        let diagonal = a.separation(&square(2.0, 2.0, 1.0));
        assert!((diagonal - 2.0_f64.sqrt()).abs() < 1e-12);
        assert!(a.separation(&square(1.0, 0.0, 1.0)).abs() < 1e-12);
        assert!((a.separation(&square(0.75, 0.5, 1.0)) + 0.25).abs() < 1e-12);
    }
}
