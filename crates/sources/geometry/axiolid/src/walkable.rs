//! Walkable plan domains shared by the walkability and metric-routing
//! services.
//!
//! ADR 0004: this module measures where a body of a given width can stand and
//! move; whether a route is acceptable is a capability's decision.
//!
//! # What is walkable
//!
//! A walkable surface is a closed exact body whose underside is one
//! horizontal plane (a space's floor). Its **free region** is its plan
//! footprint minus the plan footprint of every obstacle's part inside the
//! headroom band above that floor. A portal (a door or an opening) adds a
//! **corridor**: its plan extent along the wall, extruded through its
//! thickness until each side's surface begins, minus the obstacles in the
//! band above the portal's own sill. A portal's own body is treated as open:
//! its leaf and lining are not obstacles, so the corridor alone bounds its
//! clear width from above, never from below.
//!
//! # Soundness without one-sided erosion
//!
//! The published `axiolid-overlay` 0.3.0 erodes by a round-joined
//! approximation of a disc and does not state on which side of the exact
//! morphology the result lies, so it is not used. Candidate paths come from
//! a visibility graph through the free region less an enclosure of its
//! boundary's disc sweep, which lies inside the exact erosion. A path is
//! accepted as a witness only after an outer polygon enclosing its exact
//! sweep (segment rectangles and circumscribed polygons at the vertices,
//! grown by [`MARGIN`]) is shown to lie inside the free region with exact
//! booleans. Upper width bounds come from chords: a disc crossing a portal's
//! mid-line covers a chord of its own diameter, so the longest free interval
//! of that line bounds every body that crosses.

use axiolid_core::{Point2, Point3, Tolerance, Vec2};
use axiolid_mesh::{TriMesh, TriangleMeshView, audit_mesh};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};
use axioval_ir::ObjectId;
use std::collections::BTreeMap;

use crate::derived_relationships::{convex_hull, span, thin_axis};
use crate::geometry::{AxiolidGeometry, Extent, Triangle, mesh_extent, triangles};
use crate::planar::{plan_frame, polygon_area, projected_polygons, ring_area};

/// Distance below which two points coincide, and the kernel tolerance.
pub(crate) const ON_SURFACE: f64 = 1e-9;

/// How far past a portal's faces a surface may begin and still be its side.
pub(crate) const REACH: f64 = 1.0;

/// How far every witness sweep is grown before it is compared with the free
/// region. It dominates the overlay's grid snapping (axiolid/kernel#173), so
/// a sweep touching an obstacle can never be snapped inside.
pub(crate) const MARGIN: f64 = 1e-4;

/// How far past the side's surface boundary a portal's landing disc stands.
const LANDING: f64 = 0.01;

/// Sides of the polygon circumscribing a disc of `radius`. It overshoots the
/// disc by `radius * (1/cos(pi/sides) - 1)`: at most 0.12 % of a radius from
/// 5 cm up, and under a millimetre below, which stays inside the slack
/// [`witness`] erodes with. Small discs get few sides because the published
/// overlay (0.3.0) mistakes a polygon with very short edges for a
/// self-intersecting one.
fn circumscribed_sides(radius: f64) -> u32 {
    if radius >= 0.05 {
        64
    } else if radius >= 0.005 {
        16
    } else {
        8
    }
}

/// An upper width bound that bounds nothing: the passage's width is not
/// measured, only its existence.
pub(crate) const UNBOUNDED: f64 = f64::MAX;

/// Vertices a visibility-graph route may use before it is refused.
pub(crate) const ROUTE_BUDGET: usize = axiolid_route::MAX_VERTICES;

pub(crate) fn tolerance() -> Result<Tolerance, String> {
    Tolerance::new(ON_SURFACE, ON_SURFACE).map_err(|_| "invalid tolerance".to_owned())
}

/// The tolerance handed to the overlay, which uses it only to validate its
/// operands. The published validator (0.3.0) compares a cross product, an
/// area, with it, so a thin but valid trapezoid of a millimetre by a tenth of
/// a micrometre fails at [`ON_SURFACE`]. Operands here are convex pieces and
/// fresh convex polygons, never self-intersecting, so the check can be tight.
fn overlay_tolerance() -> Result<Tolerance, String> {
    Tolerance::new(1e-15, 1e-15).map_err(|_| "invalid tolerance".to_owned())
}

/// A plan bounding box `(min, max)`.
pub(crate) type Bounds2 = ([f64; 2], [f64; 2]);

/// A plan region: the polygons an overlay produced.
///
/// It is never handed back to the overlay as it is. The published overlay
/// (0.3.0) rejects its own output as input when a ring has collinear
/// non-adjacent edges, which every union of rooms and corridors has. Every
/// operation therefore cuts its operands into trapezoids first
/// ([`trapezoids`]), which are convex and always valid, and whose union is
/// the region.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    polygons: Vec<Polygon>,
}

impl Plan {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn polygons(&self) -> &[Polygon] {
        &self.polygons
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    pub(crate) fn area(&self) -> f64 {
        self.polygons.iter().map(polygon_area).sum()
    }

