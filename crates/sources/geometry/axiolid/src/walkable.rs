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
use crate::planar::{collinear, plan_frame, polygon_area, projected_polygons, ring_area};

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

/// The tolerance handed to the overlay, which validates its operands and
/// settles its output with it. The validator compares a cross product, an
/// area, with it, so a thin but valid sliver of a millimetre by a tenth of a
/// micrometre fails at [`ON_SURFACE`]. Operands here are fresh convex
/// polygons and the overlay's own settled output, so the check can be tight.
fn overlay_tolerance() -> Result<Tolerance, String> {
    Tolerance::new(1e-15, 1e-15).map_err(|_| "invalid tolerance".to_owned())
}

/// A plan bounding box `(min, max)`.
pub(crate) type Bounds2 = ([f64; 2], [f64; 2]);

/// A plan region: the polygons an overlay produced.
///
/// The overlay settles its output (axiolid-overlay 0.3.3), so every plan goes
/// back to it as an operand as it is, at the same tight tolerance it was
/// settled with.
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

    /// The plan moved by `offset`, back from a local frame.
    fn moved(self, offset: [f64; 2]) -> Self {
        if offset == [0.0, 0.0] {
            return self;
        }
        let shift = |ring: &Ring| Ring {
            points: ring
                .points
                .iter()
                .map(|p| Point2::new(p.x + offset[0], p.y + offset[1]))
                .collect(),
        };
        Self {
            polygons: self
                .polygons
                .iter()
                .map(|polygon| Polygon {
                    outer: shift(&polygon.outer),
                    holes: polygon.holes.iter().map(shift).collect(),
                })
                .collect(),
        }
    }

    /// One polygon of this plan as a plan of its own.
    pub(crate) fn piece(polygon: Polygon) -> Self {
        Self {
            polygons: vec![polygon],
        }
    }

    /// The polygons of an overlay region (settled output) as a plan.
    pub(crate) fn of(polygons: &[Polygon]) -> Self {
        Self {
            polygons: polygons.to_vec(),
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
        _ => overlay_soups(
            a.polygons.clone(),
            b.polygons.clone(),
            OverlayOperation::Union,
        ),
    }
}

pub(crate) fn subtract(a: &Plan, b: &Plan) -> Result<Plan, String> {
    if a.is_empty() || b.is_empty() {
        return Ok(a.clone());
    }
    overlay_soups(
        a.polygons.clone(),
        b.polygons.clone(),
        OverlayOperation::Difference,
    )
}

pub(crate) fn intersect(a: &Plan, b: &Plan) -> Result<Plan, String> {
    if a.is_empty() || b.is_empty() {
        return Ok(Plan::empty());
    }
    overlay_soups(
        a.polygons.clone(),
        b.polygons.clone(),
        OverlayOperation::Intersection,
    )
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

/// A clipped face projected to plan as the fan of triangles from its first
/// corner, each with the orientation its corners give it.
///
/// The fan's windings add up to the face's own at every point off their
/// edges: the diagonals the fan adds are walked once each way. So the
/// non-zero union of the fans is the non-zero union of the faces, while
/// every operand is a triangle, which the overlay never refuses as
/// self-intersecting. A projected face itself can be: the projection of a
/// (near-)vertical face clipped to the band is a quadrilateral or pentagon
/// whose corners lie on one line up to rounding, and the overlay takes its
/// edges, which double back along that line, for crossing ones.
/// Triangles without area are left out: those [`projected`] leaves out,
/// and those on one line up to the rounding of their coordinates
/// ([`collinear`]), as every plan shadow here is.
fn projected_fan(face: &[Point3]) -> impl Iterator<Item = Polygon> + '_ {
    (1..face.len().saturating_sub(1))
        .filter_map(move |index| projected(&[face[0], face[index], face[index + 1]]))
        .filter(|triangle| !collinear(&triangle.outer.points))
}

