//! Shelf running-length measurement: a layout of parallel shelf bands on a
//! space's footprint.
//!
//! ADR 0004: this module **measures**. It reports how many running metres of
//! shelving the layout below places in a space, for the arrangement the
//! caller describes. Whether that clears a required minimum is the
//! capability's decision, not this module's.
//!
//! # The layout
//!
//! The usable floor is the space's footprint less a clearance around every
//! door and opening the request names: the points within `c + g` of the
//! door's plan footprint, with `c` the arrangement's door clearance and `g`
//! the plan gap between the door and the space (a leaf set into a thick
//! wall stands back from the room). The swing is unknown, so the clearance
//! reaches that far in every direction.
//!
//! Bands `d` deep run parallel to one axis `u`, arranged across the other
//! axis `v` as the stacks of a store room are: a band against the wall, an
//! aisle `a` wide, two bands back to back, an aisle, and so on. Each band is
//! served by the aisle on one side of it, and carries shelving at `u` only
//! where its whole cross-section lies on the usable floor and its aisle's
//! whole cross-section lies on the footprint (an aisle may cross a door's
//! clearance; a shelf may not). The length is summed over the bands and
//! multiplied by the tiers: whole multiples of the vertical spacing between
//! the bottom elevation and the lower of the top elevation and the space's
//! clear height.
//!
//! The axes are tried along the least-area rectangle of the footprint and
//! along the least-area rectangle of every door's footprint (the rectangle
//! the plan-span service answers), each axis as `u` and the pattern anchored
//! at either wall; the measured length is the longest of these layouts. An
//! axis only chooses the layout: its interval holds on the true geometry
//! whichever axis it runs along, so a tied or unproven orientation still
//! proposes its axes.
//!
//! # Soundness
//!
//! A band's shelving is found exactly per band: between consecutive
//! breakpoints (a boundary vertex inside the band's strip, or a boundary
//! edge crossing one of its long sides) the boundary edges meeting a
//! cross-section do not change, so one cross-section decides the whole
//! interval. The lower bound tests cross-sections grown by `ε` and shrinks
//! each run by `ε` at both ends; the upper bound tests them shrunk by `ε`
//! and grows each run. `ε` covers the overlay's grid rounding and the
//! chord deviation of a tessellated space; the door clearances are grown
//! with a circumscribed polygon for the lower bound and an inscribed one for
//! the upper. So the interval holds the layout's length on the true
//! geometry, and a compliant space has a positive lower bound.

use std::collections::BTreeMap;

