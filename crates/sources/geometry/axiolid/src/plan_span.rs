//! Plan spans: the longest plan diagonal of a footprint, and the distance
//! between two footprints' centres or farthest points.
//!
//! ADR 0004: this module measures lengths; whether a room's exits lie far
//! enough apart is a rule's decision.
//!
//! A footprint is the union of its projected triangles, so every point of it
//! lies in the convex hull of the triangles' vertices, and so does every
//! vertex of the hull lie in the footprint. The distance between two points
//! is convex in each, so its maximum over a footprint is reached at hull
//! vertices: the longest diagonal is the largest distance between two hull
//! vertices, and the farthest span between two footprints the largest
//! distance between a hull vertex of each. No polygon boundary is walked.
//!
//! A centre is the footprint's centroid, from the plan overlay the plan-area
//! service measures with, so overlapping parts count once.
//!
//! A planar mesh is the object's shape and measures exactly. A tessellated
//! mesh lies within its chord deviation `d` of the true surface, and so does
//! its footprint of the true one. A longest diagonal is then within `2d`, a
//! farthest span within the two deviations added. A centroid moves only
//! through the band of area `b = 2·P·d + π·d²` where the footprints can
//! differ, whose points lie within `R + d` of the measured centroid (`R` the
//! farthest hull vertex from it): by at most `b·(R + d) / (A − b)` for a
//! measured area `A`. A footprint no larger than its band has no bounded
//! centre and is refused.
//!
//! The rectangle of least area enclosing a footprint comes from the
//! overlay's rotating calipers over every plan vertex: the choice of
//! orientation is exact, only the output is rounded, by at most the
//! kernel's stated `error` for the centre, every corner and every half
//! extent. Two corners `2L` apart along the long side (`L` the longer half
//! extent) then fix each axis within `asin(e / (L − e))`. A rectangle along
//! the coordinate axes (its first axis exactly `(1, 0)`, which the exact
//! choice returns only for an edge along x) turns nothing, so it is taken
//! from the extreme coordinates instead: exact whenever their differences
//! and sums are. A tessellated
//! footprint lies within `d` of the true one, so the true footprint's
//! extents along the measured axes lie within `d` of the measured ones, but
//! which orientation encloses the true footprint with least area is not
//! known: its orientation is unproven. Several orientations of least area
//! (up to quarter turns) are tied.
//!
//! A centre lies inside its footprint when it lies inside the measured one
//! farther from every boundary edge than the centre's own uncertainty plus
//! the chord deviation, outside likewise, and undecided otherwise: a centre
//! on the boundary of an exact footprint is undecided too.

use axiolid_core::Point2;
use axiolid_overlay::{Polygon, RectangleError, Ring, minimum_area_rectangle};
use axioval_engine::{
    CentrePlacement, PlanAreaError, PlanCentre, PlanLength, PlanRectangle, PlanSpan, PlanSpanError,
    PlanSpanService, RectangleOrientation,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle};
use crate::plan_area::{AxiolidPlanAreaService, Footprint, band, tolerance};
use crate::planar::{footprint_polygons, hull_of, polygon_moments, ring_segments};

/// A point in plan.
type Point = (f64, f64);

/// Measures plan spans of registered meshes using Axiolid.
#[derive(Debug)]
pub struct AxiolidPlanSpanService {
    footprints: AxiolidPlanAreaService,
    source: SourceId,
}

