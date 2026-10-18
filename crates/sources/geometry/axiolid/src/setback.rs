//! Setback areas in front of walls, for the plan-area service
//! (`PlanAreaService::measure_setback`).
//!
//! A wall's setback along a plan direction `n` by a depth `d` is its
//! footprint swept along the segment from `0` to `d·n`, less the footprint:
//! the Minkowski sum of the footprint and the segment holds the footprint
//! and every point in front of an edge facing `n`, within `d` of it along
//! `n`. The sum distributes over a union, and a triangle swept along a
//! segment is the convex hull of the triangle and its translate, so each
//! plan triangle is swept on its own and exactly, handed to the overlay as
//! a fan of triangles (as a grown footprint is), and the overlay unites
//! them and takes the footprint away. Several directions are swept apart
//! and united.
//!
//! The least setback sweeps the sure directions by the least depth, the
//! most every direction by the most depth; the first lies within the
//! second, so the area, the part outside a zone and the part other
//! setbacks overlap each lie between their measures over the two. A zone's
//! accuracy is bracketed as `zone_area` brackets it (the stated zone less
//! its band, and with it), so the part surely outside is the least setback
//! less the zone with its band, and the part possibly outside the most
//! setback less the zone without it. A tessellated wall's true footprint
//! lies within its chord deviation `d` of the measured one, which moves
//! every swept boundary by as much, so each bound widens by the band
//! `2·P·d + π·d²` along the setbacks' perimeter `P`; slivers the overlay
//! refuses add their area to the upper bounds.

use axiolid_core::Point2;
use axiolid_overlay::{OverlayOperation, Polygon, Ring};
use axioval_engine::{
    PlanAreaError, PlanPolygon, SetbackArea, SetbackOther, SetbackRequest, SetbackStrip,
};
use axioval_ir::Evidence;

use crate::geometry::Triangle;
use crate::plan_area::{AxiolidPlanAreaService, band, tolerance};
use crate::planar::{
    bounded_footprint, hull_of, kept_fan_triangle, overlay_refusal, projected_polygons,
    ring_perimeter,
};
use crate::zone_area::{area, boundary_band, combine, overlay_polygon};

/// One wall's setback, swept at the least and at the most.
struct Swept {
    least: Vec<Polygon>,
    most: Vec<Polygon>,
    /// The wall's chord deviation, zero for an exact body.
    deviation: f64,
    /// The area of slivers its footprint left out.
    slivers: f64,
}

/// The plan triangles of `soup` swept along `offset`, as fan triangles of
/// each triangle's hull with its translate.
fn swept_triangles(soup: &[Triangle], offset: (f64, f64)) -> Vec<Polygon> {
    let mut swept = Vec::new();
    for polygon in projected_polygons(soup) {
        let hull = hull_of(
            polygon
                .outer
                .points
                .iter()
                .flat_map(|point| [(point.x, point.y), (point.x + offset.0, point.y + offset.1)])
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
                swept.push(Polygon {
                    outer: ring,
                    holes: Vec::new(),
                });
            }
        }
    }
    swept
}

/// `strip`'s wall swept along its directions (the sure ones only for the
/// least), the least by the least depth and the most by the most.
fn swept(service: &AxiolidPlanAreaService, strip: &SetbackStrip) -> Result<Swept, PlanAreaError> {
    let object = strip.object();
    let footprint = service.measure(object)?;
    let tolerance = tolerance()?;
    let bounded = bounded_footprint(&footprint.soup, tolerance)
        .map_err(|error| PlanAreaError::Unavailable(overlay_refusal(&error).to_owned()))?;
    if bounded.polygons.is_empty() {
        return Err(PlanAreaError::Unavailable(format!(
            "{object} has no footprint to sweep a setback from"
        )));
    }
    let (least_depth, most_depth) = strip.depth_metres();
    let sweep = |depth: f64, sure_only: bool| -> Result<Vec<Polygon>, PlanAreaError> {
        if depth <= 0.0 {
            return Ok(Vec::new());
        }
        let mut pieces = Vec::new();
        for direction in strip.directions() {
            if sure_only && !direction.is_sure() {
                continue;
            }
            let [x, y] = direction.direction();
            pieces.extend(swept_triangles(&footprint.soup, (x * depth, y * depth)));
        }
        if pieces.is_empty() {
            return Ok(Vec::new());
        }
        combine(
            pieces,
            bounded.polygons.clone(),
            OverlayOperation::Difference,
        )
    };
    Ok(Swept {
        least: sweep(least_depth, true)?,
        most: sweep(most_depth, false)?,
        deviation: footprint.deviation,
        slivers: bounded.slivers,
    })
}

/// The overlay's polygons as stated plan polygons; a ring the overlay
/// returns that encloses no area is no part of a setback.
fn stated(polygons: &[Polygon]) -> Vec<PlanPolygon> {
    let ring = |ring: &Ring| -> Vec<[f64; 2]> {
        ring.points.iter().map(|point| [point.x, point.y]).collect()
    };
    polygons
        .iter()
        .filter_map(|polygon| {
            PlanPolygon::try_new(
                ring(&polygon.outer),
                polygon.holes.iter().map(ring).collect(),
            )
            .ok()
        })
        .collect()
}

