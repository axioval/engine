//! Plan containment of footprints in a stated zone, for the plan-area
//! service (`PlanAreaService::measure_zone_area`).
//!
//! The objects' footprints are united by the overlay as a footprint is
//! (`planar::bounded_footprint`: slivers the overlay refuses are left out
//! and their area bounds what they could add). The part inside the zone is
//! measured as the overlay's intersection with it and the part outside as
//! the overlay's difference, each directly, so a footprint wholly inside
//! measures no area outside, exactly, rather than a small difference of two
//! large areas.
//!
//! A zone stated within an accuracy `a` has its true boundary within `a`
//! of the stated one. Its band, the stated boundary grown by `a`, is drawn
//! as the hull of each boundary segment grown by a circumscribed 16-gon
//! (so it holds every point within `a`), handed to the overlay as fan
//! triangles. The true zone then holds the stated zone less the band and
//! lies within the stated zone with the band: the area surely inside is
//! the footprint's intersection with the first, the area possibly inside
//! its intersection with the second, and the outside the other way round.
//! A tessellated footprint widens every bound by its band `2·P·d + π·d²`,
//! but only a part it could reach (#364): where every projected triangle,
//! grown by the deviation `d`, stays clear of the zone's band (or of its
//! boundary, stated exactly), each lies wholly on one side of the true
//! boundary, so a side no triangle lies on holds none of the true
//! footprint, exactly.

use axiolid_core::{Point2, Point3};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};
use axioval_engine::{PlanAreaError, PlanPolygon, ZoneArea, ZoneAreaRequest};
use axioval_ir::Evidence;

use crate::geometry::Triangle;
use crate::plan_area::{AxiolidPlanAreaService, band, tolerance};
use crate::planar::{
    Disc, bounded_footprint, footprint_measure, hull_of, kept_fan_triangle, overlay_refusal,
    plan_frame, polygon_area,
};

/// The smallest accuracy drawn as a band, in metres: below it the overlay
/// refuses the band's corners, so a smaller one is drawn at this width
/// (growing more still holds the true boundary).
const MINIMUM_BAND: f64 = 1e-6;

/// How far past the band's reach a triangle must stay for its side to be
/// read from one corner, in metres: well above the rounding of the
/// distances measured here.
const SIDE_MARGIN: f64 = 1e-6;

/// Which sides of the zone the true footprint may lie on, when the
/// footprint's deviation band cannot reach the zone's boundary band:
/// every projected triangle of `soup` farther than `reach` from every
/// boundary segment lies wholly on one side, read from its first corner.
/// `None` when a triangle comes within `reach`. Returns whether any
/// triangle lies inside and whether any lies outside.
fn sides(soup: &[Triangle], zone: &[PlanPolygon], reach: f64) -> Option<(bool, bool)> {
    let segments: Vec<(Point2, Point2)> = zone
        .iter()
        .flat_map(PlanPolygon::rings)
        .flat_map(|ring| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|([ax, ay], [bx, by])| (Point2::new(*ax, *ay), Point2::new(*bx, *by)))
        })
        .collect();
    let plan = |point: &Point3| Point2::new(point.x, point.y);
    let (mut inside, mut outside) = (false, false);
    for triangle in soup {
        let corners = triangle.map(|point| plan(&point));
        let (low, high) = corners.iter().fold(
            (
                Point2::new(f64::INFINITY, f64::INFINITY),
                Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
            ),
            |(low, high), p| {
                (
                    Point2::new(low.x.min(p.x), low.y.min(p.y)),
                    Point2::new(high.x.max(p.x), high.y.max(p.y)),
                )
            },
        );
        for &(a, b) in &segments {
            if a.x.max(b.x) < low.x - reach
                || a.x.min(b.x) > high.x + reach
                || a.y.max(b.y) < low.y - reach
                || a.y.min(b.y) > high.y + reach
            {
                continue;
            }
            let edges = [
                (corners[0], corners[1]),
                (corners[1], corners[2]),
                (corners[2], corners[0]),
            ];
            if edges
                .iter()
                .any(|edge| segment_distance(*edge, (a, b)) <= reach)
                || in_triangle(&corners, a)
            {
                return None;
            }
        }
        if in_zone(zone, corners[0]) {
            inside = true;
        } else {
            outside = true;
        }
    }
    Some((inside, outside))
}

