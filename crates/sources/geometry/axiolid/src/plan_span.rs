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
//! A section, the intersection of several exact footprints, takes its width
//! and length from the same rectangle over its outer vertices, and only
//! when its orientation is unique: another rectangle of least area may have
//! other sides. A tessellated member refuses, since the short side of a
//! least-area rectangle does not grow monotonically with the shape, so no
//! chord band bounds it. Recesses are the pockets between a footprint's
//! outer boundary and its convex hull, and a tessellated footprint refuses:
//! its chords make and hide recesses.
//!
//! A centre lies inside its footprint when it lies inside the measured one
//! farther from every boundary edge than the centre's own uncertainty plus
//! the chord deviation, outside likewise, and undecided otherwise: a centre
//! on the boundary of an exact footprint is undecided too.

use axiolid_core::Point2;
use axiolid_overlay::{
    FillRule, OverlayInput, OverlayOperation, Polygon, RectangleError, Ring,
    minimum_area_rectangle, overlay,
};
use axioval_engine::{
    CentrePlacement, PlanAreaError, PlanCentre, PlanLength, PlanRecess, PlanRecesses,
    PlanRectangle, PlanSection, PlanSpan, PlanSpanError, PlanSpanService, RectangleOrientation,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

use crate::geometry::{AxiolidGeometry, Triangle};
use crate::plan_area::{AxiolidPlanAreaService, Footprint, band, tolerance};
use crate::planar::{
    footprint_polygons, hull_of, plan_frame, polygon_area, polygon_moments, ring_segments,
};

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

    /// A length in `[lower, upper]` metres, exact exactly when a point.
    fn interval(
        &self,
        (lower, upper): (f64, f64),
        locator: String,
    ) -> Result<PlanLength, PlanSpanError> {
        #[allow(clippy::float_cmp)]
        let exact = lower == upper;
        PlanLength::try_new(
            lower,
            upper,
            Evidence {
                source: self.source.clone(),
                locator,
                exact,
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

/// The least-area rectangle enclosing a set of plan points, every value
/// bounded: the one rectangle `measure_rectangle`, sections and the shelf
/// layout all use.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Enclosing {
    /// The measured centre.
    pub(crate) centre: [f64; 2],
    /// How far the true centre can lie from [`Self::centre`].
    pub(crate) radius: f64,
    /// Two unit axes, the second the first turned a quarter
    /// counter-clockwise.
    pub(crate) axes: [[f64; 2]; 2],
    /// How far, in radians, each true axis can be turned.
    pub(crate) turn: f64,
    /// `(lower, upper)` half extents along the axes.
    pub(crate) halves: [(f64, f64); 2],
    /// How well the orientation is known.
    pub(crate) orientation: RectangleOrientation,
}

impl Enclosing {
    /// The short and the long side, each `(lower, upper)`: only for a
    /// unique orientation, since another rectangle of least area may have
    /// other sides.
    pub(crate) fn sides(&self) -> Option<[(f64, f64); 2]> {
        if self.orientation != RectangleOrientation::Unique {
            return None;
        }
        let [(a0, a1), (b0, b1)] = self.halves;
        Some([
            (2.0 * a0.min(b0), 2.0 * a1.min(b1)),
            (2.0 * a0.max(b0), 2.0 * a1.max(b1)),
        ])
    }
}

/// The least-area rectangle around `points`, lying within `deviation` of
/// the true shape's vertices: a positive deviation widens the centre and
/// the half extents and leaves the orientation unproven, and several
/// orientations of least area are tied.
pub(crate) fn least_area_rectangle(
    points: &[Point2],
    deviation: f64,
) -> Result<Enclosing, RectangleError> {
    let measured = minimum_area_rectangle(points)?;
    let rectangle = measured.rectangle;
    let orientation = if deviation > 0.0 {
        RectangleOrientation::Unproven
    } else if measured.evidence.minimal_orientations > 1 {
        RectangleOrientation::Tied
    } else {
        RectangleOrientation::Unique
    };
    let axes = rectangle.axes.map(|axis| [axis.x, axis.y]);
    // Along the coordinate axes nothing is turned: the extremes are input
    // coordinates, and only their differences and sums round.
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
    Ok(Enclosing {
        centre,
        radius: radius + deviation,
        axes,
        turn,
        halves: halves.map(|half| half.widened(deviation)),
        orientation,
    })
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

/// Pockets shallower than this are not reported as recesses: the overlay
/// rounds its output to a grid (axiolid/kernel#173), which can bend a
/// straight wall by about `1.5e-8` of the plan's extent.
const RECESS_RESOLUTION: f64 = 1e-6;

/// A recess: its mouth's ends, its width and its depth.
type Pocket = (Point, Point, f64, f64);

/// The recesses of polygons' outer boundaries against their convex hulls.
///
/// On a simple counter-clockwise outer ring, the ring vertices on the hull's
/// boundary (its vertices, and ring vertices lying on its edges) follow the
/// hull in order. Between two consecutive ones, any further ring vertices
/// bound a pocket; its mouth joins the two, which lie on one hull edge, since
/// a hull vertex between them would be a boundary vertex between them. The
/// depth is the farthest pocket vertex from the mouth's line: distance to a
/// line is linear on each side of it, so a vertex attains it. Holes are
/// enclosed courtyards, not recesses, and are not walked.
fn pockets(polygons: &[Polygon]) -> Vec<Pocket> {
    let mut found = Vec::new();
    for polygon in polygons {
        let mut ring: Vec<Point> = polygon.outer.points.iter().map(|p| (p.x, p.y)).collect();
        let signed: f64 = (0..ring.len())
            .map(|i| {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum();
        if signed < 0.0 {
            ring.reverse();
        }
        let hull = hull_of(ring.clone());
        if hull.len() < 3 {
            continue;
        }
        let on_hull: Vec<usize> = (0..ring.len())
            .filter(|index| {
                let point = ring[*index];
                hull.contains(&point)
                    || (0..hull.len()).any(|edge| {
                        segment_distance(point, hull[edge], hull[(edge + 1) % hull.len()])
                            <= RECESS_RESOLUTION
                    })
            })
            .collect();
        for (position, &start) in on_hull.iter().enumerate() {
            let end = on_hull[(position + 1) % on_hull.len()];
            let (a, b) = (ring[start], ring[end]);
            let width = distance(a, b);
            if width <= 0.0 {
                continue;
            }
            let mut index = (start + 1) % ring.len();
            let mut depth = 0.0_f64;
            while index != end {
                let p = ring[index];
                let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
                depth = depth.max(cross.abs() / width);
                index = (index + 1) % ring.len();
            }
            if depth > RECESS_RESOLUTION {
                found.push((a, b, width, depth));
            }
        }
    }
    found
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
        let Enclosing {
            centre,
            radius: slack,
            axes,
            turn,
            halves,
            orientation,
        } = least_area_rectangle(&points, footprint.deviation).map_err(|error| match error {
            RectangleError::Empty => {
                PlanSpanError::Unavailable(format!("{object} has no footprint (no body)"))
            }
            _ => PlanSpanError::InvalidMeasurement,
        })?;
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
    fn measure_recesses(&self, object: &ObjectId) -> Result<PlanRecesses, PlanSpanError> {
        let footprint = self.footprint(object)?;
        if footprint.deviation > 0.0 {
            return Err(PlanSpanError::Unavailable(format!(
                "{object} is tessellated: its chords make and hide recesses, so none are measured"
            )));
        }
        let polygons = Self::polygons(object, &footprint.soup)?;
        if polygons.is_empty() {
            return Err(PlanSpanError::Unavailable(format!(
                "{object} has no footprint (no body)"
            )));
        }
        let mut recesses = Vec::new();
        for (a, b, width, depth) in pockets(&polygons) {
            let locator = format!(
                "plan-recess:{object}:({:.6},{:.6})-({:.6},{:.6})",
                a.0, a.1, b.0, b.1
            );
            recesses.push(PlanRecess::try_new(
                [[a.0, a.1], [b.0, b.1]],
                self.length(width, 0.0, format!("{locator}:width"))?,
                self.length(depth, 0.0, format!("{locator}:depth"))?,
            )?);
        }
        let locator = format!("plan-recesses:{object}:{}", recesses.len());
        PlanRecesses::try_new(
            object.clone(),
            recesses,
            Evidence::exact(object.source.clone(), locator),
        )
    }

    fn measure_section(&self, objects: &[ObjectId]) -> Result<PlanSection, PlanSpanError> {
        let tolerance = tolerance()
            .map_err(|_| PlanSpanError::Unavailable("invalid overlay tolerance".into()))?;
        let mut section: Option<Vec<Polygon>> = None;
        for object in objects {
            let footprint = self.footprint(object)?;
            // The short side of a minimum-area rectangle does not grow
            // monotonically with the set, so a chord band cannot bound it.
            if footprint.deviation > 0.0 {
                return Err(PlanSpanError::Unavailable(format!(
                    "{object} is tessellated, so the width of a section through it is not bounded"
                )));
            }
            let own = Self::polygons(object, &footprint.soup)?;
            if own.is_empty() {
                return Err(PlanSpanError::Unavailable(format!(
                    "{object} has no footprint (no body)"
                )));
            }
            section = Some(match section {
                None => own,
                Some(current) if current.is_empty() => current,
                Some(current) => intersect(current, own, tolerance).ok_or_else(|| {
                    PlanSpanError::Unavailable(format!(
                        "the section through {object} cannot be computed"
                    ))
                })?,
            });
        }
        let polygons = section.unwrap_or_default();
        let area: f64 = polygons.iter().map(polygon_area).sum();
        let names: Vec<String> = objects.iter().map(ToString::to_string).collect();
        let locator = format!("plan-section:{}", names.join(","));
        let sides = if polygons.is_empty() || area <= 0.0 {
            None
        } else {
            let points: Vec<Point2> = polygons
                .iter()
                .flat_map(|polygon| polygon.outer.points.iter().copied())
                .collect();
            // The same rectangle `measure_rectangle` answers: its sides are
            // the section's only for a unique orientation.
            let rectangle = least_area_rectangle(&points, 0.0).map_err(|_| {
                PlanSpanError::Unavailable(format!("the section {locator} has no rectangle"))
            })?;
            let [short, long] = rectangle.sides().ok_or_else(|| {
                PlanSpanError::Unavailable(format!(
                    "several orientations enclose the section {locator} with the least area, so \
                     its width is not known"
                ))
            })?;
            Some((
                self.interval(short, format!("{locator}:width"))?,
                self.interval(long, format!("{locator}:length"))?,
            ))
        };
        let area = if sides.is_some() { area } else { 0.0 };
        PlanSection::try_new(
            objects.to_vec(),
            (area, area),
            sides,
            Evidence::exact(self.source.clone(), format!("{locator}:area")),
        )
    }
}

impl AxiolidPlanSpanService {
    /// The plan union of `object`'s triangles.
    fn polygons(object: &ObjectId, soup: &[Triangle]) -> Result<Vec<Polygon>, PlanSpanError> {
        let tolerance = tolerance()
            .map_err(|_| PlanSpanError::Unavailable("invalid overlay tolerance".into()))?;
        footprint_polygons(soup, tolerance).ok_or_else(|| {
            PlanSpanError::Unavailable(format!("the footprint of {object} cannot be computed"))
        })
    }
}

/// The intersection of two polygon sets, each filled as its non-zero union.
fn intersect(
    first: Vec<Polygon>,
    second: Vec<Polygon>,
    tolerance: axiolid_core::Tolerance,
) -> Option<Vec<Polygon>> {
    let first = OverlayInput {
        frame: plan_frame(),
        polygons: first,
    };
    let second = OverlayInput {
        frame: plan_frame(),
        polygons: second,
    };
    overlay(
        &first,
        &second,
        OverlayOperation::Intersection,
        FillRule::NonZero,
        tolerance,
    )
    .ok()
    .map(|result| result.polygons)
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