/// What tessellated walls and refused slivers may move the measured
/// areas by: the largest chord deviation, the perimeter along which it
/// moves a boundary, and the slivers' area.
#[derive(Default)]
struct Slack {
    deviation: f64,
    perimeter: f64,
    slivers: f64,
}

impl Slack {
    /// `swept`'s share: its deviation along both its setbacks' boundaries
    /// and its slivers.
    fn add(&mut self, swept: &Swept) {
        self.deviation = self.deviation.max(swept.deviation);
        self.perimeter += perimeter(&swept.least) + perimeter(&swept.most);
        self.slivers += swept.slivers;
    }

    /// `(lower, upper)` widened by the band and the slivers.
    fn widen(&self, (lower, upper): (f64, f64)) -> (f64, f64) {
        let band = band(self.perimeter, self.deviation);
        if band == 0.0 && self.slivers == 0.0 {
            (lower, upper)
        } else {
            ((lower - band).max(0.0), upper + band + self.slivers)
        }
    }
}

fn perimeter(polygons: &[Polygon]) -> f64 {
    polygons
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .map(ring_perimeter)
        .sum()
}

/// The overlap of the wall's setback with the other walls': surely by the
/// sure others' least setbacks, possibly by every other's most, and the
/// walls whose most setback overlaps the wall's.
fn overlap(
    service: &AxiolidPlanAreaService,
    subject: &Swept,
    others: &[SetbackOther],
    slack: &mut Slack,
) -> Result<((f64, f64), Vec<axioval_ir::ObjectId>), PlanAreaError> {
    let (mut sure, mut every, mut overlapping) = (Vec::new(), Vec::new(), Vec::new());
    for other in others {
        let swept = swept(service, other.strip())?;
        let touching = area(&combine(
            subject.most.clone(),
            swept.most.clone(),
            OverlayOperation::Intersection,
        )?);
        // The other's boundary moves by its own deviation.
        slack.add(&swept);
        if touching > 0.0 || swept.deviation > 0.0 || swept.slivers > 0.0 {
            overlapping.push(other.strip().object().clone());
        }
        if other.is_sure() {
            sure.extend(swept.least);
        }
        every.extend(swept.most);
    }
    let least = area(&combine(
        subject.least.clone(),
        sure,
        OverlayOperation::Intersection,
    )?);
    let most = area(&combine(
        subject.most.clone(),
        every,
        OverlayOperation::Intersection,
    )?);
    Ok(((least, most.max(least)), overlapping))
}

/// The request's setback, its area, and the parts asked for.
pub(crate) fn measure(
    service: &AxiolidPlanAreaService,
    request: &SetbackRequest,
) -> Result<SetbackArea, PlanAreaError> {
    let strip = request.strip();
    let object = strip.object();
    let subject = swept(service, strip)?;
    let whole = (area(&subject.least), area(&subject.most));
    let mut slack = Slack::default();
    slack.add(&subject);
    let outside = match request.zone() {
        None => None,
        Some((zone, accuracy)) => {
            let polygons: Vec<Polygon> = zone.iter().map(overlay_polygon).collect();
            let (sure, possible) = if accuracy > 0.0 {
                let band = boundary_band(zone, accuracy);
                (
                    combine(polygons.clone(), band.clone(), OverlayOperation::Difference)?,
                    combine(polygons, band, OverlayOperation::Union)?,
                )
            } else {
                (polygons.clone(), polygons)
            };
            let least = area(&combine(
                subject.least.clone(),
                possible,
                OverlayOperation::Difference,
            )?);
            let most = area(&combine(
                subject.most.clone(),
                sure,
                OverlayOperation::Difference,
            )?);
            Some((least, most.max(least)))
        }
    };
    let (overlap, overlapping) = if request.others().is_empty() {
        (None, Vec::new())
    } else {
        let (bounds, overlapping) = overlap(service, &subject, request.others(), &mut slack)?;
        (Some(bounds), overlapping)
    };
    let area_bounds = slack.widen((whole.0, whole.1.max(whole.0)));
    let part = |bounds: (f64, f64)| {
        let (lower, upper) = slack.widen(bounds);
        (lower, upper.min(area_bounds.1))
    };
    let outside = outside.map(part);
    let overlap = overlap.map(part);
    #[allow(clippy::float_cmp)]
    let exact = [Some(area_bounds), outside, overlap]
        .iter()
        .flatten()
        .all(|(lower, upper)| lower == upper);
    let (least_depth, most_depth) = strip.depth_metres();
    let locator = format!(
        "setback:{object}:depth={least_depth}..{most_depth}:directions={}",
        strip
            .directions()
            .iter()
            .map(|direction| {
                let [x, y] = direction.direction();
                let sure = if direction.is_sure() { "" } else { "?" };
                format!("({x:.6},{y:.6}){sure}")
            })
            .collect::<Vec<_>>()
            .join(",")
    );
    SetbackArea::try_new(
        object.clone(),
        (stated(&subject.least), stated(&subject.most)),
        area_bounds,
        outside,
        overlap,
        overlapping,
        Evidence {
            source: object.source.clone(),
            locator,
            exact,
            witness: None,
        },
    )
}
