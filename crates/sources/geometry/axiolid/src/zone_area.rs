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
//! A tessellated footprint widens every bound by its band `2·P·d + π·d²`.

use axiolid_core::Point2;
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};
use axioval_engine::{PlanAreaError, PlanPolygon, ZoneArea, ZoneAreaRequest};
use axioval_ir::Evidence;

use crate::plan_area::{AxiolidPlanAreaService, band, tolerance};
use crate::planar::{
    Disc, bounded_footprint, footprint_measure, hull_of, kept_fan_triangle, overlay_refusal,
    plan_frame, polygon_area,
};

/// The smallest accuracy drawn as a band, in metres: below it the overlay
/// refuses the band's corners, so a smaller one is drawn at this width
/// (growing more still holds the true boundary).
const MINIMUM_BAND: f64 = 1e-6;

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
    let inside = widen((inside_sure, inside_possible.max(inside_sure)));
    let outside = widen((outside_sure, outside_possible.max(outside_sure)));
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