/// Units in the last place of a footprint's largest coordinate within which
/// [`band_footprint`] first takes two features for one.
const SNAP_ULPS: f64 = 8.0;

/// How much coarser each retry of a refused band footprint snaps.
const SNAP_STEP: f64 = 4.0;

/// The coarsest tolerance [`band_footprint`] ever snaps with: a hundredth
/// of [`MARGIN`], so a snapped footprint stays far inside every witness's
/// margin.
const SNAP_CEILING: f64 = MARGIN / 100.0;

/// The tolerances [`band_footprint`] hands the overlay for `polygons`, in
/// the order they are tried: from a few units in the last place of their
/// largest coordinate (at least a metre), never below [`overlay_tolerance`]'s,
/// coarser by [`SNAP_STEP`] up to [`ON_SURFACE`], or, at coordinates so
/// large that a few units in their last place already exceed it
/// (georeferenced ones, about 6e6 m, where eight units are 1e-8 m), up to
/// [`SNAP_STEP`] squared times the first, never past [`SNAP_CEILING`]. So
/// every coordinate size up to about 5e8 m has a tolerance to try (#304).
///
/// The overlay takes features closer than its tolerance as touching and
/// decides everything else exactly. Clipped corners of faces that meet in
/// the model come out a few units in the last place apart, and the overlay
/// fails to link the boundary of an arrangement holding such
/// near-coincident points (`SelfIntersection`), so they must snap; a
/// coarser snap merges more of them. Snapping moves a point by no more than
/// the tolerance, at most [`SNAP_CEILING`], far below [`MARGIN`], which
/// every witness keeps from the obstacles, and below the overlay's own grid
/// snapping of its output (axiolid/kernel#173, about `1e-7` of the
/// coordinates).
fn snapping_tolerances<'p>(
    polygons: impl IntoIterator<Item = &'p Polygon>,
) -> Result<Vec<Tolerance>, String> {
    let magnitude = polygons
        .into_iter()
        .flat_map(|polygon| &polygon.outer.points)
        .flat_map(|point| [point.x.abs(), point.y.abs()])
        .fold(1.0, f64::max);
    let mut linear = (SNAP_ULPS * f64::EPSILON * magnitude).max(overlay_tolerance()?.linear());
    let coarsest = ON_SURFACE
        .max(linear * SNAP_STEP * SNAP_STEP)
        .min(SNAP_CEILING);
    let mut tolerances = Vec::new();
    while linear <= coarsest {
        tolerances
            .push(Tolerance::new(linear, linear).map_err(|_| "invalid tolerance".to_owned())?);
        linear *= SNAP_STEP;
    }
    Ok(tolerances)
}

/// The union of the non-zero fills of `crossings` and of `above`, at the
/// first of `tolerances` the overlay accepts.
///
/// At each tolerance it is tried in one overlay, `crossings` the subject
/// and `above` the clip: the overlay fills each by its own windings before
/// it unites them, so `above` keeps its section's winding however
/// `crossings` overlaps it. The overlay may fail to link one arrangement and
/// not another of the same region, so it is then tried in two steps, each
/// soup united alone and the settled results joined. Every attempt measures
/// the same region within its snapping.
fn snapped_union(
    crossings: &[Polygon],
    above: &[Polygon],
    tolerances: &[Tolerance],
) -> Result<Plan, String> {
    if crossings.is_empty() && above.is_empty() {
        return Ok(Plan::empty());
    }
    let unite = |subject: Vec<Polygon>, clip: Vec<Polygon>, tolerance: Tolerance| {
        let frame = plan_frame();
        overlay(
            &OverlayInput {
                frame,
                polygons: subject,
            },
            &OverlayInput {
                frame,
                polygons: clip,
            },
            OverlayOperation::Union,
            FillRule::NonZero,
            tolerance,
        )
        .map(|result| result.polygons)
    };
    let mut refusal = None;
    for &tolerance in tolerances {
        let error = match unite(crossings.to_vec(), above.to_vec(), tolerance) {
            Ok(polygons) => return Ok(Plan { polygons }),
            Err(error) => error,
        };
        refusal.get_or_insert(error);
        let stepped = unite(crossings.to_vec(), Vec::new(), tolerance).and_then(|crossings| {
            unite(Vec::new(), above.to_vec(), tolerance)
                .and_then(|above| unite(crossings, above, tolerance))
        });
        if let Ok(polygons) = stepped {
            return Ok(Plan { polygons });
        }
    }
    Err(refusal.map_or_else(
        || "no snapping tolerance".to_owned(),
        |error| format!("plan Union failed: {error:?}"),
    ))
}