    /// One polygon of this plan as a plan of its own.
    pub(crate) fn piece(polygon: Polygon) -> Self {
        Self {
            polygons: vec![polygon],
        }
    }
}

/// Slab heights closer than this are one slab: the overlay snaps its output
/// to a grid, and slivers of snapping noise are not geometry.
const SLAB: f64 = 1e-7;

/// A polygon soup covering `plan` exactly, of convex trapezoids between
/// consecutive vertex heights.
///
/// Heights closer than [`SLAB`] merge, so no trapezoid is thinner; an edge
/// both of whose ends fall in one merged height is horizontal.
pub(crate) fn trapezoids(plan: &Plan) -> Vec<Polygon> {
    let mut pieces = Vec::new();
    for shape in &plan.polygons {
        let rings: Vec<&Ring> = std::iter::once(&shape.outer).chain(&shape.holes).collect();
        let mut heights: Vec<f64> = rings
            .iter()
            .flat_map(|ring| ring.points.iter().map(|p| p.y))
            .collect();
        heights.sort_by(f64::total_cmp);
        let mut levels: Vec<f64> = Vec::new();
        for y in heights {
            if levels.last().is_none_or(|last| y - last > SLAB) {
                levels.push(y);
            }
        }
        let level = |y: f64| levels.iter().rposition(|l| *l <= y + SLAB).unwrap_or(0);
        let edges: Vec<(Point2, Point2, usize, usize)> = rings
            .iter()
            .flat_map(|ring| edges(ring))
            .filter_map(|(a, b)| {
                let (a, b) = if a.y <= b.y { (a, b) } else { (b, a) };
                let (low, high) = (level(a.y), level(b.y));
                (high > low).then_some((a, b, low, high))
            })
            .collect();
        let x_at = |(a, b, low, high): &(Point2, Point2, usize, usize), index: usize| {
            if index == *low {
                a.x
            } else if index == *high {
                b.x
            } else {
                a.x + (b.x - a.x) * (levels[index] - a.y) / (b.y - a.y)
            }
        };
        for index in 0..levels.len().saturating_sub(1) {
            let mut crossing: Vec<(f64, f64, f64)> = edges
                .iter()
                .filter(|edge| edge.2 <= index && edge.3 > index)
                .map(|edge| {
                    let (bottom, top) = (x_at(edge, index), x_at(edge, index + 1));
                    (f64::midpoint(bottom, top), bottom, top)
                })
                .collect();
            crossing.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (y0, y1) = (levels[index], levels[index + 1]);
            for pair in crossing.chunks_exact(2) {
                pieces.extend(polygon(vec![
                    Point2::new(pair[0].1, y0),
                    Point2::new(pair[1].1, y0),
                    Point2::new(pair[1].2, y1),
                    Point2::new(pair[0].2, y1),
                ]));
            }
        }
    }
    pieces
}

fn overlay_soups(
    subject: Vec<Polygon>,
    clip: Vec<Polygon>,
    operation: OverlayOperation,
) -> Result<Plan, String> {
    let frame = plan_frame();
    let result = overlay(
        &OverlayInput {
            frame,
            polygons: subject,
        },
        &OverlayInput {
            frame,
            polygons: clip,
        },
        operation,
        FillRule::NonZero,
        overlay_tolerance()?,
    )
    .map_err(|error| format!("plan {operation:?} failed: {error:?}"))?;
    Ok(Plan {
        polygons: result.polygons,
    })
}

/// The union of a soup of fresh polygons under the non-zero fill rule.
///
/// Orientation is kept: overlapping polygons of opposite winding cancel, which
/// is what a section through a closed body needs. Callers wanting a plain
/// union orient every polygon counter-clockwise first.
pub(crate) fn union(polygons: Vec<Polygon>) -> Result<Plan, String> {
    if polygons.is_empty() {
        return Ok(Plan::empty());
    }
    overlay_soups(polygons.clone(), polygons, OverlayOperation::Union)
}

pub(crate) fn join(a: &Plan, b: &Plan) -> Result<Plan, String> {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => Ok(b.clone()),
        (_, true) => Ok(a.clone()),
        _ => overlay_soups(trapezoids(a), trapezoids(b), OverlayOperation::Union),
    }
}

pub(crate) fn subtract(a: &Plan, b: &Plan) -> Result<Plan, String> {
    if a.is_empty() || b.is_empty() {
        return Ok(a.clone());
    }
    overlay_soups(trapezoids(a), trapezoids(b), OverlayOperation::Difference)
}

fn intersect(a: &Plan, b: &Plan) -> Result<Plan, String> {
    if a.is_empty() || b.is_empty() {
        return Ok(Plan::empty());
    }
    overlay_soups(trapezoids(a), trapezoids(b), OverlayOperation::Intersection)
}

/// A ring from `points` without repeated consecutive points; `None` when it
/// bounds no area.
fn ring(points: Vec<Point2>) -> Option<Ring> {
    let mut clean: Vec<Point2> = Vec::with_capacity(points.len());
    for point in points {
        if clean
            .last()
            .is_none_or(|last: &Point2| (*last - point).length() > ON_SURFACE)
        {
            clean.push(point);
        }
    }
    while clean.len() > 1 && (clean[0] - clean[clean.len() - 1]).length() <= ON_SURFACE {
        clean.pop();
    }
    let ring = Ring { points: clean };
    (ring.points.len() >= 3 && ring_area(&ring).abs() > ON_SURFACE * ON_SURFACE * 4.0)
        .then_some(ring)
}

