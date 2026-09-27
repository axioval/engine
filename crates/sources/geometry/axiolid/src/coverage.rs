//! Effective coverage: the union of several sources' effect areas, clipped
//! to a subject's footprint.
//!
//! ADR 0004: this module measures the covered area; whether it is enough is
//! a rule's decision.
//!
//! Every effect is bracketed between an inner region, inside the true one,
//! and an outer region holding it:
//!
//! - **Grown**: the source's footprint dilated by the range with a stated
//!   side (`Region::dilate_inner` and `Region::dilate_outer`).
//! - **Visible**: the exact visibility polygon of the source's footprint
//!   centre in the free region, cut to a regular 64-gon inscribed in the
//!   range's disc (inner) or circumscribing it (outer).
//! - **Travel**: the free region is cut into convex cells, and each cell is
//!   judged by the shortest-path distance `D` at its centroid `g`, from the
//!   kernel's distance map. Within a convex cell of the free region `D` is
//!   1-Lipschitz, so every point lies within `D(g) ± ρ` for the cell's
//!   radius `ρ` about `g`: a cell is covered when `D(g) + ρ` is within the
//!   range, uncovered when `D(g) − ρ` is beyond it, and split otherwise,
//!   until a depth or cell budget leaves it undecided (outer only).
//!
//! Travel and sight stay in the free region: the subject's footprint less
//! the blockers' footprints, all blockers for the inner bounds and only the
//! certain ones for the outer. A source centre outside the free region
//! covers none of it; one on its boundary is unmeasured.
//!
//! Only exact meshes are measured. A tessellated subject or blocker refuses
//! the request; a tessellated or unmeasured source leaves its effect
//! unmeasured, which keeps the covered area's upper bound at the whole
//! footprint. The overlay snaps its output to a grid (axiolid/kernel#173),
//! so areas carry that rounding, as every plan area here does.

use axiolid_core::{Point2, Tolerance};
use axiolid_overlay::{Polygon, Region, Ring, union_soup};
use axiolid_route::{Unreachable, distance_map};
use axioval_engine::{
    CoverageEvidence, CoverageRequest, EffectMeets, EffectReach, PlanArea, PlanAreaError,
};
use axioval_ir::{Evidence, ObjectId};

use crate::plan_area::{AxiolidPlanAreaService, tolerance};
use crate::planar::{footprint_polygons, polygon_moments, ring_area};
use crate::walkable::{Plan, trapezoids};

/// Sides of the regular polygons bracketing the range's disc.
const DISC_SIDES: u32 = 64;

/// Below this area, in square metres, an overlap is not claimed: the
/// overlay's grid snapping could have made it.
const AREA_EPSILON: f64 = 1e-6;

/// Distance within which a centre counts as on a boundary.
const ON_BOUNDARY: f64 = 1e-9;

/// Distance-map queries per effect bound before the rest stays undecided.
const CELL_BUDGET: usize = 4096;

/// How often a travel cell is split at most.
const MAX_DEPTH: u32 = 7;

/// Relative rounding bound on a travel distance, and its absolute floor.
const DISTANCE_SLACK: f64 = 1e-9;

/// An effect's inner and outer regions, or why it was not measured.
type Effect = Result<(Region, Region), String>;

