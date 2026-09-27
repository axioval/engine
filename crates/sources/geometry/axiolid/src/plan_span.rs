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
//! A centre lies inside its footprint when it lies inside the measured one
//! farther from every boundary edge than the centre's own uncertainty plus
//! the chord deviation, outside likewise, and undecided otherwise: a centre
//! on the boundary of an exact footprint is undecided too.

use axiolid_overlay::{Polygon, Ring};
use axioval_engine::{
    CentrePlacement, PlanAreaError, PlanCentre, PlanLength, PlanSpan, PlanSpanError,
    PlanSpanService,
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