/// A polygon from its outer points, counter-clockwise.
pub(crate) fn polygon(points: Vec<Point2>) -> Option<Polygon> {
    let mut outer = ring(points)?;
    if ring_area(&outer) < 0.0 {
        outer.points.reverse();
    }
    Some(Polygon {
        outer,
        holes: Vec::new(),
    })
}

/// Keeps the part of a convex polygon on one side of `z = level`.
fn clip_z(polygon: &[Point3], level: f64, keep_above: bool) -> Vec<Point3> {
    let inside = |p: &Point3| {
        if keep_above {
            p.z >= level
        } else {
            p.z <= level
        }
    };
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for index in 0..polygon.len() {
        let current = polygon[index];
        let next = polygon[(index + 1) % polygon.len()];
        if inside(&current) {
            out.push(current);
        }
        if inside(&current) != inside(&next) {
            let t = (level - current.z) / (next.z - current.z);
            let mut crossing = current + (next - current) * t;
            crossing.z = level;
            out.push(crossing);
        }
    }
    out
}

/// A clipped face projected to plan, orientation kept.
fn projected(face: &[Point3]) -> Option<Polygon> {
    let outer = ring(face.iter().map(|p| Point2::new(p.x, p.y)).collect())?;
    Some(Polygon {
        outer,
        holes: Vec::new(),
    })
}

/// Plan projection of the part of `mesh` inside the open band `lo < z < hi`.
///
/// A vertical line meets the body within the band exactly when its foot just
/// above `lo` is inside the body, or it crosses the body's boundary within
/// the band. The second set is the union of the boundary clipped to the band;
/// the first is the non-zero winding of the boundary above `lo` seen from
/// below, which needs a closed, consistently wound body. A body touching the
/// band only at `lo` or `hi` does not obstruct it.
///
/// # Errors
///
/// When a body reaching down to the band is not a closed solid, or the
/// overlay fails.
pub(crate) fn band_footprint(
    object: &ObjectId,
    mesh: &TriMesh,
    lo: f64,
    hi: f64,
) -> Result<Plan, String> {
    let Some((min, max)) = mesh_extent(mesh) else {
        return Ok(Plan::empty());
    };
    if max[2] <= lo + ON_SURFACE || min[2] >= hi - ON_SURFACE {
        return Ok(Plan::empty());
    }
    let faces = triangles(mesh);
    let mut crossings = Vec::new();
    for [a, b, c] in &faces {
        let clipped = clip_z(&clip_z(&[*a, *b, *c], lo, true), hi, false);
        if clipped.len() < 3
            || clipped.iter().all(|p| p.z <= lo + ON_SURFACE)
            || clipped.iter().all(|p| p.z >= hi - ON_SURFACE)
        {
            continue;
        }
        if let Some(mut face) = projected(&clipped) {
            if ring_area(&face.outer) < 0.0 {
                face.outer.points.reverse();
            }
            crossings.push(face);
        }
    }
    let mut region = union(crossings)?;
    if min[2] <= lo + ON_SURFACE {
        let health = audit_mesh(mesh, tolerance()?);
        if !health.is_closed_two_manifold() {
            return Err(format!(
                "{object} reaches below the headroom band but is not a closed solid, so what \
                 it occupies in the band is undecided"
            ));
        }
        let mut above = Vec::new();
        for [a, b, c] in &faces {
            let clipped = clip_z(&[*a, *b, *c], lo, true);
            if clipped.len() < 3 || clipped.iter().all(|p| p.z <= lo + ON_SURFACE) {
                continue;
            }
            above.extend(projected(&clipped));
        }
        region = join(&region, &union(above)?)?;
    }
    Ok(region)
}

/// A walkable surface: a closed exact body standing on one horizontal floor.
#[derive(Clone, Debug)]
pub(crate) struct Floor {
    pub(crate) id: ObjectId,
    /// Floor elevation.
    pub(crate) z0: f64,
    pub(crate) top: f64,
    pub(crate) footprint: Plan,
    pub(crate) bounds: Bounds2,
}

/// Reads a walkable surface, refusing anything that is not an exact closed
/// body with one horizontal underside.
pub(crate) fn floor(geometry: &AxiolidGeometry, id: &ObjectId) -> Result<Floor, String> {
    let mesh = body(geometry, id, "walkable surface")?;
    if geometry.is_tessellated(id) {
        return Err(format!(
            "walkable surface {id} is a tessellation, so its floor is approximate"
        ));
    }
    if !audit_mesh(mesh, tolerance()?).is_closed_two_manifold() {
        return Err(format!("walkable surface {id} is not a closed solid"));
    }
    let (min, max) = mesh_extent(mesh).ok_or_else(|| format!("{id} has an empty mesh"))?;
    let faces = triangles(mesh);
    let footprint = union(projected_polygons(&faces))?;
    let underside: Vec<Triangle> = faces
        .iter()
        .filter(|face| face.iter().all(|p| (p.z - min[2]).abs() <= ON_SURFACE))
        .copied()
        .collect();
    let floor_area = union(projected_polygons(&underside))?.area();
    let area = footprint.area();
    if area <= ON_SURFACE || (area - floor_area).abs() > 1e-9 * area.max(1.0) {
        return Err(format!(
            "the underside of walkable surface {id} is not one horizontal floor"
        ));
    }
    Ok(Floor {
        id: id.clone(),
        z0: min[2],
        top: max[2],
        footprint,
        bounds: ([min[0], min[1]], [max[0], max[1]]),
    })
}

