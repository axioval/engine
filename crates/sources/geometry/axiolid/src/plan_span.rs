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

use axioval_engine::{PlanAreaError, PlanLength, PlanSpan, PlanSpanError, PlanSpanService};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle};
use crate::plan_area::{AxiolidPlanAreaService, Footprint, band, tolerance};
use crate::planar::{footprint_polygons, polygon_moments};

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
            return Ok((centre, 0.0));
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

fn distance(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn cross(o: Point, a: Point, b: Point) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// The convex hull of the triangles' plan vertices (monotone chain), with
/// collinear points dropped; a segment's hull is its two ends.
fn convex_hull(triangles: &[Triangle]) -> Vec<Point> {
    let mut points: Vec<Point> = triangles
        .iter()
        .flatten()
        .map(|point| (point.x, point.y))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let mut hull: Vec<Point> = Vec::with_capacity(2 * points.len());
    for pass in [false, true] {
        let floor = hull.len() + 1;
        let ordered: Box<dyn Iterator<Item = &Point>> = if pass {
            Box::new(points.iter().rev().skip(1))
        } else {
            Box::new(points.iter())
        };
        for point in ordered {
            while hull.len() >= floor.max(2)
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0
            {
                hull.pop();
            }
            hull.push(*point);
        }
    }
    hull.pop();
    hull
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
}

#[cfg(test)]
mod tests {
    use axiolid_core::Point3;

    use super::{convex_hull, farthest};

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