/// The triangles of the pieces of `mesh` ([`crate::shell::pieces`]) whose
/// lowest and highest elevations `needed` selects, in triangle order: the
/// pieces whose inside a measurement needs. Each must bound a solid; the
/// other pieces count by their surface alone, as a body that does not
/// reach the plane does.
///
/// `None` when a selected piece is open (or the mesh cannot be read), so
/// the solid it would bound is undecided.
fn solid_pieces(mesh: &TriMesh, needed: impl Fn((f64, f64)) -> bool) -> Option<Vec<usize>> {
    let mut triangles = Vec::new();
    for piece in crate::shell::pieces(mesh)? {
        if !needed(piece.z) {
            continue;
        }
        if !piece.closed {
            return None;
        }
        triangles.extend(piece.triangles);
    }
    triangles.sort_unstable();
    Some(triangles)
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
/// Only the pieces of the body ([`solid_pieces`]) that reach from below
/// `lo` into the band need to be closed (#309): the section is theirs.
/// Any other piece, open or not, counts by its boundary in the band, as a
/// whole body off the floor does.
///
/// Faces go to the overlay as triangle fans ([`projected_fan`]), united
/// ([`snapped_union`]) at a tolerance scaled to their coordinates
/// ([`snapping_tolerances`]). The overlay's output is settled, so it goes
/// back to the overlay at the tighter tolerance every other plan uses.
///
/// # Errors
///
/// When a piece reaching down to the band is not a closed solid, or the
/// overlay fails; both name `object`.
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
    let origin = local_origin(&min);
    let faces = local_faces(mesh, origin);
    let mut crossings = Vec::new();
    for [a, b, c] in &faces {
        let clipped = clip_z(&clip_z(&[*a, *b, *c], lo, true), hi, false);
        if clipped.len() < 3
            || clipped.iter().all(|p| p.z <= lo + ON_SURFACE)
            || clipped.iter().all(|p| p.z >= hi - ON_SURFACE)
        {
            continue;
        }
        for mut face in projected_fan(&clipped) {
            if ring_area(&face.outer) < 0.0 {
                face.outer.points.reverse();
            }
            crossings.push(face);
        }
    }
    let mut above = Vec::new();
    if min[2] <= lo + ON_SURFACE {
        let reaching =
            |(bottom, top): (f64, f64)| bottom <= lo + ON_SURFACE && top > lo + ON_SURFACE;
        for piece in solid_pieces(mesh, reaching).ok_or_else(|| {
            format!(
                "{object} reaches below the headroom band but is not a closed solid, so \
                     what it occupies in the band is undecided"
            )
        })? {
            let [a, b, c] = faces[piece];
            let clipped = clip_z(&[a, b, c], lo, true);
            if clipped.len() < 3 || clipped.iter().all(|p| p.z <= lo + ON_SURFACE) {
                continue;
            }
            above.extend(projected_fan(&clipped));
        }
    }
    let tolerances = snapping_tolerances(crossings.iter().chain(&above))?;
    snapped_union(&crossings, &above, &tolerances)
        .map(|plan| plan.moved(origin))
        .map_err(|error| format!("the band footprint of {object} failed: {error}"))
}