/// An object's mesh, or why it has none a measurement could use.
pub(crate) fn body<'g>(
    geometry: &'g AxiolidGeometry,
    id: &ObjectId,
    role: &str,
) -> Result<&'g TriMesh, String> {
    if let Some(mesh) = geometry.mesh(id) {
        return Ok(mesh);
    }
    if let Some((_, reason)) = geometry.unmeasured().find(|(object, _)| *object == id) {
        return Err(format!(
            "{role} {id} has a body that was not measured: {reason}"
        ));
    }
    if geometry.has_no_body(id) {
        return Err(format!("{role} {id} has no body to measure"));
    }
    Err(format!("{role} {id} has no described geometry"))
}

/// An obstacle with a mesh, and the extent enclosing its true body.
pub(crate) struct Obstacle<'g> {
    pub(crate) id: &'g ObjectId,
    pub(crate) mesh: &'g TriMesh,
    pub(crate) extent: Extent,
    pub(crate) tessellated: bool,
}

/// The obstacles among `ids`: bodiless ones obstruct nothing; an unmeasured
/// or undescribed one has an unknown extent, so it refuses.
pub(crate) fn obstacles<'g>(
    geometry: &'g AxiolidGeometry,
    ids: impl IntoIterator<Item = &'g ObjectId>,
) -> Result<Vec<Obstacle<'g>>, String> {
    let mut found = Vec::new();
    for id in ids {
        if geometry.mesh(id).is_none() && geometry.has_no_body(id) {
            continue;
        }
        let mesh = body(geometry, id, "obstacle")?;
        let extent = geometry.enclosing_extent(id).ok_or_else(|| {
            format!("obstacle {id} has an empty mesh or an invalid chord deviation")
        })?;
        found.push(Obstacle {
            id,
            mesh,
            extent,
            tessellated: geometry.is_tessellated(id),
        });
    }
    Ok(found)
}

fn bounds_gap(a: &Bounds2, b: &Bounds2) -> f64 {
    (0..2)
        .map(|axis| {
            let gap = (b.0[axis] - a.1[axis]).max(a.0[axis] - b.1[axis]).max(0.0);
            gap * gap
        })
        .sum::<f64>()
        .sqrt()
}

pub(crate) fn plan_gap(a: &Bounds2, b: &Extent) -> f64 {
    bounds_gap(a, &([b.0[0], b.0[1]], [b.1[0], b.1[1]]))
}

/// What the obstacles occupy inside the band `lo < z < hi` over `area`.
///
/// # Errors
///
/// When a tessellated obstacle could change it, or a band footprint cannot
/// be computed.
pub(crate) fn obstruction(
    obstacles: &[Obstacle<'_>],
    area: &Bounds2,
    lo: f64,
    hi: f64,
) -> Result<Plan, String> {
    let mut region = Plan::empty();
    for obstacle in obstacles {
        let (min, max) = obstacle.extent;
        if plan_gap(area, &obstacle.extent) > ON_SURFACE
            || max[2] <= lo + ON_SURFACE
            || min[2] >= hi - ON_SURFACE
        {
            continue;
        }
        if obstacle.tessellated {
            return Err(format!(
                "obstacle {} is a tessellation inside the headroom band, so what it occupies \
                 is approximate",
                obstacle.id
            ));
        }
        let footprint = band_footprint(obstacle.id, obstacle.mesh, lo, hi)?;
        region = join(&region, &footprint)?;
    }
    Ok(region)
}

/// The plan bounds of a region; `None` when it is empty.
pub(crate) fn region_bounds(region: &Plan) -> Option<Bounds2> {
    let mut points = region.polygons().iter().flat_map(|p| p.outer.points.iter());
    let first = points.next()?;
    let mut bounds = ([first.x, first.y], [first.x, first.y]);
    for point in points {
        bounds.0 = [bounds.0[0].min(point.x), bounds.0[1].min(point.y)];
        bounds.1 = [bounds.1[0].max(point.x), bounds.1[1].max(point.y)];
    }
    Some(bounds)
}

fn in_ring(ring: &Ring, point: Point2) -> bool {
    let mut inside = false;
    let count = ring.points.len();
    for index in 0..count {
        let a = ring.points[index];
        let b = ring.points[(index + 1) % count];
        if (a.y > point.y) != (b.y > point.y) {
            let crossing = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
            if point.x < crossing {
                inside = !inside;
            }
        }
    }
    inside
}

/// Whether `point` lies inside `region`, holes excluded.
pub(crate) fn contains(region: &Plan, point: Point2) -> bool {
    region.polygons().iter().any(|polygon| {
        in_ring(&polygon.outer, point) && !polygon.holes.iter().any(|hole| in_ring(hole, point))
    })
}

fn rings(region: &Plan) -> impl Iterator<Item = &Ring> {
    region
        .polygons()
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
}

fn edges(ring: &Ring) -> impl Iterator<Item = (Point2, Point2)> + '_ {
    let count = ring.points.len();
    (0..count).map(move |index| (ring.points[index], ring.points[(index + 1) % count]))
}