use axiolid_core::{Point2, Tolerance, Vec2};
use axiolid_mesh::{TriMesh, TriangleMeshView};
use axiolid_overlay::{FillRule, OverlayInput, OverlayOperation, Polygon, Ring, overlay};
use axioval_engine::{
    GeometryFidelity, LinearInterval, LinearQuantityError, LinearQuantityEvidence,
    LinearQuantityKind, LinearQuantityRequest, LinearQuantityService, ShelfGeometry,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::plan_span::least_area_rectangle;
use crate::planar::{Disc, footprint_polygons, grown_polygons, plan_frame, projected_polygons};

/// Linear tolerance handed to the overlay.
const LINEAR_TOLERANCE: f64 = 1e-9;

/// The margin a cross-section keeps from the measured boundary: above the
/// overlay's grid rounding (about `1.5e-8` of the extent, axiolid/kernel#173)
/// for any building-sized plan.
const MARGIN: f64 = 1e-6;

/// How much of a band's or an aisle's depth may touch the boundary: its
/// cross-section counts as on the floor when all but this much at each end
/// is. A band is laid against a wall, so without it a cross-section would
/// start on the boundary, and a wall rounded a hair off the layout's axis
/// would leave it one point long.
const CONSTRUCTION_TOLERANCE: f64 = 1e-3;

/// The farthest a door may stand from its space in plan and still have its
/// clearance placed: more than a thick wall away, the door is not on this
/// space's boundary and its clearance would be a guess.
const MAXIMUM_DOOR_GAP: f64 = 1.0;

/// Two axes closer than this (the sine of the angle between them) are one
/// orientation.
const SAME_AXIS: f64 = 1e-9;

/// A shape the host supplied for an opening that has no material body.
#[derive(Clone, Debug)]
enum Void {
    Mesh(TriMesh, f64),
    Unmeasured,
}

/// Measures shelf capacity from application-supplied meshes.
pub struct AxiolidLinearQuantityService {
    geometry: AxiolidGeometry,
    voids: BTreeMap<ObjectId, Void>,
}

impl AxiolidLinearQuantityService {
    /// Binds a geometry set to the source that identifies its objects.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self {
            geometry,
            voids: BTreeMap::new(),
        }
    }

    /// The exact planar void of an opening the geometry declares bodiless,
    /// so its clearance can be placed.
    #[must_use]
    pub fn with_opening_void(mut self, opening: ObjectId, mesh: TriMesh) -> Self {
        self.voids.insert(opening, Void::Mesh(mesh, 0.0));
        self
    }

    /// The void of an opening as a tessellation within
    /// `chord_deviation_metres` of the true shape.
    #[must_use]
    pub fn with_tessellated_opening_void(
        mut self,
        opening: ObjectId,
        mesh: TriMesh,
        chord_deviation_metres: f64,
    ) -> Self {
        self.voids
            .insert(opening, Void::Mesh(mesh, chord_deviation_metres));
        self
    }

    /// An opening whose void could not be meshed: a space it opens into is
    /// refused rather than measured without its clearance.
    #[must_use]
    pub fn with_unmeasured_opening_void(mut self, opening: ObjectId) -> Self {
        self.voids.insert(opening, Void::Unmeasured);
        self
    }

    /// A door's plan triangles and chord deviation.
    fn door(&self, door: &ObjectId) -> Result<(Vec<Triangle>, f64), LinearQuantityError> {
        if self.geometry.is_unmeasured(door) {
            return Err(LinearQuantityError::Unavailable);
        }
        if let Some(mesh) = self.geometry.mesh(door) {
            let deviation = deviation(self.geometry.fidelity(door))?;
            return Ok((triangles(mesh), deviation));
        }
        match self.voids.get(door) {
            Some(Void::Mesh(mesh, deviation)) if deviation.is_finite() && *deviation >= 0.0 => {
                Ok((triangles(mesh), *deviation))
            }
            Some(Void::Mesh(..)) => Err(LinearQuantityError::InvalidGeometry),
            Some(Void::Unmeasured) | None => Err(LinearQuantityError::Unavailable),
        }
    }
}

fn deviation(
    fidelity: Result<GeometryFidelity, axioval_engine::ProximityError>,
) -> Result<f64, LinearQuantityError> {
    fidelity
        .map(|fidelity| fidelity.deviation_metres())
        .map_err(|_| LinearQuantityError::InvalidGeometry)
}

fn tolerance() -> Result<Tolerance, LinearQuantityError> {
    Tolerance::new(LINEAR_TOLERANCE, 1e-9).map_err(|_| LinearQuantityError::Unavailable)
}

/// `first` less `second`, both filled as their non-zero unions.
fn difference(
    first: Vec<Polygon>,
    second: Vec<Polygon>,
    tolerance: Tolerance,
) -> Result<Vec<Polygon>, LinearQuantityError> {
    if first.is_empty() || second.is_empty() {
        return Ok(first);
    }
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
        OverlayOperation::Difference,
        FillRule::NonZero,
        tolerance,
    )
    .map(|result| result.polygons)
    .map_err(|_| LinearQuantityError::Unavailable)
}

/// A plan point.
type Point = (f64, f64);

/// A boundary edge.
type Edge = (Point, Point);

fn rings(polygons: &[Polygon]) -> impl Iterator<Item = &Ring> {
    polygons
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
}

fn ring_edges(ring: &Ring) -> impl Iterator<Item = Edge> + '_ {
    let points = &ring.points;
    (0..points.len()).map(move |index| {
        let (a, b) = (points[index], points[(index + 1) % points.len()]);
        ((a.x, a.y), (b.x, b.y))
    })
}

fn point_segment(point: Point, (a, b): Edge) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point.0 - a.0 - t * dx).hypot(point.1 - a.1 - t * dy)
}

fn cross(o: Point, a: Point, b: Point) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// The distance between two segments: zero when they cross.
fn segment_segment(first: Edge, second: Edge) -> f64 {
    let (a, b) = first;
    let (c, d) = second;
    let crosses = cross(a, b, c).signum() * cross(a, b, d).signum() < 0.0
        && cross(c, d, a).signum() * cross(c, d, b).signum() < 0.0;
    if crosses {
        return 0.0;
    }
    point_segment(a, second)
        .min(point_segment(b, second))
        .min(point_segment(c, first))
        .min(point_segment(d, first))
}