/// Coordinates from which a footprint is taken in a plan frame of its own
/// (#304): about 6e6 m, where eight units in the last place are 1e-8 m and
/// the overlay fails to link arrangements of corners a few of them apart.
/// Below it every footprint is taken where it lies, bit for bit as before.
const LOCAL_FRAME_FROM: f64 = 1.0e5;

/// The plan origin of a footprint's own frame: its body's lowest corner in
/// whole metres where its coordinates reach [`LOCAL_FRAME_FROM`], else the
/// world origin. Moving there is exact for every point within a factor two
/// of it (Sterbenz), and moving back rounds by half a unit in the last
/// place of the coordinates at most, far below [`MARGIN`].
fn local_origin(min: &[f64; 3]) -> [f64; 2] {
    if min[0].abs().max(min[1].abs()) >= LOCAL_FRAME_FROM {
        [min[0].floor(), min[1].floor()]
    } else {
        [0.0, 0.0]
    }
}

/// A mesh's triangles moved by `-origin` in plan.
fn local_faces(mesh: &TriMesh, origin: [f64; 2]) -> Vec<Triangle> {
    let faces = triangles(mesh);
    if origin == [0.0, 0.0] {
        return faces;
    }
    faces
        .into_iter()
        .map(|face| face.map(|p| Point3::new(p.x - origin[0], p.y - origin[1], p.z)))
        .collect()
}