/// The parameter intervals of `origin + direction * t`, `t` in `[t0, t1]`,
/// that lie in `region`, merged and in order.
pub(crate) fn line_intervals(
    region: &Plan,
    origin: Point2,
    direction: Vec2,
    t0: f64,
    t1: f64,
) -> Vec<(f64, f64)> {
    let mut cuts = vec![t0, t1];
    for ring in rings(region) {
        for (a, b) in edges(ring) {
            let edge = b - a;
            let denominator = direction.perp_dot(edge);
            if denominator.abs() <= f64::EPSILON * direction.length() * edge.length() {
                // Parallel: only a collinear edge adds cuts, at its ends.
                if (a - origin).perp_dot(direction).abs() <= ON_SURFACE * direction.length() {
                    let scale = direction.length_squared();
                    cuts.push((a - origin).dot(direction) / scale);
                    cuts.push((b - origin).dot(direction) / scale);
                }
                continue;
            }
            let s = (a - origin).perp_dot(direction) / denominator;
            if (-ON_SURFACE..=1.0 + ON_SURFACE).contains(&s) {
                cuts.push((a - origin).perp_dot(edge) / denominator);
            }
        }
    }
    cuts.retain(|t| *t >= t0 && *t <= t1);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    for pair in cuts.windows(2) {
        let middle = origin + direction * f64::midpoint(pair[0], pair[1]);
        if !contains(region, middle) {
            continue;
        }
        match intervals.last_mut() {
            Some(last) if (last.1 - pair[0]).abs() <= 1e-12 => last.1 = pair[1],
            _ => intervals.push((pair[0], pair[1])),
        }
    }
    intervals
}

/// Merges interval lists into their union.
pub(crate) fn merge_intervals(mut intervals: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in intervals {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 1e-12 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// A polygon enclosing the disc of `radius` around `centre`.
fn circumscribed(centre: Point2, radius: f64) -> Option<Polygon> {
    let count = circumscribed_sides(radius);
    let sides = f64::from(count);
    let outer = radius / (std::f64::consts::PI / sides).cos();
    polygon(
        (0..count)
            .map(|index| {
                let angle = std::f64::consts::TAU * f64::from(index) / sides;
                centre + Vec2::new(angle.cos(), angle.sin()) * outer
            })
            .collect(),
    )
}

/// Whether a disc of `radius` swept along `path` stays inside `domain`.
///
/// The exact sweep is enclosed by rectangles along the segments and polygons
/// circumscribing the discs at the vertices, all grown by [`MARGIN`]; the
/// enclosure must leave nothing outside the domain. `true` is a proof,
/// `false` is only the absence of one.
pub(crate) fn sweep_inside(domain: &Plan, path: &[Point2], radius: f64) -> Result<bool, String> {
    if domain.is_empty() || path.is_empty() {
        return Ok(false);
    }
    let grown = radius + MARGIN;
    let mut parts: Vec<Polygon> = path
        .iter()
        .filter_map(|point| circumscribed(*point, grown))
        .collect();
    for pair in path.windows(2) {
        let along = pair[1] - pair[0];
        let length = along.length();
        if length <= ON_SURFACE {
            continue;
        }
        let side = along.perp() / length * grown;
        parts.extend(polygon(vec![
            pair[0] - side,
            pair[1] - side,
            pair[1] + side,
            pair[0] + side,
        ]));
    }
    let sweep = union(parts)?;
    Ok(subtract(&sweep, domain)?.is_empty())
}

/// A path along which a disc of `radius` provably stays inside `domain`.
///
/// The straight segment is tried first. Otherwise a visibility-graph
/// shortest path through the domain eroded by a little more than `radius` is
/// proposed and checked like the segment. The erosion lies inside the exact
/// one; it only proposes.
pub(crate) fn witness(
    domain: &Plan,
    from: Point2,
    to: Point2,
    radius: f64,
) -> Result<Option<Vec<Point2>>, String> {
    if sweep_inside(domain, &[from, to], radius)? {
        return Ok(Some(vec![from, to]));
    }
    let slack = 0.005 + 0.002 * radius;
    let eroded = erode(domain, radius + slack)?;
    if eroded.is_empty() {
        return Ok(None);
    }
    match axiolid_route::shortest_path_within(eroded.polygons(), &[], from, to, ROUTE_BUDGET) {
        Ok(Ok(route)) if sweep_inside(domain, &route.polyline, radius)? => Ok(Some(route.polyline)),
        _ => Ok(None),
    }
}

/// `plan` less every point within `radius` of its boundary: the plan less
/// rectangles along its edges and polygons circumscribing discs at its
/// vertices. The removed set encloses the exact one, so the result lies inside
/// the exact erosion; it only proposes paths, which are proven separately.
fn erode(plan: &Plan, radius: f64) -> Result<Plan, String> {
    let mut removed = Vec::new();
    for ring in rings(plan) {
        for (a, b) in edges(ring) {
            removed.extend(circumscribed(a, radius));
            let along = b - a;
            let length = along.length();
            if length <= ON_SURFACE {
                continue;
            }
            let side = along.perp() / length * radius;
            removed.extend(polygon(vec![a - side, b - side, b + side, a + side]));
        }
    }
    subtract(plan, &union(removed)?)
}

/// A point inside a region's largest polygon: its outer ring's centroid.
pub(crate) fn centroid(region: &Plan) -> Option<Point2> {
    let largest = region.polygons().iter().max_by(|a, b| {
        ring_area(&a.outer)
            .abs()
            .total_cmp(&ring_area(&b.outer).abs())
    })?;
    let points = &largest.outer.points;
    let area = ring_area(&largest.outer);
    if area.abs() <= ON_SURFACE {
        return None;
    }
    let mut sum = Vec2::ZERO;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        sum += (a + b) * a.perp_dot(b);
    }
    Some(sum / (6.0 * area))
}

fn segment_distance(a: (Point2, Point2), b: (Point2, Point2)) -> f64 {
    let point_segment = |p: Point2, (s, e): (Point2, Point2)| {
        let along = e - s;
        let length = along.length_squared();
        let t = if length <= 0.0 {
            0.0
        } else {
            ((p - s).dot(along) / length).clamp(0.0, 1.0)
        };
        (p - (s + along * t)).length()
    };
    let cross = |p: Point2, q: Point2, r: Point2| (q - p).perp_dot(r - p);
    let (d1, d2) = (cross(a.0, a.1, b.0), cross(a.0, a.1, b.1));
    let (d3, d4) = (cross(b.0, b.1, a.0), cross(b.0, b.1, a.1));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0;
    }
    point_segment(a.0, b)
        .min(point_segment(a.1, b))
        .min(point_segment(b.0, a))
        .min(point_segment(b.1, a))
}