impl AxiolidPlanSpanService {
    /// Creates a service over the supplied geometry.
    ///
    /// Footprints are gathered as the plan-area service gathers them, so a
    /// declared group spans the union of its members.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry, source: SourceId) -> Self {
        Self {
            footprints: AxiolidPlanAreaService::new(geometry, source.clone()),
            source,
        }
    }

    fn footprint(&self, object: &ObjectId) -> Result<Footprint, PlanSpanError> {
        self.footprints
            .measure(object)
            .map_err(|error| match error {
                PlanAreaError::UnknownObject(object) => PlanSpanError::UnknownObject(object),
                PlanAreaError::Unavailable(reason) => PlanSpanError::Unavailable(reason),
                other => PlanSpanError::Unavailable(other.to_string()),
            })
    }

    /// The convex hull of `object`'s footprint, refused when it has no point.
    fn hull(object: &ObjectId, footprint: &Footprint) -> Result<Vec<Point>, PlanSpanError> {
        let hull = convex_hull(&footprint.soup);
        if hull.is_empty() {
            return Err(PlanSpanError::Unavailable(format!(
                "{object} has no footprint (no body)"
            )));
        }
        Ok(hull)
    }

    /// The centroid of `object`'s footprint and how far the true one can lie
    /// from it.
    fn centre(
        object: &ObjectId,
        footprint: &Footprint,
        hull: &[Point],
    ) -> Result<(Point, f64), PlanSpanError> {
        Self::located(object, footprint, hull).map(|(centre, slack, _)| (centre, slack))
    }

    /// As [`Self::centre`], with the measured footprint's polygons.
    fn located(
        object: &ObjectId,
        footprint: &Footprint,
        hull: &[Point],
    ) -> Result<(Point, f64, Vec<Polygon>), PlanSpanError> {
        let tolerance = tolerance()
            .map_err(|_| PlanSpanError::Unavailable("invalid overlay tolerance".into()))?;
        let polygons = footprint_polygons(&footprint.soup, tolerance).ok_or_else(|| {
            PlanSpanError::Unavailable(format!("the footprint of {object} cannot be computed"))
        })?;
        let (area, x, y) = polygons
            .iter()
            .map(polygon_moments)
            .fold((0.0, 0.0, 0.0), |(a, x, y), (pa, px, py)| {
                (a + pa, x + px, y + py)
            });
        if area <= 0.0 {
            return Err(PlanSpanError::Unavailable(format!(
                "{object}'s footprint has no area, so it has no centre"
            )));
        }
        let centre = (x / area, y / area);
        if footprint.deviation == 0.0 {
            return Ok((centre, 0.0, polygons));
        }
        let slack = band(footprint.perimeter, footprint.deviation);
        if area <= slack {
            return Err(PlanSpanError::Unavailable(format!(
                "{object}'s footprint is too small for its chord deviation to bound its centre"
            )));
        }
        let reach = hull
            .iter()
            .map(|point| distance(*point, centre))
            .fold(0.0, f64::max);
        Ok((
            centre,
            slack * (reach + footprint.deviation) / (area - slack),
            polygons,
        ))
    }

    fn length(
        &self,
        measured: f64,
        slack: f64,
        locator: String,
    ) -> Result<PlanLength, PlanSpanError> {
        if slack == 0.0 {
            return PlanLength::try_new(
                measured,
                measured,
                Evidence::exact(self.source.clone(), locator),
            );
        }
        PlanLength::try_new(
            (measured - slack).max(0.0),
            measured + slack,
            Evidence {
                source: self.source.clone(),
                locator,
                exact: false,
            },
        )
    }
}

/// How far, in radians, an axis fixed by two corners `2 * long` apart can
/// turn when each corner may move by `error`: a quarter turn when the
/// corners may meet.
fn axis_error(long: f64, error: f64) -> f64 {
    if error == 0.0 {
        return 0.0;
    }
    if long <= error {
        return std::f64::consts::FRAC_PI_2;
    }
    (error / (long - error)).min(1.0).asin()
}

/// A `(lower, upper)` interval of lengths.
trait Widened {
    /// The interval grown by `margin` each way, never below zero.
    fn widened(self, margin: f64) -> Self;
}

impl Widened for (f64, f64) {
    fn widened(self, margin: f64) -> Self {
        ((self.0 - margin).max(0.0), self.1 + margin)
    }
}

/// `a + b` rounded, and its rounding error, exactly (two-sum).
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let sum = a + b;
    let back = sum - a;
    (sum, (a - (sum - back)) + (b - back))
}

/// Half of `high - low`, as an interval holding the exact value.
fn half_width((low, high): (f64, f64)) -> (f64, f64) {
    let (difference, error) = two_sum(high, -low);
    let (lower, upper) = if error > 0.0 {
        (difference, difference.next_up())
    } else if error < 0.0 {
        (difference.next_down(), difference)
    } else {
        (difference, difference)
    };
    (0.5 * lower.max(0.0), 0.5 * upper)
}

/// The midpoint of `low` and `high`, and how far the exact one can lie
/// from it.
fn midpoint((low, high): (f64, f64)) -> (f64, f64) {
    let (sum, error) = two_sum(low, high);
    let radius = if error == 0.0 {
        0.0
    } else {
        0.5 * (sum.next_up() - sum)
    };
    (0.5 * sum, radius)
}

/// Whether `point` lies inside the polygons, outside them, or too close to
/// a boundary edge (within `margin`) to tell.
fn placement(polygons: &[Polygon], point: Point, margin: f64) -> CentrePlacement {
    let rings = || {
        polygons
            .iter()
            .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
    };
    let nearest = rings()
        .flat_map(ring_segments)
        .map(|(a, b)| segment_distance(point, (a.x, a.y), (b.x, b.y)))
        .fold(f64::INFINITY, f64::min);
    if nearest <= margin {
        return CentrePlacement::Undecided;
    }
    let inside = polygons.iter().any(|polygon| {
        encloses(&polygon.outer, point) && !polygon.holes.iter().any(|hole| encloses(hole, point))
    });
    if inside {
        CentrePlacement::Inside
    } else {
        CentrePlacement::Outside
    }
}