/// Whether a point lies inside polygons (even-odd over every ring, which is
/// their union for the disjoint polygons an overlay returns).
fn inside(polygons: &[Polygon], (x, y): Point) -> bool {
    let mut inside = false;
    for ring in rings(polygons) {
        for (a, b) in ring_edges(ring) {
            if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                inside = !inside;
            }
        }
    }
    inside
}

/// The plan gap between a door's triangles and a footprint: zero when they
/// touch or overlap.
fn gap(door: &[Polygon], footprint: &[Polygon]) -> f64 {
    let edges: Vec<Edge> = rings(footprint).flat_map(ring_edges).collect();
    let mut nearest = f64::INFINITY;
    for triangle in door {
        for point in &triangle.outer.points {
            if inside(footprint, (point.x, point.y)) {
                return 0.0;
            }
        }
        for side in ring_edges(&triangle.outer) {
            for edge in &edges {
                nearest = nearest.min(segment_segment(side, *edge));
            }
        }
    }
    nearest
}

/// The first axis of a point set's least-area rectangle (the one
/// `measure_rectangle` answers), if it has one.
///
/// The axis only proposes a direction to lay bands along: every layout's
/// interval is sound on its own, so a tied or unproven orientation still
/// proposes one, and the longest layout counts.
fn rectangle_axis(points: &[Point2]) -> Option<Vec2> {
    let [x, y] = least_area_rectangle(points, 0.0).ok()?.axes[0];
    (x.is_finite() && y.is_finite() && x.hypot(y) > 0.5).then(|| Vec2::new(x, y))
}

/// Sorted, disjoint intervals of `u`.
type Runs = Vec<(f64, f64)>;

/// Where the cross-section `{u} × [v0, v1]` lies inside the region bounded
/// by `edges`: exact on the measured boundary.
///
/// Between consecutive breakpoints no boundary vertex lies in the strip and
/// no edge crosses its long sides, so the edges meeting a cross-section are
/// the same throughout and one cross-section decides the interval.
fn inside_runs(edges: &[Edge], polygons_inside: impl Fn(Point) -> bool, v0: f64, v1: f64) -> Runs {
    let mut breaks = Vec::new();
    for &(p, q) in edges {
        if p.1.max(q.1) < v0 || p.1.min(q.1) > v1 {
            continue;
        }
        for point in [p, q] {
            if (v0..=v1).contains(&point.1) {
                breaks.push(point.0);
            }
        }
        for line in [v0, v1] {
            if (p.1 - line) * (q.1 - line) < 0.0 {
                breaks.push(p.0 + (line - p.1) * (q.0 - p.0) / (q.1 - p.1));
            }
        }
    }
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let middle = 0.5 * (v0 + v1);
    let mut runs: Runs = Vec::new();
    for pair in breaks.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if b <= a {
            continue;
        }
        let u = 0.5 * (a + b);
        let met = edges.iter().any(|&(p, q)| {
            if (p.0 - u) * (q.0 - u) > 0.0 || (p.0 - q.0).abs() == 0.0 {
                return false;
            }
            let v = p.1 + (u - p.0) * (q.1 - p.1) / (q.0 - p.0);
            (v0..=v1).contains(&v)
        });
        if met || !polygons_inside((u, middle)) {
            continue;
        }
        match runs.last_mut() {
            #[allow(clippy::float_cmp)]
            Some(last) if last.1 == a => last.1 = b,
            _ => runs.push((a, b)),
        }
    }
    runs
}

/// Runs surely inside: cross-sections grown by `margin`, runs shrunk by it.
fn sure_runs(region: &Plan, v0: f64, v1: f64, margin: f64) -> Runs {
    inside_runs(
        &region.edges,
        |point| region.contains(point),
        v0 - margin,
        v1 + margin,
    )
    .into_iter()
    .filter_map(|(a, b)| (b - a > 2.0 * margin).then_some((a + margin, b - margin)))
    .collect()
}