/// Whether two regions overlap or their boundaries touch.
pub(crate) fn touching(a: &Plan, b: &Plan) -> Result<bool, String> {
    let (Some(first), Some(second)) = (region_bounds(a), region_bounds(b)) else {
        return Ok(false);
    };
    if bounds_gap(&first, &second) > ON_SURFACE {
        return Ok(false);
    }
    if !intersect(a, b)?.is_empty() {
        return Ok(true);
    }
    for ring_a in rings(a) {
        for edge_a in edges(ring_a) {
            for ring_b in rings(b) {
                if edges(ring_b).any(|edge_b| segment_distance(edge_a, edge_b) <= ON_SURFACE) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Where a portal lies: the frame its corridor is measured in.
#[derive(Clone, Debug)]
pub(crate) struct PortalFrame {
    pub(crate) id: ObjectId,
    /// Unit plan normal through the portal's thickness.
    pub(crate) normal: Vec2,
    /// Unit plan direction along the wall.
    pub(crate) along: Vec2,
    pub(crate) centre: Point2,
    /// Half the portal's thickness.
    pub(crate) half: f64,
    /// Extent along the wall, relative to `centre`.
    pub(crate) u0: f64,
    pub(crate) u1: f64,
    pub(crate) z0: f64,
    pub(crate) z1: f64,
}

impl PortalFrame {
    pub(crate) fn point(&self, u: f64, v: f64) -> Point2 {
        self.centre + self.along * u + self.normal * v
    }

    /// The corridor between `v0` and `v1` through the thickness.
    pub(crate) fn band(&self, v0: f64, v1: f64) -> Option<Polygon> {
        polygon(vec![
            self.point(self.u0, v0),
            self.point(self.u1, v0),
            self.point(self.u1, v1),
            self.point(self.u0, v1),
        ])
    }
}

/// Host-declared facts about portals a mesh cannot show.
#[derive(Clone, Debug, Default)]
pub(crate) struct PortalFacts {
    voids: BTreeMap<ObjectId, Result<(TriMesh, bool), String>>,
    clear_widths: BTreeMap<ObjectId, f64>,
}

/// What bounds a portal's clear width from below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Clearance {
    /// A bodiless opening: its corridor is its clear passage.
    Void,
    /// A body whose clear width the host or the request stated.
    Stated(f64),
    /// A body with a leaf and lining nobody measured.
    Unstated,
}

impl Clearance {
    /// Whether a body of `width` passes the leaf and lining.
    pub(crate) fn admits(self, width: f64) -> bool {
        if width <= 0.0 {
            return true;
        }
        match self {
            Self::Void => true,
            Self::Stated(stated) => stated >= width,
            Self::Unstated => false,
        }
    }

    /// Adds a clear width a request states: the narrower statement wins,
    /// and an opening's void stays its clear passage.
    pub(crate) fn with_stated(self, stated: Option<f64>) -> Self {
        match (self, stated) {
            (Self::Stated(own), Some(stated)) => Self::Stated(own.min(stated)),
            (Self::Unstated, Some(stated)) => Self::Stated(stated),
            (clearance, _) => clearance,
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            Self::Void => "void".into(),
            Self::Stated(width) => format!("stated={width:.6}"),
            Self::Unstated => "unstated".into(),
        }
    }
}

impl PortalFacts {
    pub(crate) fn with_void(mut self, portal: ObjectId, mesh: TriMesh, exact: bool) -> Self {
        self.voids.insert(portal, Ok((mesh, exact)));
        self
    }

    pub(crate) fn with_unmeasured_void(mut self, portal: ObjectId, reason: String) -> Self {
        self.voids.insert(portal, Err(reason));
        self
    }

    pub(crate) fn with_clear_width(mut self, portal: ObjectId, metres: f64) -> Self {
        self.clear_widths.insert(portal, metres);
        self
    }

    pub(crate) fn clearance(&self, portal: &ObjectId) -> Result<Clearance, String> {
        if self.voids.contains_key(portal) {
            return Ok(Clearance::Void);
        }
        match self.clear_widths.get(portal) {
            Some(width) if width.is_finite() && *width > 0.0 => Ok(Clearance::Stated(*width)),
            Some(_) => Err(format!("the stated clear width of {portal} is invalid")),
            None => Ok(Clearance::Unstated),
        }
    }

    /// A portal's frame, from its void or its body.
    pub(crate) fn frame(
        &self,
        geometry: &AxiolidGeometry,
        portal: &ObjectId,
    ) -> Result<PortalFrame, String> {
        let (mesh, exact) = match self.voids.get(portal) {
            Some(Ok((mesh, exact))) => (mesh, *exact),
            Some(Err(reason)) => {
                return Err(format!(
                    "the void of portal {portal} was not measured: {reason}"
                ));
            }
            None => (
                body(geometry, portal, "portal")?,
                !geometry.is_tessellated(portal),
            ),
        };
        if !exact {
            return Err(format!(
                "portal {portal} is a tessellation, so its faces are approximate"
            ));
        }
        let (min, max) =
            mesh_extent(mesh).ok_or_else(|| format!("portal {portal} has an empty mesh"))?;
        let plan: Vec<Point2> = (0..mesh.position_count())
            .map(|index| {
                let position = mesh.position(index);
                Point2::new(position.x, position.y)
            })
            .collect();
        let axis = thin_axis(&plan).ok_or_else(|| {
            format!("portal {portal} has no single direction through its thickness")
        })?;
        let along = axis.normal.perp();
        let hull = convex_hull(&plan);
        let (low, high) = span(&hull, along);
        let offset = axis.centre.dot(along);
        Ok(PortalFrame {
            id: portal.clone(),
            normal: axis.normal,
            along,
            centre: axis.centre,
            half: axis.thickness * 0.5,
            u0: low - offset,
            u1: high - offset,
            z0: min[2],
            z1: max[2],
        })
    }
}

/// A portal side: the surface a probe first enters, and how far past the
/// face it begins.
#[derive(Clone, Debug)]
pub(crate) struct Side {
    pub(crate) floor: usize,
    pub(crate) gap: f64,
}

/// How far along `direction` from `origin` the footprint begins, within
/// `reach`.
pub(crate) fn entry(footprint: &Plan, origin: Point2, direction: Vec2, reach: f64) -> Option<f64> {
    line_intervals(footprint, origin, direction, 0.0, reach)
        .first()
        .map(|interval| interval.0)
}

/// The surfaces on each side of a portal (index 0 the `-` side, 1 the `+`).
///
/// A surface qualifies when its height range overlaps the portal's. One
/// holding both faces encloses the portal and is on neither side; two
/// entered at the same distance leave the side undecided and refuse.
pub(crate) fn sides(frame: &PortalFrame, floors: &[Floor]) -> Result<[Option<Side>; 2], String> {
    let mut found: [Option<Side>; 2] = [None, None];
    let faces = [-1.0, 1.0].map(|sign| (sign, frame.point(0.0, sign * frame.half)));
    let reach_bounds: Bounds2 = {
        let a = frame.point(frame.u0, -frame.half - REACH);
        let b = frame.point(frame.u1, frame.half + REACH);
        ([a.x.min(b.x), a.y.min(b.y)], [a.x.max(b.x), a.y.max(b.y)])
    };
    for (slot, (sign, face)) in faces.iter().enumerate() {
        let mut best: Option<(f64, usize)> = None;
        for (index, floor) in floors.iter().enumerate() {
            if floor.z0 >= frame.z1 || floor.top <= frame.z0 {
                continue;
            }
            if bounds_gap(&floor.bounds, &reach_bounds) > REACH {
                continue;
            }
            let entries = faces.map(|(s, f)| entry(&floor.footprint, f, frame.normal * s, REACH));
            if entries.iter().all(|e| e.is_some_and(|t| t <= ON_SURFACE)) {
                continue;
            }
            let Some(gap) = entry(&floor.footprint, *face, frame.normal * *sign, REACH) else {
                continue;
            };
            match best {
                Some((least, other)) if (gap - least).abs() <= ON_SURFACE => {
                    return Err(format!(
                        "portal {} opens onto {} and {} at the same distance",
                        frame.id, floors[other].id, floor.id
                    ));
                }
                Some((least, _)) if least < gap => {}
                _ => best = Some((gap, index)),
            }
        }
        found[slot] = best.map(|(gap, floor)| Side { floor, gap });
    }
    Ok(found)
}

/// The obstruction band above a floor at `z0` with top `top`.
///
/// A stated band is measured from the floor; without one the band is the
/// object's own height, so a door's lintel never blocks the door.
pub(crate) fn band_limits(
    band: Option<axioval_engine::LengthInterval>,
    z0: f64,
    top: f64,
) -> (f64, f64) {
    band.map_or((z0, top), |band| {
        (z0 + band.lower_metres(), z0 + band.upper_metres())
    })
}

/// A portal's corridor through its thickness, extended to each side's
/// surface (just past it, so the two overlap), minus the obstruction.
pub(crate) fn corridor(
    frame: &PortalFrame,
    sides: &[Option<Side>; 2],
    obstruction: &Plan,
) -> Result<Plan, String> {
    let reach = |side: &Option<Side>| side.as_ref().map_or(0.0, |side| side.gap + 1e-5);
    let Some(shape) = frame.band(
        -frame.half - reach(&sides[0]),
        frame.half + reach(&sides[1]),
    ) else {
        return Ok(Plan::empty());
    };
    subtract(&union(vec![shape])?, obstruction)
}

/// The free intervals of a portal's mid-line (in `u`) that overlap the
/// portal's own extent, and the gaps around each (how far the line stays
/// outside the pieces beyond either end).
///
/// A body crossing the mid-line inside one of these intervals covers a chord
/// of its own diameter inside it, so the longest bounds every body crossing
/// the portal.
pub(crate) fn mid_line(frame: &PortalFrame, pieces: &[&Plan]) -> Vec<(f64, f64, f64, f64)> {
    let far = 1e6;
    let intervals = merge_intervals(
        pieces
            .iter()
            .flat_map(|piece| line_intervals(piece, frame.centre, frame.along, -far, far))
            .collect(),
    );
    (0..intervals.len())
        .filter(|index| {
            let (a, b) = intervals[*index];
            b > frame.u0 + ON_SURFACE && a < frame.u1 - ON_SURFACE
        })
        .map(|index| {
            let (a, b) = intervals[index];
            let before = index
                .checked_sub(1)
                .map_or(far, |previous| a - intervals[previous].1);
            let after = intervals.get(index + 1).map_or(far, |next| next.0 - b);
            (a, b, before, after)
        })
        .collect()
}

/// The chord bound on a body crossing the portal, and the free stretch of
/// the portal's own extent where the widest crossing interval lies.
pub(crate) fn mid_line_width(frame: &PortalFrame, pieces: &[&Plan]) -> (f64, Option<(f64, f64)>) {
    let best = mid_line(frame, pieces)
        .into_iter()
        .max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)));
    best.map_or((0.0, None), |(a, b, _, _)| {
        (
            b - a + 2.0 * MARGIN,
            Some((a.max(frame.u0), b.min(frame.u1))),
        )
    })
}