/// Whether a point off the ring's boundary lies inside it (crossing count).
fn encloses(ring: &Ring, (x, y): Point) -> bool {
    let mut inside = false;
    for (a, b) in ring_segments(ring) {
        if (a.y > y) != (b.y > y) && x < a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y) {
            inside = !inside;
        }
    }
    inside
}

/// The distance from `point` to the segment from `a` to `b`.
fn segment_distance(point: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(point, (a.0 + t * dx, a.1 + t * dy))
}

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The convex hull of the triangles' plan vertices, with collinear points
/// dropped; a segment's hull is its two ends.
fn convex_hull(triangles: &[Triangle]) -> Vec<Point> {
    hull_of(
        triangles
            .iter()
            .flatten()
            .map(|point| (point.x, point.y))
            .collect(),
    )
}

/// The largest distance between a point of `first` and a point of `second`.
fn farthest(first: &[Point], second: &[Point]) -> f64 {
    first
        .iter()
        .flat_map(|a| second.iter().map(move |b| distance(*a, *b)))
        .fold(0.0, f64::max)
}

impl PlanSpanService for AxiolidPlanSpanService {
    fn measure_diameter(&self, object: &ObjectId) -> Result<PlanLength, PlanSpanError> {
        let footprint = self.footprint(object)?;
        let hull = Self::hull(object, &footprint)?;
        self.length(
            farthest(&hull, &hull),
            2.0 * footprint.deviation,
            format!("plan-diameter:{object}"),
        )
    }

    fn measure_span(
        &self,
        first: &ObjectId,
        second: &ObjectId,
        between: PlanSpan,
    ) -> Result<PlanLength, PlanSpanError> {
        let one = self.footprint(first)?;
        let other = self.footprint(second)?;
        let one_hull = Self::hull(first, &one)?;
        let other_hull = Self::hull(second, &other)?;
        let locator = format!("plan-span:{}:{first}:{second}", between.name());
        match between {
            PlanSpan::Farthest => self.length(
                farthest(&one_hull, &other_hull),
                one.deviation + other.deviation,
                locator,
            ),
            PlanSpan::Centres => {
                let (a, a_slack) = Self::centre(first, &one, &one_hull)?;
                let (b, b_slack) = Self::centre(second, &other, &other_hull)?;
                self.length(distance(a, b), a_slack + b_slack, locator)
            }
        }
    }

    fn measure_centre(&self, object: &ObjectId) -> Result<PlanCentre, PlanSpanError> {
        let footprint = self.footprint(object)?;
        let hull = Self::hull(object, &footprint)?;
        let (centre, slack, polygons) = Self::located(object, &footprint, &hull)?;
        let placement = placement(&polygons, centre, slack + footprint.deviation);
        let locator = format!(
            "plan-centre:{object}:({:.6},{:.6}):{}",
            centre.0,
            centre.1,
            match placement {
                CentrePlacement::Inside => "inside",
                CentrePlacement::Outside => "outside",
                CentrePlacement::Undecided => "on-boundary",
            }
        );
        let evidence = Evidence {
            source: object.source.clone(),
            locator,
            exact: slack == 0.0,
        };
        PlanCentre::try_new(
            object.clone(),
            [centre.0, centre.1],
            slack,
            placement,
            evidence,
        )
    }