/// Runs possibly inside: cross-sections shrunk by `margin`, runs grown by it.
fn possible_runs(region: &Plan, v0: f64, v1: f64, margin: f64) -> Runs {
    let (low, high) = if v1 - v0 > 2.0 * margin {
        (v0 + margin, v1 - margin)
    } else {
        let middle = 0.5 * (v0 + v1);
        (middle, middle)
    };
    let mut runs: Runs = Vec::new();
    for (a, b) in inside_runs(&region.edges, |point| region.contains(point), low, high) {
        let (a, b) = (a - margin, b + margin);
        match runs.last_mut() {
            Some(last) if last.1 >= a => last.1 = last.1.max(b),
            _ => runs.push((a, b)),
        }
    }
    runs
}

/// Where both sets of runs hold.
fn intersect(first: &Runs, second: &Runs) -> Runs {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < first.len() && j < second.len() {
        let low = first[i].0.max(second[j].0);
        let high = first[i].1.min(second[j].1);
        if high > low {
            out.push((low, high));
        }
        if first[i].1 < second[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn total(runs: &Runs) -> f64 {
    runs.iter().map(|(a, b)| b - a).sum()
}

/// A region in one layout's `(u, v)` frame: its boundary edges and rings.
struct Plan {
    edges: Vec<Edge>,
    polygons: Vec<Polygon>,
}

impl Plan {
    fn of(polygons: &[Polygon], u: Vec2) -> Self {
        let v = Vec2::new(-u.y, u.x);
        let turn = |point: &Point2| {
            Point2::new(point.x * u.x + point.y * u.y, point.x * v.x + point.y * v.y)
        };
        let polygons: Vec<Polygon> = polygons
            .iter()
            .map(|polygon| Polygon {
                outer: Ring {
                    points: polygon.outer.points.iter().map(turn).collect(),
                },
                holes: polygon
                    .holes
                    .iter()
                    .map(|hole| Ring {
                        points: hole.points.iter().map(turn).collect(),
                    })
                    .collect(),
            })
            .collect();
        let edges = rings(&polygons).flat_map(ring_edges).collect();
        Self { edges, polygons }
    }

    fn contains(&self, point: Point) -> bool {
        inside(&self.polygons, point)
    }

    fn span(&self) -> Option<(f64, f64)> {
        let mut values = self.edges.iter().map(|(p, _)| p.1);
        let first = values.next()?;
        Some(values.fold((first, first), |(low, high), v| (low.min(v), high.max(v))))
    }
}

/// The footprint, the usable floor surely free of door clearances, and the
/// usable floor possibly free of them, all in one layout's frame.
struct Floor {
    footprint: Plan,
    sure: Plan,
    possible: Plan,
}

/// The band length of one layout: `(lower, upper)` metres per tier.
fn layout(floor: &Floor, shelf: ShelfGeometry, from_low_wall: bool, margin: f64) -> (f64, f64) {
    let Some((low, high)) = floor.footprint.span() else {
        return (0.0, 0.0);
    };
    let (depth, aisle) = (shelf.depth_metres(), shelf.horizontal_spacing_metres());
    let pitch = 2.0 * depth + aisle;
    let (mut lower, mut upper) = (0.0, 0.0);
    // The anchoring wall is measured too, so a cross-section may sit up to
    // the margin off its true place: the cross-sections keep twice the
    // margin.
    let reach = 2.0 * margin;
    let inset =
        |(from, to): (f64, f64)| (from + CONSTRUCTION_TOLERANCE, to - CONSTRUCTION_TOLERANCE);
    let mut band = |band: (f64, f64), served_by: (f64, f64)| {
        let (band, served_by) = (inset(band), inset(served_by));
        let sure = intersect(
            &sure_runs(&floor.sure, band.0, band.1, reach),
            &sure_runs(&floor.footprint, served_by.0, served_by.1, reach),
        );
        let possible = intersect(
            &possible_runs(&floor.possible, band.0, band.1, reach),
            &possible_runs(&floor.footprint, served_by.0, served_by.1, reach),
        );
        lower += total(&sure);
        upper += total(&possible);
    };
    let mut index = 0.0_f64;
    loop {
        // The aisle's side towards the anchoring wall, then its far side.
        let near = depth + index * pitch;
        if near > high - low {
            break;
        }
        let far = near + aisle;
        let (aisle_low, aisle_high) = if from_low_wall {
            (low + near, low + far)
        } else {
            (high - far, high - near)
        };
        band((aisle_low - depth, aisle_low), (aisle_low, aisle_high));
        band((aisle_high, aisle_high + depth), (aisle_low, aisle_high));
        index += 1.0;
    }
    (lower, upper)
}

/// Whole tiers between `bottom` and the lower of `top` and `height`.
fn tiers(shelf: ShelfGeometry, height: f64) -> f64 {
    let stack = shelf.top_elevation_metres().min(height) - shelf.bottom_elevation_metres();
    if stack < 0.0 {
        return 0.0;
    }
    let spacing = shelf.vertical_spacing_metres();
    let mut count = (stack / spacing).floor().max(0.0);
    // The quotient is rounded; settle it on the products, which is what
    // "fits" means.
    while count > 0.0 && count * spacing > stack {
        count -= 1.0;
    }
    while (count + 1.0) * spacing <= stack {
        count += 1.0;
    }
    count
}

fn z_span(mesh: &TriMesh) -> Option<(f64, f64)> {
    let mut positions = (0..mesh.position_count()).map(|index| mesh.position(index).z);
    let first = positions.next()?;
    Some(positions.fold((first, first), |(low, high), z| (low.min(z), high.max(z))))
}

impl AxiolidLinearQuantityService {
    fn measure(
        &self,
        request: &LinearQuantityRequest,
        arrangement: ShelfGeometry,
    ) -> Result<(f64, f64, LinearInterval), LinearQuantityError> {
        let scope = request.scope();
        if self.geometry.is_unmeasured(scope) {
            return Err(LinearQuantityError::Unavailable);
        }
        let mesh = self
            .geometry
            .mesh(scope)
            .ok_or(LinearQuantityError::Unavailable)?;
        let room_deviation = deviation(self.geometry.fidelity(scope))?;
        let tolerance = tolerance()?;
        let room = triangles(mesh);
        let footprint =
            footprint_polygons(&room, tolerance).ok_or(LinearQuantityError::Unavailable)?;
        if footprint.is_empty() {
            return Err(LinearQuantityError::Unavailable);
        }
        let (floor, ceiling) = z_span(mesh).ok_or(LinearQuantityError::Unavailable)?;
        let height = LinearInterval::try_new(
            (ceiling - floor - 2.0 * room_deviation).max(0.0),
            ceiling - floor + 2.0 * room_deviation,
        )?;

        // Clearances: grown more than the true one for the sure floor, less
        // for the possible floor.
        let clearance = arrangement.door_clearance_metres();
        let mut outer_zones = Vec::new();
        let mut inner_zones = Vec::new();
        let mut axes: Vec<Vec2> = Vec::new();
        let points: Vec<Point2> = rings(&footprint)
            .flat_map(|ring| ring.points.clone())
            .collect();
        axes.extend(rectangle_axis(&points));
        for door in request.doors() {
            let (shape, door_deviation) = self.door(door)?;
            let plan = projected_polygons(&shape);
            if plan.is_empty() {
                return Err(LinearQuantityError::Unavailable);
            }
            let gap = gap(&plan, &footprint);
            if gap > MAXIMUM_DOOR_GAP {
                return Err(LinearQuantityError::Unavailable);
            }
            let slack = door_deviation + room_deviation + MARGIN;
            outer_zones.extend(grown_polygons(
                &shape,
                clearance + gap + slack,
                Disc::Circumscribed,
            ));
            inner_zones.extend(grown_polygons(
                &shape,
                (clearance + gap - slack).max(0.0),
                Disc::Inscribed,
            ));
            let door_points: Vec<Point2> = plan
                .iter()
                .flat_map(|polygon| polygon.outer.points.clone())
                .collect();
            axes.extend(rectangle_axis(&door_points));
        }
        let sure = difference(footprint.clone(), outer_zones, tolerance)?;
        let possible = difference(footprint.clone(), inner_zones, tolerance)?;

        let mut families: Vec<Vec2> = Vec::new();
        for axis in axes {
            let known = families.iter().any(|seen| {
                (seen.x * axis.y - seen.y * axis.x).abs() < SAME_AXIS
                    || (seen.x * axis.x + seen.y * axis.y).abs() < SAME_AXIS
            });
            if !known {
                families.push(axis);
            }
        }
        let margin = MARGIN + room_deviation;
        let (mut lower, mut upper) = (0.0_f64, 0.0_f64);
        for axis in families {
            for u in [axis, Vec2::new(-axis.y, axis.x)] {
                let floor = Floor {
                    footprint: Plan::of(&footprint, u),
                    sure: Plan::of(&sure, u),
                    possible: Plan::of(&possible, u),
                };
                for from_low_wall in [true, false] {
                    let (low, high) = layout(&floor, arrangement, from_low_wall, margin);
                    lower = lower.max(low);
                    upper = upper.max(high);
                }
            }
        }
        let low_tiers = tiers(arrangement, height.lower_metres());
        let high_tiers = tiers(arrangement, height.upper_metres());
        Ok((lower * low_tiers, upper * high_tiers, height))
    }
}

impl LinearQuantityService for AxiolidLinearQuantityService {
    fn measure_linear_quantity(
        &self,
        request: &LinearQuantityRequest,
    ) -> Result<LinearQuantityEvidence, LinearQuantityError> {
        // The kind enum is non-exhaustive: a kind added upstream must fail
        // closed here rather than being silently measured as shelving.
        let LinearQuantityKind::ShelfRunningLength(arrangement) = request.kind() else {
            return Err(LinearQuantityError::Unavailable);
        };
        let (lower, upper, height) = self.measure(request, arrangement)?;
        let measured = LinearInterval::try_new(lower, upper)?;
        let scope = request.scope();
        let doors: Vec<String> = request.doors().iter().map(ToString::to_string).collect();
        Ok(LinearQuantityEvidence::try_new(
            request.clone(),
            measured,
            Evidence::exact(
                scope.source.clone(),
                format!("axiolid:shelf-layout:{scope}:doors=[{}]", doors.join(",")),
            ),
        )?
        .with_clear_height(height))
    }
}

#[cfg(test)]
mod tests {
    use super::{Edge, Runs, inside_runs, intersect, tiers};
    use axioval_engine::ShelfGeometry;

    fn square(size: f64) -> Vec<Edge> {
        let p = [(0.0, 0.0), (size, 0.0), (size, size), (0.0, size)];
        (0..4).map(|i| (p[i], p[(i + 1) % 4])).collect()
    }

    #[test]
    fn a_cross_section_inside_a_square_runs_its_width() {
        let edges = square(4.0);
        let inside = |(x, y): (f64, f64)| (0.0..4.0).contains(&x) && (0.0..4.0).contains(&y);
        assert_eq!(inside_runs(&edges, inside, 1.0, 2.0), vec![(0.0, 4.0)]);
        assert!(inside_runs(&edges, inside, 3.0, 5.0).is_empty());
    }

    #[test]
    fn a_notch_interrupts_a_run() {
        // A 4 x 4 square with a 1 x 1 notch cut from the middle of its top.
        let p = [
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 4.0),
            (2.5, 4.0),
            (2.5, 3.0),
            (1.5, 3.0),
            (1.5, 4.0),
            (0.0, 4.0),
        ];
        let edges: Vec<Edge> = (0..p.len()).map(|i| (p[i], p[(i + 1) % p.len()])).collect();
        let inside = |(x, y): (f64, f64)| {
            (0.0..4.0).contains(&x)
                && (0.0..4.0).contains(&y)
                && !((1.5..2.5).contains(&x) && y > 3.0)
        };
        assert_eq!(
            inside_runs(&edges, inside, 3.2, 3.8),
            vec![(0.0, 1.5), (2.5, 4.0)]
        );
        assert_eq!(
            inside_runs(&edges, inside, 2.0, 3.8),
            vec![(0.0, 1.5), (2.5, 4.0)]
        );
        assert_eq!(inside_runs(&edges, inside, 1.0, 2.5), vec![(0.0, 4.0)]);
    }

    #[test]
    fn runs_intersect() {
        let first: Runs = vec![(0.0, 2.0), (3.0, 5.0)];
        let second: Runs = vec![(1.0, 4.0)];
        assert_eq!(intersect(&first, &second), vec![(1.0, 2.0), (3.0, 4.0)]);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn tiers_are_whole_multiples_of_the_spacing() {
        let shelf = ShelfGeometry::try_new(0.3, 1.0, 0.4, 0.0, 2.0, 0.9).unwrap();
        assert_eq!(tiers(shelf, 3.0), 5.0);
        assert_eq!(tiers(shelf, 2.0), 5.0);
        assert_eq!(tiers(shelf, 1.99), 4.0);
        assert_eq!(tiers(shelf, 0.3), 0.0);
        let raised = ShelfGeometry::try_new(0.3, 1.0, 0.4, 0.5, 2.0, 0.9).unwrap();
        assert_eq!(tiers(raised, 0.4), 0.0);
    }
}