/// Cuts across every free mid-line interval of a portal no body of the
/// given width can cross, each reaching a little past the interval's ends
/// into the gap beyond, so a path along a boundary cannot slip round its
/// ends. A cut only blocks crossings the chord bound already rules out.
pub(crate) fn mid_line_barriers(frame: &PortalFrame, pieces: &[&Plan]) -> Vec<Vec<Point2>> {
    mid_line(frame, pieces)
        .into_iter()
        .map(|(a, b, before, after)| {
            let start = a - (before / 3.0).min(MARGIN);
            let end = b + (after / 3.0).min(MARGIN);
            vec![frame.point(start, 0.0), frame.point(end, 0.0)]
        })
        .collect()
}

/// Where a body of `radius` stands in front of a portal on side `sign`, at
/// `u` along it: past the side's surface boundary by the radius and a
/// landing allowance. `None` when the surface does not continue there.
pub(crate) fn landing(
    frame: &PortalFrame,
    floor: &Floor,
    u: f64,
    sign: f64,
    radius: f64,
) -> Option<Point2> {
    let face = frame.point(u, sign * frame.half);
    let gap = entry(&floor.footprint, face, frame.normal * sign, REACH)?;
    Some(frame.point(u, sign * (frame.half + gap + radius + LANDING)))
}