/// Where a closed body surely occupies a vertical column of height
/// `2 * half` centred on `level`: its section just above `level` (`.0`,
/// the non-zero winding of its boundary above it, as in
/// [`band_footprint`]) and the shadow of its boundary between
/// `level - half` and `level + half` (`.1`), faces only touching those
/// limits left out. A plan point in the section but outside the shadow has
/// the whole column inside the (closed) body, since the column's centre is
/// inside and it never crosses the boundary.
///
/// A tessellated obstacle's sure footprint is the first less the second,
/// eroded by its chord deviation `d` with `half = d`: a point deeper than
/// `d` inside the mesh in every direction lies inside the true body.
///
/// The section is taken from the pieces of the body standing across
/// `level` ([`solid_pieces`]), each of which must be closed; the shadow from
/// every piece. A point in a closed piece's section and outside every
/// shadow has its column inside that piece.
///
/// # Errors
///
/// When a piece standing across `level` is not a closed solid, or the
/// overlay fails; both name `object`.
pub(crate) fn solid_column(
    object: &ObjectId,
    mesh: &TriMesh,
    level: f64,
    half: f64,
) -> Result<(Plan, Plan), String> {
    let Some((min, max)) = mesh_extent(mesh) else {
        return Ok((Plan::empty(), Plan::empty()));
    };
    if max[2] <= level || min[2] >= level {
        return Ok((Plan::empty(), Plan::empty()));
    }
    let straddling =
        solid_pieces(mesh, |(bottom, top)| bottom < level && level < top).ok_or_else(|| {
            format!(
                "{object} reaches into the band but is not a closed solid, so what it surely \
                 occupies there is undecided"
            )
        })?;
    let origin = local_origin(&min);
    let faces = local_faces(mesh, origin);
    let mut above = Vec::new();
    for triangle in straddling {
        let [a, b, c] = faces[triangle];
        let clipped = clip_z(&[a, b, c], level, true);
        if clipped.len() >= 3 && !clipped.iter().all(|p| p.z <= level) {
            above.extend(projected_fan(&clipped));
        }
    }
    let mut shadow = Vec::new();
    for [a, b, c] in &faces {
        // A face on edge casts no area; its neighbours within the column,
        // or the section's own boundary, cover its shadow.
        let within = clip_z(
            &clip_z(&[*a, *b, *c], level - half, true),
            level + half,
            false,
        );
        if within.len() >= 3
            && !within.iter().all(|p| p.z <= level - half)
            && !within.iter().all(|p| p.z >= level + half)
        {
            for mut face in projected_fan(&within) {
                if ring_area(&face.outer) < 0.0 {
                    face.outer.points.reverse();
                }
                shadow.push(face);
            }
        }
    }
    let section_tolerances = snapping_tolerances(&above)?;
    let section = snapped_union(&[], &above, &section_tolerances)
        .map_err(|error| format!("the section of {object} failed: {error}"))?;
    let shadow_tolerances = snapping_tolerances(&shadow)?;
    let shadow = snapped_union(&shadow, &[], &shadow_tolerances)
        .map_err(|error| format!("the boundary shadow of {object} failed: {error}"))?;
    Ok((section.moved(origin), shadow.moved(origin)))
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

/// What one obstacle occupies inside a band, and where its body begins.
#[derive(Clone, Debug)]
pub(crate) struct Blocker {
    pub(crate) id: ObjectId,
    pub(crate) plan: Plan,
    /// The bottom of the obstacle's body (exact: tessellations refuse).
    pub(crate) bottom: f64,
}

/// What each obstacle occupies inside the band `lo < z < hi` over `area`;
/// obstacles occupying nothing there are left out.
///
/// # Errors
///
/// When a tessellated obstacle could change it, or a band footprint cannot
/// be computed.
pub(crate) fn obstructions(
    obstacles: &[Obstacle<'_>],
    area: &Bounds2,
    lo: f64,
    hi: f64,
) -> Result<Vec<Blocker>, String> {
    let mut found = Vec::new();
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
        let plan = band_footprint(obstacle.id, obstacle.mesh, lo, hi)?;
        if !plan.is_empty() {
            found.push(Blocker {
                id: obstacle.id.clone(),
                plan,
                bottom: min[2],
            });
        }
    }
    Ok(found)
}

/// What the obstacles occupy inside the band `lo < z < hi` over `area`.
///
/// # Errors
///
/// As [`obstructions`].
pub(crate) fn obstruction(
    obstacles: &[Obstacle<'_>],
    area: &Bounds2,
    lo: f64,
    hi: f64,
) -> Result<Plan, String> {
    let mut region = Plan::empty();
    for blocker in obstructions(obstacles, area, lo, hi)? {
        region = join(&region, &blocker.plan)?;
    }
    Ok(region)
}

/// `bounds` grown by `by` on every side.
pub(crate) fn grown(bounds: &Bounds2, by: f64) -> Bounds2 {
    (
        [bounds.0[0] - by, bounds.0[1] - by],
        [bounds.1[0] + by, bounds.1[1] + by],
    )
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
    let cuts = line_cuts(region, origin, direction, t0, t1);
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

/// How much of the segment `a`–`b` lies over `region`: the length whose
/// points lie inside it farther than [`MARGIN`] from its boundary, and the
/// length inside it or within [`MARGIN`] of its boundary. The first never
/// exceeds the exact length inside, the second never falls short of the
/// length inside or along the boundary.
pub(crate) fn segment_cover(region: &Plan, a: Point2, b: Point2) -> (f64, f64) {
    let direction = b - a;
    let length = direction.length();
    if length <= 0.0 {
        return (0.0, 0.0);
    }
    let (mut inside, mut over) = (0.0, 0.0);
    for pair in line_cuts(region, a, direction, 0.0, 1.0).windows(2) {
        let middle = a + direction * f64::midpoint(pair[0], pair[1]);
        let part = (pair[1] - pair[0]) * length;
        let near = rings(region)
            .flat_map(edges)
            .any(|edge| point_segment_distance(middle, edge) <= MARGIN);
        let within = contains(region, middle);
        if within || near {
            over += part;
        }
        if within && !near {
            inside += part;
        }
    }
    (inside, over)
}

fn point_segment_distance(point: Point2, (start, end): (Point2, Point2)) -> f64 {
    let along = end - start;
    let length = along.length_squared();
    let t = if length <= 0.0 {
        0.0
    } else {
        ((point - start).dot(along) / length).clamp(0.0, 1.0)
    };
    (point - (start + along * t)).length()
}

/// The parameters in `[t0, t1]` where `origin + direction * t` meets the
/// boundary of `region`, with both ends, sorted: between two consecutive
/// ones the line lies wholly inside, outside or along the boundary.
fn line_cuts(region: &Plan, origin: Point2, direction: Vec2, t0: f64, t1: f64) -> Vec<f64> {
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
    cuts
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
    let sweep = sweep(path, radius + MARGIN)?;
    Ok(subtract(&sweep, domain)?.is_empty())
}

/// A region enclosing the sweep of a disc of `radius` along `path`:
/// rectangles along the segments and circumscribed polygons at the vertices.
pub(crate) fn sweep(path: &[Point2], radius: f64) -> Result<Plan, String> {
    let mut parts: Vec<Polygon> = path
        .iter()
        .filter_map(|point| circumscribed(*point, radius))
        .collect();
    for pair in path.windows(2) {
        let along = pair[1] - pair[0];
        let length = along.length();
        if length <= ON_SURFACE {
            continue;
        }
        let side = along.perp() / length * radius;
        parts.extend(polygon(vec![
            pair[0] - side,
            pair[1] - side,
            pair[1] + side,
            pair[0] + side,
        ]));
    }
    union(parts)
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
    let point_segment = point_segment_distance;
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
    within(a, b, 0.0)
}

/// Whether two regions overlap or come within `reach` of each other (at
/// least [`ON_SURFACE`]).
pub(crate) fn within(a: &Plan, b: &Plan, reach: f64) -> Result<bool, String> {
    let reach = reach.max(ON_SURFACE);
    let (Some(first), Some(second)) = (region_bounds(a), region_bounds(b)) else {
        return Ok(false);
    };
    if bounds_gap(&first, &second) > reach {
        return Ok(false);
    }
    if !intersect(a, b)?.is_empty() {
        return Ok(true);
    }
    for ring_a in rings(a) {
        for edge_a in edges(ring_a) {
            for ring_b in rings(b) {
                if edges(ring_b).any(|edge_b| segment_distance(edge_a, edge_b) <= reach) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// The closest point of `(start, end)` to `point`.
fn nearest_on(point: Point2, (start, end): (Point2, Point2)) -> Point2 {
    let along = end - start;
    let length = along.length_squared();
    if length <= 0.0 {
        return start;
    }
    start + along * ((point - start).dot(along) / length).clamp(0.0, 1.0)
}

/// A pair of points, one on each region's boundary, as close as any: where
/// the two regions come closest. `None` when either is empty.
pub(crate) fn closest(a: &Plan, b: &Plan) -> Option<(Point2, Point2)> {
    let mut best: Option<(f64, Point2, Point2)> = None;
    for ring_a in rings(a) {
        for edge_a in edges(ring_a) {
            for ring_b in rings(b) {
                for edge_b in edges(ring_b) {
                    for (p, q) in [
                        (edge_a.0, nearest_on(edge_a.0, edge_b)),
                        (edge_a.1, nearest_on(edge_a.1, edge_b)),
                        (nearest_on(edge_b.0, edge_a), edge_b.0),
                        (nearest_on(edge_b.1, edge_a), edge_b.1),
                    ] {
                        let distance = (p - q).length();
                        if best.is_none_or(|(least, ..)| distance < least) {
                            best = Some((distance, p, q));
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, p, q)| (p, q))
}

/// A point inside `plan`: the middle of its first convex piece.
pub(crate) fn inner_point(plan: &Plan) -> Option<Point2> {
    trapezoids(plan).into_iter().find_map(|piece| {
        let points = &piece.outer.points;
        let count = points.len();
        if count < 3 {
            return None;
        }
        let sum = points
            .iter()
            .fold(Vec2::ZERO, |sum, point| sum + Vec2::new(point.x, point.y));
        #[allow(clippy::cast_precision_loss)]
        let middle = sum / count as f64;
        Some(Point2::new(middle.x, middle.y))
    })
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