/// Whether `point` lies inside `corners` (a triangle of positive area).
fn in_triangle(corners: &[Point2; 3], point: Point2) -> bool {
    let side = |a: Point2, b: Point2| (b - a).perp_dot(point - a);
    let signs = [
        side(corners[0], corners[1]),
        side(corners[1], corners[2]),
        side(corners[2], corners[0]),
    ];
    signs.iter().all(|s| *s > 0.0) || signs.iter().all(|s| *s < 0.0)
}

/// Whether `point` lies inside a polygon of `zone`, holes excluded.
fn in_zone(zone: &[PlanPolygon], point: Point2) -> bool {
    let in_ring = |ring: &[[f64; 2]]| {
        let mut inside = false;
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if (a[1] > point.y) != (b[1] > point.y) {
                let crossing = (b[0] - a[0]) * (point.y - a[1]) / (b[1] - a[1]) + a[0];
                if point.x < crossing {
                    inside = !inside;
                }
            }
        }
        inside
    };
    zone.iter().any(|polygon| {
        in_ring(polygon.outer()) && !polygon.holes().iter().any(|hole| in_ring(hole))
    })
}

/// The distance between two segments.
fn segment_distance(a: (Point2, Point2), b: (Point2, Point2)) -> f64 {
    let cross = |p: Point2, q: Point2, r: Point2| (q - p).perp_dot(r - p);
    if cross(a.0, a.1, b.0) * cross(a.0, a.1, b.1) < 0.0
        && cross(b.0, b.1, a.0) * cross(b.0, b.1, a.1) < 0.0
    {
        return 0.0;
    }
    let to = |point: Point2, (start, end): (Point2, Point2)| {
        let along = end - start;
        let length = along.length_squared();
        let t = if length > 0.0 {
            ((point - start).dot(along) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (point - (start + along * t)).length()
    };
    to(a.0, b).min(to(a.1, b)).min(to(b.0, a)).min(to(b.1, a))
}

/// A stated polygon as the overlay takes it.
pub(crate) fn overlay_polygon(polygon: &PlanPolygon) -> Polygon {
    let ring = |points: &[[f64; 2]]| Ring {
        points: points.iter().map(|[x, y]| Point2::new(*x, *y)).collect(),
    };
    Polygon {
        outer: ring(polygon.outer()),
        holes: polygon.holes().iter().map(|hole| ring(hole)).collect(),
    }
}

/// Every point within `radius` of the zone's boundary, as fan triangles
/// of each boundary segment's hull grown by a circumscribed 16-gon.
pub(crate) fn boundary_band(zone: &[PlanPolygon], radius: f64) -> Vec<Polygon> {
    let offsets = Disc::Circumscribed.vertices(radius.max(MINIMUM_BAND));
    let mut band = Vec::new();
    for ring in zone.iter().flat_map(PlanPolygon::rings) {
        for (start, end) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let hull = hull_of(
                [start, end]
                    .into_iter()
                    .flat_map(|[x, y]| offsets.iter().map(move |(dx, dy)| (x + dx, y + dy)))
                    .collect(),
            );
            let Some(&apex) = hull.first() else {
                continue;
            };
            for pair in hull[1..].windows(2) {
                let ring = Ring {
                    points: [apex, pair[0], pair[1]]
                        .into_iter()
                        .map(|(x, y)| Point2::new(x, y))
                        .collect(),
                };
                if kept_fan_triangle(&ring) {
                    band.push(Polygon {
                        outer: ring,
                        holes: Vec::new(),
                    });
                }
            }
        }
    }
    band
}

/// The overlay of two polygon sets, each filled as its non-zero union;
/// an empty operand is the empty set.
pub(crate) fn combine(
    first: Vec<Polygon>,
    second: Vec<Polygon>,
    operation: OverlayOperation,
) -> Result<Vec<Polygon>, PlanAreaError> {
    match (first.is_empty(), second.is_empty(), operation) {
        (true, _, _) | (_, true, OverlayOperation::Intersection) => return Ok(Vec::new()),
        (false, true, _) => return Ok(first),
        _ => {}
    }
    let input = |polygons| OverlayInput {
        frame: plan_frame(),
        polygons,
    };
    overlay(
        &input(first),
        &input(second),
        operation,
        FillRule::NonZero,
        tolerance()?,
    )
    .map(|result| result.polygons)
    .map_err(|error| PlanAreaError::Unavailable(overlay_refusal(&error).to_owned()))
}

pub(crate) fn area(polygons: &[Polygon]) -> f64 {
    polygons.iter().map(polygon_area).sum()
}

/// The request's footprint, wholly, inside and outside its zone.
pub(crate) fn measure(
    service: &AxiolidPlanAreaService,
    request: &ZoneAreaRequest,
) -> Result<ZoneArea, PlanAreaError> {
    let mut soup = Vec::new();
    let mut deviation = 0.0_f64;
    for object in request.objects() {
        let footprint = service.measure(object)?;
        soup.extend(footprint.soup);
        deviation = deviation.max(footprint.deviation);
    }
    let tolerance = tolerance()?;
    let named = || {
        request
            .objects()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let (whole, perimeter) = footprint_measure(&soup, tolerance).ok_or_else(|| {
        PlanAreaError::Unavailable(format!("the footprint of {} cannot be computed", named()))
    })?;
    let footprint = bounded_footprint(&soup, tolerance)
        .map_err(|error| PlanAreaError::Unavailable(overlay_refusal(&error).to_owned()))?;
    let zone: Vec<Polygon> = request.zone().iter().map(overlay_polygon).collect();
    let accuracy = request.accuracy_metres();
    // The stated zone less its band lies inside the true one, and the
    // stated zone with its band holds it.
    let (sure, possible) = if accuracy > 0.0 {
        let band = boundary_band(request.zone(), accuracy);
        (
            combine(zone.clone(), band.clone(), OverlayOperation::Difference)?,
            combine(zone, band, OverlayOperation::Union)?,
        )
    } else {
        (zone.clone(), zone)
    };
    let pieces = |zone: Vec<Polygon>, operation| {
        combine(footprint.polygons.clone(), zone, operation).map(|polygons| area(&polygons))
    };
    let inside_sure = pieces(sure.clone(), OverlayOperation::Intersection)?;
    let inside_possible = pieces(possible.clone(), OverlayOperation::Intersection)?;
    let outside_sure = pieces(possible, OverlayOperation::Difference)?;
    let outside_possible = pieces(sure, OverlayOperation::Difference)?;
    let slack = band(perimeter, deviation);
    let slivers = footprint.slivers;
    let footprint_bounds = if slack == 0.0 && slivers == 0.0 {
        (whole, whole)
    } else {
        ((whole - slack).max(0.0), whole + slack + slivers)
    };
    let widen = |(lower, upper): (f64, f64)| {
        if slack == 0.0 && slivers == 0.0 {
            (lower, upper)
        } else {
            (
                (lower - slack).max(0.0),
                (upper + slack + slivers).min(footprint_bounds.1),
            )
        }
    };
    // A side no triangle lies on, with every triangle's deviation band
    // clear of the zone's band, holds none of the true footprint (#364).
    let reach = if accuracy > 0.0 {
        accuracy.max(MINIMUM_BAND) / (std::f64::consts::PI / 16.0).cos()
    } else {
        0.0
    };
    let sided = if slack == 0.0 && slivers == 0.0 {
        None
    } else {
        sides(&soup, request.zone(), deviation + reach + SIDE_MARGIN)
    };
    let clear = |within: bool, bounds: (f64, f64)| match sided {
        Some(_) if !within => (0.0, 0.0),
        _ => widen(bounds),
    };
    let (some_inside, some_outside) = sided.unwrap_or((true, true));
    let inside = clear(some_inside, (inside_sure, inside_possible.max(inside_sure)));
    let outside = clear(
        some_outside,
        (outside_sure, outside_possible.max(outside_sure)),
    );
    #[allow(clippy::float_cmp)]
    let exact = [footprint_bounds, inside, outside]
        .iter()
        .all(|(lower, upper)| lower == upper);
    let locator = format!("zone-area:{}:accuracy={accuracy}", named());
    // Evidence about the objects cites their one source.
    let source = request
        .objects()
        .first()
        .map_or_else(|| service.source().clone(), |object| object.source.clone());
    ZoneArea::try_new(
        request.objects().to_vec(),
        footprint_bounds,
        inside,
        outside,
        Evidence {
            source,
            locator,
            exact,
        },
    )
}