    fn measure_rectangle(&self, object: &ObjectId) -> Result<PlanRectangle, PlanSpanError> {
        let footprint = self.footprint(object)?;
        let points: Vec<Point2> = footprint
            .soup
            .iter()
            .flatten()
            .map(|point| Point2::new(point.x, point.y))
            .collect();
        let measured = minimum_area_rectangle(&points).map_err(|error| match error {
            RectangleError::Empty => {
                PlanSpanError::Unavailable(format!("{object} has no footprint (no body)"))
            }
            _ => PlanSpanError::InvalidMeasurement,
        })?;
        let rectangle = measured.rectangle;
        let deviation = footprint.deviation;
        let orientation = if deviation > 0.0 {
            RectangleOrientation::Unproven
        } else if measured.evidence.minimal_orientations > 1 {
            RectangleOrientation::Tied
        } else {
            RectangleOrientation::Unique
        };
        let axes = rectangle.axes.map(|axis| [axis.x, axis.y]);
        // Along the coordinate axes nothing is turned: the extremes are
        // input coordinates, and only their differences and sums round.
        #[allow(clippy::float_cmp)]
        let (centre, radius, halves, turn) = if axes[0] == [1.0, 0.0] {
            let extremes = |coordinate: fn(&Point2) -> f64| {
                points
                    .iter()
                    .map(coordinate)
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                        (low.min(value), high.max(value))
                    })
            };
            let (x, y) = (extremes(|p| p.x), extremes(|p| p.y));
            let (cx, rx) = midpoint(x);
            let (cy, ry) = midpoint(y);
            ([cx, cy], rx + ry, [half_width(x), half_width(y)], 0.0)
        } else {
            let error = measured.evidence.error;
            let [a, b] = rectangle.half_extents;
            (
                [rectangle.centre.x, rectangle.centre.y],
                error,
                [(a, a).widened(error), (b, b).widened(error)],
                axis_error(a.max(b), error),
            )
        };
        let slack = radius + deviation;
        let halves = halves.map(|half| half.widened(deviation));
        #[allow(clippy::float_cmp)]
        let exact = slack == 0.0
            && turn == 0.0
            && halves.iter().all(|(low, high)| low == high)
            && orientation == RectangleOrientation::Unique;
        let locator = format!(
            "plan-rectangle:{object}:({:.6},{:.6}):{}",
            centre[0],
            centre[1],
            orientation.name()
        );
        let evidence = Evidence {
            source: object.source.clone(),
            locator,
            exact,
        };
        PlanRectangle::try_new(
            object.clone(),
            centre,
            slack,
            axes,
            turn,
            halves,
            orientation,
            evidence,
        )
    }
}

#[cfg(test)]
mod tests {
    use axiolid_core::Point3;

    use axiolid_overlay::{Polygon, Ring};
    use axioval_engine::CentrePlacement;

    use super::{convex_hull, farthest, placement};

    fn ring(points: &[(f64, f64)]) -> Ring {
        Ring {
            points: points
                .iter()
                .map(|(x, y)| axiolid_core::Point2::new(*x, *y))
                .collect(),
        }
    }

    #[test]
    fn a_centre_is_placed_inside_outside_or_on_the_boundary() {
        // A 4 x 4 square with a 2 x 2 hole in its middle.
        let square = Polygon {
            outer: ring(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]),
            holes: vec![ring(&[(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)])],
        };
        let polygons = [square];
        assert_eq!(
            placement(&polygons, (0.5, 2.0), 0.0),
            CentrePlacement::Inside
        );
        assert_eq!(
            placement(&polygons, (2.0, 2.0), 0.0),
            CentrePlacement::Outside
        );
        assert_eq!(
            placement(&polygons, (5.0, 2.0), 0.0),
            CentrePlacement::Outside
        );
        assert_eq!(
            placement(&polygons, (0.0, 2.0), 0.0),
            CentrePlacement::Undecided
        );
        // Within the margin of an edge, either side is possible.
        assert_eq!(
            placement(&polygons, (0.5, 2.0), 0.6),
            CentrePlacement::Undecided
        );
    }

    fn triangle(points: [(f64, f64); 3]) -> [Point3; 3] {
        points.map(|(x, y)| Point3::new(x, y, 0.0))
    }

    #[test]
    fn the_hull_drops_interior_and_collinear_points() {
        let hull = convex_hull(&[
            triangle([(0.0, 0.0), (4.0, 0.0), (2.0, 1.0)]),
            triangle([(4.0, 0.0), (4.0, 3.0), (2.0, 0.0)]),
            triangle([(0.0, 3.0), (4.0, 3.0), (0.0, 0.0)]),
        ]);
        assert_eq!(hull, [(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 3.0)]);
    }

    #[test]
    fn a_hull_diameter_matches_every_vertex_pair() {
        // An L-shaped footprint: its longest diagonal joins two outer corners.
        let soup = [
            triangle([(0.0, 0.0), (6.0, 0.0), (6.0, 2.0)]),
            triangle([(0.0, 0.0), (6.0, 2.0), (0.0, 2.0)]),
            triangle([(0.0, 2.0), (2.0, 2.0), (2.0, 5.0)]),
            triangle([(0.0, 2.0), (2.0, 5.0), (0.0, 5.0)]),
        ];
        let vertices: Vec<(f64, f64)> = soup.iter().flatten().map(|p| (p.x, p.y)).collect();
        let hull = convex_hull(&soup);
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(farthest(&hull, &hull), farthest(&vertices, &vertices));
        }
        assert!((farthest(&hull, &hull) - 61.0_f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn a_segment_is_its_own_hull() {
        let hull = convex_hull(&[triangle([(0.0, 0.0), (1.0, 0.0), (3.0, 0.0)])]);
        assert_eq!(hull, [(0.0, 0.0), (3.0, 0.0)]);
        assert!((farthest(&hull, &hull) - 3.0).abs() < 1e-12);
    }
}