#[allow(clippy::too_many_lines)]
pub(crate) fn measure(
    service: &AxiolidPlanAreaService,
    request: &CoverageRequest,
) -> Result<CoverageEvidence, PlanAreaError> {
    let tolerance = tolerance()?;
    let subject = request.subject();
    let unavailable = |what: String| PlanAreaError::Unavailable(what);
    let exact_region = |object: &ObjectId, role: &str| -> Result<(Region, f64), PlanAreaError> {
        let footprint = service.measure(object)?;
        if footprint.deviation > 0.0 {
            return Err(unavailable(format!(
                "{role} {object} is a tessellation, so its footprint is not exact"
            )));
        }
        let polygons = footprint_polygons(&footprint.soup, tolerance).ok_or_else(|| {
            unavailable(format!(
                "the footprint of {role} {object} cannot be computed"
            ))
        })?;
        let region = Region::new(polygons, tolerance)
            .map_err(|error| unavailable(format!("the footprint of {object}: {error:?}")))?;
        Ok((region, footprint.area))
    };
    let (footprint, area) = exact_region(subject, "subject")?;
    if footprint.is_empty() || area <= 0.0 {
        return Err(unavailable(format!("{subject} has no plan footprint")));
    }
    let overlay = |error| unavailable(format!("the coverage of {subject}: {error:?}"));
    let (free_inner, free_outer) = match request.reach() {
        EffectReach::Grown => (Region::empty(), Region::empty()),
        EffectReach::Travel | EffectReach::Visible => {
            let (mut all, mut certain) = (Region::empty(), Region::empty());
            for blocker in request.blockers() {
                if service.has_no_body(blocker.object()) {
                    continue;
                }
                let (region, _) = exact_region(blocker.object(), "blocker")?;
                all = all.union(&region, tolerance).map_err(overlay)?;
                if blocker.is_certain() {
                    certain = certain.union(&region, tolerance).map_err(overlay)?;
                }
            }
            (
                footprint.difference(&all, tolerance).map_err(overlay)?,
                footprint.difference(&certain, tolerance).map_err(overlay)?,
            )
        }
    };
    let range = request.range_metres();
    let clipped = |effect: &Region| {
        effect
            .intersection(&footprint, tolerance)
            .map(|within| within.area())
            .map_err(|error| format!("the overlay could not clip it to the footprint: {error:?}"))
    };
    let mut inner_union = Region::empty();
    let mut outer_union = Region::empty();
    let mut unmeasured = false;
    let mut effects = Vec::with_capacity(request.sources().len());
    for source in request.sources() {
        let object = source.object();
        let effect = match request.reach() {
            EffectReach::Grown => grown(service, object, range, tolerance),
            reach => centre(service, object, tolerance).and_then(|centre| {
                let one = |free: &Region, inner: bool| match reach {
                    EffectReach::Travel => travel(free, centre, range, inner, tolerance),
                    _ => visible(free, centre, range, inner, tolerance),
                };
                Ok((one(&free_inner, true)?, one(&free_outer, false)?))
            }),
        }
        .and_then(|(inner, outer)| {
            let areas = (clipped(&inner)?, clipped(&outer)?);
            Ok((inner, outer, areas))
        });
        let meets = match effect {
            Ok((inner, outer, (inner_area, outer_area))) => {
                let meets = if inner_area > AREA_EPSILON {
                    EffectMeets::Surely
                } else if outer_area <= 0.0 {
                    EffectMeets::No
                } else {
                    EffectMeets::Possibly
                };
                let joined = (|| {
                    if source.is_certain() {
                        inner_union = inner_union.union(&inner, tolerance)?;
                    }
                    outer_union = outer_union.union(&outer, tolerance)?;
                    Ok(())
                })();
                joined.map_err(overlay)?;
                meets
            }
            Err(reason) => {
                unmeasured = true;
                EffectMeets::Unmeasured(reason)
            }
        };
        effects.push((object.clone(), meets));
    }
    let union_area = |union: &Region| {
        clipped(union)
            .map_err(|_| unavailable(format!("the coverage of {subject} cannot be computed")))
    };
    let lower = union_area(&inner_union)?.clamp(0.0, area);
    let upper = if unmeasured {
        area
    } else {
        union_area(&outer_union)?.clamp(lower, area)
    };
    let locator = format!(
        "coverage:{subject}:{}:{range}:{}",
        request.reach().as_str(),
        request
            .sources()
            .iter()
            .map(|source| source.object().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    #[allow(clippy::float_cmp)]
    let exact = lower == upper;
    let covered = PlanArea::try_new(
        lower,
        upper,
        Evidence {
            source: subject.source.clone(),
            locator,
            exact,
        },
    )?;
    let footprint = PlanArea::try_new(
        area,
        area,
        Evidence::exact(subject.source.clone(), format!("footprint:{subject}")),
    )?;
    CoverageEvidence::try_new(subject.clone(), footprint, covered, effects)
}

/// A source's exact footprint as a region; `None` for a bodiless source.
fn source_region(
    service: &AxiolidPlanAreaService,
    object: &ObjectId,
    tolerance: Tolerance,
) -> Result<Option<Region>, String> {
    if service.has_no_body(object) {
        return Ok(None);
    }
    let footprint = service.measure(object).map_err(|error| error.to_string())?;
    if footprint.deviation > 0.0 {
        return Err(format!(
            "{object} is a tessellation, so its effect is not exact"
        ));
    }
    let polygons = footprint_polygons(&footprint.soup, tolerance)
        .ok_or_else(|| format!("the footprint of {object} cannot be computed"))?;
    Region::new(polygons, tolerance)
        .map(Some)
        .map_err(|error| format!("the footprint of {object}: {error:?}"))
}

/// The footprint dilated by `range` on each side of the true dilation. A
/// bodiless source covers nothing.
fn grown(
    service: &AxiolidPlanAreaService,
    object: &ObjectId,
    range: f64,
    tolerance: Tolerance,
) -> Effect {
    let Some(region) = source_region(service, object, tolerance)? else {
        return Ok((Region::empty(), Region::empty()));
    };
    let failed = |error| format!("{object} grown by {range} m: {error:?}");
    Ok((
        region.dilate_inner(range, tolerance).map_err(failed)?,
        region.dilate_outer(range, tolerance).map_err(failed)?,
    ))
}

/// The centroid of a source's footprint.
fn centre(
    service: &AxiolidPlanAreaService,
    object: &ObjectId,
    tolerance: Tolerance,
) -> Result<Point2, String> {
    let region = source_region(service, object, tolerance)?
        .ok_or_else(|| format!("{object} has no body, so no centre"))?;
    let (mut area, mut x, mut y) = (0.0, 0.0, 0.0);
    for polygon in region.polygons() {
        let (a, mx, my) = polygon_moments(polygon);
        area += a;
        x += mx;
        y += my;
    }
    if area <= 0.0 {
        return Err(format!("{object} has no plan footprint, so no centre"));
    }
    Ok(Point2::new(x / area, y / area))
}

/// Where a point lies in a region: `Some(true)` inside, `Some(false)`
/// outside, `None` on (or within rounding of) its boundary.
fn locate(region: &Region, point: Point2) -> Option<bool> {
    let mut winding = 0_i32;
    for ring in region.boundary_rings() {
        let points = &ring.points;
        for index in 0..points.len() {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            if segment_distance(point, a, b) <= ON_BOUNDARY {
                return None;
            }
            let cross = (b.x - a.x) * (point.y - a.y) - (point.x - a.x) * (b.y - a.y);
            if a.y <= point.y && b.y > point.y && cross > 0.0 {
                winding += 1;
            } else if b.y <= point.y && a.y > point.y && cross < 0.0 {
                winding -= 1;
            }
        }
    }
    Some(winding != 0)
}

fn segment_distance(point: Point2, a: Point2, b: Point2) -> f64 {
    let along = b - a;
    let length = along.dot(along);
    let t = if length > 0.0 {
        ((point - a).dot(along) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point - (a + along * t)).length()
}

/// Where the centre stands in the free region: `Ok(false)` outside it, so
/// the effect is empty; an error on its boundary.
fn inside(free: &Region, centre: Point2) -> Result<bool, String> {
    if free.is_empty() {
        return Ok(false);
    }
    locate(free, centre).ok_or_else(|| {
        format!(
            "the centre ({:.6}, {:.6}) lies on the boundary of the free region",
            centre.x, centre.y
        )
    })
}

/// A regular polygon inscribed in (`inner`) or circumscribing the disc of
/// `radius` about `centre`.
fn disc(centre: Point2, radius: f64, inner: bool, tolerance: Tolerance) -> Result<Region, String> {
    let step = std::f64::consts::TAU / f64::from(DISC_SIDES);
    let (offset, reach) = if inner {
        (0.0, radius)
    } else {
        // Rounding the vertices may pull an edge inward by an ulp; the
        // margin keeps the polygon around the disc.
        (
            step / 2.0,
            radius / (step / 2.0).cos() * (1.0 + 1e-12) + 1e-12,
        )
    };
    let points = (0..DISC_SIDES)
        .map(|index| {
            let angle = offset + step * f64::from(index);
            Point2::new(
                centre.x + reach * angle.cos(),
                centre.y + reach * angle.sin(),
            )
        })
        .collect();
    Region::new(
        vec![Polygon {
            outer: Ring { points },
            holes: Vec::new(),
        }],
        tolerance,
    )
    .map_err(|error| format!("the range's disc: {error:?}"))
}

/// What the centre sees of the free region, within the range.
fn visible(
    free: &Region,
    centre: Point2,
    range: f64,
    inner: bool,
    tolerance: Tolerance,
) -> Result<Region, String> {
    if range <= 0.0 || !inside(free, centre)? {
        return Ok(Region::empty());
    }
    let seen = free
        .visibility_polygon(centre, tolerance)
        .map_err(|error| format!("the view from the centre: {error:?}"))?;
    seen.intersection(&disc(centre, range, inner, tolerance)?, tolerance)
        .map_err(|error| format!("the view within range: {error:?}"))
}

/// The part of the free region within `range` of travel from the centre:
/// surely (`inner`) or possibly.
fn travel(
    free: &Region,
    centre: Point2,
    range: f64,
    inner: bool,
    tolerance: Tolerance,
) -> Result<Region, String> {
    if !inside(free, centre)? {
        return Ok(Region::empty());
    }
    let map = distance_map(free.polygons(), &[], &[centre])
        .map_err(|error| format!("the travel distances from the centre: {error:?}"))?;
    let mut cells: Vec<([Point2; 3], u32)> = Vec::new();
    for polygon in free.polygons() {
        for piece in trapezoids(&Plan::piece(polygon.clone())) {
            let points = &piece.outer.points;
            for index in 1..points.len().saturating_sub(1) {
                cells.push(([points[0], points[index], points[index + 1]], 0));
            }
        }
    }
    let mut reached: Vec<Ring> = Vec::new();
    let mut budget = CELL_BUDGET;
    while let Some((corners, depth)) = cells.pop() {
        let g = Point2::new(
            (corners[0].x + corners[1].x + corners[2].x) / 3.0,
            (corners[0].y + corners[1].y + corners[2].y) / 3.0,
        );
        let radius = corners
            .iter()
            .map(|corner| (*corner - g).length())
            .fold(0.0, f64::max);
        let judged = if budget == 0 {
            None
        } else {
            budget -= 1;
            match map.nearest(g) {
                Ok(Ok(reach)) => {
                    let d = reach.route.length;
                    let slack = DISTANCE_SLACK * (1.0 + d);
                    if d + slack + radius <= range {
                        Some(true)
                    } else if d - slack - radius > range {
                        Some(false)
                    } else {
                        None
                    }
                }
                // No part of a convex cell of the free region is reachable
                // when its centroid is not.
                Ok(Err(Unreachable::DisconnectedComponents)) => Some(false),
                _ => None,
            }
        };
        match judged {
            Some(true) => reached.push(triangle_ring(corners)),
            Some(false) => {}
            None if depth < MAX_DEPTH && budget > 0 => {
                let [a, b, c] = corners;
                let mid = |p: Point2, q: Point2| Point2::new(0.5 * (p.x + q.x), 0.5 * (p.y + q.y));
                let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
                for child in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
                    cells.push((child, depth + 1));
                }
            }
            None => {
                if !inner {
                    reached.push(triangle_ring(corners));
                }
            }
        }
    }
    reached.retain(|ring| ring_area(ring).abs() > 0.0);
    let cells = |error| format!("the travel cells within range: {error:?}");
    Region::new(union_soup(&reached, tolerance).map_err(cells)?, tolerance).map_err(cells)
}

/// A cell as a counter-clockwise ring.
fn triangle_ring(corners: [Point2; 3]) -> Ring {
    let mut ring = Ring {
        points: corners.to_vec(),
    };
    if ring_area(&ring) < 0.0 {
        ring.points.reverse();
    }
    ring
}