/// Whether segment `a`–`b` crosses the portal's mid-line within its extent.
pub(crate) fn crosses_mid_line(frame: &PortalFrame, a: Point2, b: Point2) -> bool {
    let (p, q) = (frame.point(frame.u0, 0.0), frame.point(frame.u1, 0.0));
    let side = |x: Point2| (q - p).perp_dot(x - p);
    let other = |x: Point2| (b - a).perp_dot(x - a);
    side(a) * side(b) <= 0.0 && other(p) * other(q) <= 0.0 && (side(a) != 0.0 || side(b) != 0.0)
}

fn root(group: &mut [usize], mut index: usize) -> usize {
    while group[index] != index {
        group[index] = group[group[index]];
        index = group[index];
    }
    index
}

/// Whether `from` and `to` lie in different pieces of `domain` once `cuts`
/// are taken out, pieces that touch counting as one.
///
/// `false` whenever either point lies in no piece: separation is only
/// claimed between two points that are both placed.
pub(crate) fn separated(
    domain: &Plan,
    cuts: Vec<Polygon>,
    from: Point2,
    to: Point2,
) -> Result<bool, String> {
    let rest = subtract(domain, &union(cuts)?)?;
    let pieces: Vec<Plan> = rest
        .polygons()
        .iter()
        .map(|piece| Ok::<_, String>(Plan::piece(piece.clone())))
        .collect::<Result<_, String>>()?;
    let holder = |point: Point2| pieces.iter().position(|piece| contains(piece, point));
    let (Some(start), Some(goal)) = (holder(from), holder(to)) else {
        return Ok(false);
    };
    let mut group: Vec<usize> = (0..pieces.len()).collect();
    for a in 0..pieces.len() {
        for b in a + 1..pieces.len() {
            if touching(&pieces[a], &pieces[b])? {
                let (first, second) = (root(&mut group, a), root(&mut group, b));
                group[first] = second;
            }
        }
    }
    Ok(root(&mut group, start) != root(&mut group, goal))
}
