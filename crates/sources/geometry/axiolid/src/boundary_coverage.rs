//! Space-boundary coverage: how much of a space body's surface the space's
//! declared boundary surfaces cover.
//!
//! ADR 0004: this module measures; whether coverage suffices is a rule's
//! decision.
//!
//! The body's faces are grouped by oriented plane. Each boundary triangle is
//! assigned to the face plane all its corners lie within the request's
//! tolerance of (the nearest, when several do); a boundary with a triangle
//! on no face plane lies off the surface, is reported so and covers
//! nothing. Within each plane the faces and every boundary's part are
//! projected into the plane and unioned into overlay regions; the uncovered
//! area is the faces' region less the boundaries' union, the covered area
//! the rest of the faces, and the overlap the part two boundaries
//! cover, accumulated boundary by boundary so three overlapping boundaries
//! count their common part once.
//!
//! A plane along a coordinate axis whose points all share that coordinate
//! projects by dropping it, without rounding; any other plane projects
//! through rounded dot products, so every area there widens by the band the
//! rounding can move each region's boundary by. A tessellated boundary
//! widens by its chord deviation in the same way. Evidence is exact only
//! when nothing widened and every boundary point lies exactly on its plane.
//! A curved (tessellated) body refuses: its facets are not the faces a
//! boundary lies on.

use std::collections::BTreeMap;

use axiolid_core::{Point2, Point3, Tolerance};
use axiolid_mesh::TriMesh;
use axiolid_overlay::{Region, Ring, union_soup};
use axioval_engine::{
    BoundaryCoverage, BoundaryCoverageError, BoundaryCoverageRequest, BoundaryCoverageService,
    BoundaryOverlap, BoundaryPlacement, CoverageAreas, MeasuredBoundary, SurfaceAreaInterval,
};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::{AxiolidGeometry, Triangle, triangles};
use crate::planar::{OVERLAY_SNAP, collinear, ring_perimeter};

/// Body triangles whose normals differ by less than this (as `1 − cos`) and
/// whose corners lie within [`FACE_PLANE_METRES`] of one plane form one face
/// plane. Both are far below a millimetre and far above the rounding a
/// placement leaves in an exact mesh.
const FACE_NORMAL_TOLERANCE: f64 = 1e-9;
const FACE_PLANE_METRES: f64 = 1e-6;

/// Areas below this are numerical dust (as in free space), not overlap.
const AREA_EPSILON_M2: f64 = 1e-9;

/// Triangles with less area than this have no plane.
const DEGENERATE_M2: f64 = 1e-15;

/// A boundary surface as the host registered it.
#[derive(Clone, Debug)]
struct Boundary {
    element: Option<ObjectId>,
    surface: Result<(TriMesh, Option<f64>), String>,
}

/// Measures space-boundary coverage from host-registered boundary surfaces.
#[derive(Clone, Debug)]
pub struct AxiolidBoundaryCoverageService {
    geometry: AxiolidGeometry,
    spaces: BTreeMap<ObjectId, BTreeMap<ObjectId, Boundary>>,
}

impl AxiolidBoundaryCoverageService {
    /// A service over `geometry`, which holds the spaces' bodies. It knows
    /// no space until one is declared.
    #[must_use]
    pub fn new(geometry: AxiolidGeometry) -> Self {
        Self {
            geometry,
            spaces: BTreeMap::new(),
        }
    }

    /// Declares `space` a space whose boundaries this service knows
    /// completely; without a boundary registered afterwards, it has none.
    #[must_use]
    pub fn with_space(mut self, space: ObjectId) -> Self {
        self.spaces.entry(space).or_default();
        self
    }

    /// Registers a boundary of `space` whose surface `mesh` is exact (every
    /// face planar), in the same coordinates as the space's body. `element`
    /// is the element on its other side, when the source names one.
    #[must_use]
    pub fn with_boundary(
        self,
        space: ObjectId,
        boundary: ObjectId,
        element: Option<ObjectId>,
        mesh: TriMesh,
    ) -> Self {
        self.insert(space, boundary, element, Ok((mesh, None)))
    }

    /// Registers a boundary whose surface `mesh` approximates a curved one
    /// within `chord_deviation_metres`.
    #[must_use]
    pub fn with_tessellated_boundary(
        self,
        space: ObjectId,
        boundary: ObjectId,
        element: Option<ObjectId>,
        mesh: TriMesh,
        chord_deviation_metres: f64,
    ) -> Self {
        self.insert(
            space,
            boundary,
            element,
            Ok((mesh, Some(chord_deviation_metres))),
        )
    }

    /// Registers a boundary whose surface the host could not read. The
    /// space is then not measured: its coverage without the boundary would
    /// be a guess.
    #[must_use]
    pub fn with_unmeasured_boundary(
        self,
        space: ObjectId,
        boundary: ObjectId,
        element: Option<ObjectId>,
        reason: impl Into<String>,
    ) -> Self {
        self.insert(space, boundary, element, Err(reason.into()))
    }

    fn insert(
        mut self,
        space: ObjectId,
        boundary: ObjectId,
        element: Option<ObjectId>,
        surface: Result<(TriMesh, Option<f64>), String>,
    ) -> Self {
        self.spaces
            .entry(space)
            .or_default()
            .insert(boundary, Boundary { element, surface });
        self
    }

    fn body(&self, space: &ObjectId) -> Result<&TriMesh, BoundaryCoverageError> {
        if self.geometry.has_no_body(space) {
            return Err(BoundaryCoverageError::NoBody(space.clone()));
        }
        if self.geometry.is_unmeasured(space) {
            return Err(BoundaryCoverageError::Unavailable(format!(
                "{space} has a body that was not meshed"
            )));
        }
        if self.geometry.is_tessellated(space) {
            return Err(BoundaryCoverageError::Unavailable(format!(
                "{space} has curved faces, whose facets are not the faces its boundaries lie on"
            )));
        }
        self.geometry.mesh(space).ok_or_else(|| {
            BoundaryCoverageError::Unavailable(format!("{space} has no registered mesh"))
        })
    }
}

impl BoundaryCoverageService for AxiolidBoundaryCoverageService {
    fn measure_boundary_coverage(
        &self,
        request: &BoundaryCoverageRequest,
    ) -> Result<BoundaryCoverage, BoundaryCoverageError> {
        let space = request.space();
        let declared = self
            .spaces
            .get(space)
            .ok_or_else(|| BoundaryCoverageError::UnknownSpace(space.clone()))?;
        let body = self.body(space)?;
        let mut boundaries = Vec::with_capacity(declared.len());
        for (id, boundary) in declared {
            let (mesh, deviation) = boundary.surface.as_ref().map_err(|reason| {
                BoundaryCoverageError::Unavailable(format!(
                    "boundary {id} of {space} has no measurable surface: {reason}"
                ))
            })?;
            if deviation.is_some_and(|d| !d.is_finite() || d < 0.0) {
                return Err(BoundaryCoverageError::Unavailable(format!(
                    "boundary {id} of {space} declares an invalid chord deviation"
                )));
            }
            boundaries.push(Input {
                id: id.clone(),
                element: boundary.element.clone(),
                triangles: triangles(mesh),
                deviation: *deviation,
            });
        }
        measure(request, &triangles(body), &boundaries)
    }
}

/// One boundary's surface, ready to measure.
struct Input {
    id: ObjectId,
    element: Option<ObjectId>,
    triangles: Vec<Triangle>,
    deviation: Option<f64>,
}

/// An oriented face plane of the body and its triangles.
struct FacePlane {
    normal: [f64; 3],
    offset: f64,
    faces: Vec<Triangle>,
    /// Per boundary index, its triangles on this plane.
    boundaries: BTreeMap<usize, Vec<Triangle>>,
}

/// How a plane's points are mapped into it.
enum Projection {
    /// Drop coordinate `axis`: exact.
    Axis(usize),
    /// Dot products with two in-plane axes from `origin`: rounded.
    Frame {
        origin: [f64; 3],
        u: [f64; 3],
        v: [f64; 3],
    },
}

impl Projection {
    fn project(&self, p: [f64; 3]) -> Point2 {
        match self {
            Self::Axis(axis) => {
                let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
                Point2::new(p[a], p[b])
            }
            Self::Frame { origin, u, v } => {
                let d = sub(p, *origin);
                Point2::new(dot(d, *u), dot(d, *v))
            }
        }
    }
}

fn measure(
    request: &BoundaryCoverageRequest,
    body: &[Triangle],
    boundaries: &[Input],
) -> Result<BoundaryCoverage, BoundaryCoverageError> {
    let tolerance = Tolerance::new(1e-9, 1e-9)
        .map_err(|error| BoundaryCoverageError::Unavailable(format!("tolerance: {error:?}")))?;
    let mut planes = face_planes(body);
    if planes.is_empty() {
        return Err(BoundaryCoverageError::Unavailable(format!(
            "{} has a body without faces",
            request.space()
        )));
    }
    let reach = request.plane_tolerance_metres() + FACE_PLANE_METRES;
    let (off_surface, on_planes) = assign(&mut planes, boundaries, reach);
    let mut tally = Tally {
        exact: on_planes && boundaries.iter().all(|b| b.deviation.is_none()),
        boundaries: vec![Bounds::default(); boundaries.len()],
        ..Tally::default()
    };
    for plane in &planes {
        tally.plane(plane, boundaries, tolerance)?;
    }

    let surface_upper = tally.surface.upper();
    let clamp = |bounds: &Bounds| bounds.interval(surface_upper);
    let areas = CoverageAreas {
        surface: tally.surface.interval(f64::INFINITY)?,
        covered: clamp(&tally.covered)?,
        uncovered: clamp(&tally.uncovered)?,
        overlap: clamp(&tally.overlap)?,
    };
    let measured = boundaries
        .iter()
        .enumerate()
        .map(|(index, boundary)| {
            let placement = if off_surface[index] {
                BoundaryPlacement::OffSurface
            } else {
                BoundaryPlacement::OnSurface {
                    area: tally.boundaries[index].interval(f64::INFINITY)?,
                }
            };
            Ok(MeasuredBoundary::new(
                boundary.id.clone(),
                boundary.element.clone(),
                placement,
            ))
        })
        .collect::<Result<Vec<_>, BoundaryCoverageError>>()?;
    let overlaps = tally
        .pairs
        .iter()
        .map(|((first, second), bounds)| {
            BoundaryOverlap::try_new(
                boundaries[*first].id.clone(),
                boundaries[*second].id.clone(),
                bounds.interval(f64::INFINITY)?,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let space = request.space();
    let evidence = Evidence {
        source: space.source.clone(),
        locator: format!("axiolid:boundary-coverage:{space}"),
        exact: tally.exact,
    };
    BoundaryCoverage::try_new(request.clone(), areas, measured, overlaps, evidence)
}

/// Assigns each boundary triangle to the nearest face plane all its corners
/// lie within `reach` of. A boundary with a triangle on no plane lies off
/// the surface and is assigned nowhere. Returns which boundaries lie off the
/// surface, and whether every assigned corner lies exactly on its plane.
fn assign(planes: &mut [FacePlane], boundaries: &[Input], reach: f64) -> (Vec<bool>, bool) {
    let mut on_planes = true;
    let mut off_surface = vec![false; boundaries.len()];
    for (index, boundary) in boundaries.iter().enumerate() {
        let mut assigned: Vec<(usize, Triangle)> = Vec::new();
        for triangle in &boundary.triangles {
            if area3(triangle) <= DEGENERATE_M2 {
                continue;
            }
            let nearest = planes
                .iter()
                .enumerate()
                .map(|(plane, face)| (plane, farthest_corner(face, triangle)))
                .filter(|(_, distance)| *distance <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((plane, distance)) = nearest else {
                off_surface[index] = true;
                break;
            };
            on_planes &= distance == 0.0;
            assigned.push((plane, *triangle));
        }
        if !off_surface[index] {
            for (plane, triangle) in assigned {
                planes[plane]
                    .boundaries
                    .entry(index)
                    .or_default()
                    .push(triangle);
            }
        }
    }
    (off_surface, on_planes)
}

/// Areas summed over the face planes.
#[derive(Default)]
struct Tally {
    exact: bool,
    surface: Bounds,
    covered: Bounds,
    uncovered: Bounds,
    overlap: Bounds,
    /// Per boundary index, its area on the surface.
    boundaries: Vec<Bounds>,
    /// Per pair of boundary indices, the area both cover.
    pairs: BTreeMap<(usize, usize), Bounds>,
}

impl Tally {
    /// Adds one face plane's areas.
    fn plane(
        &mut self,
        plane: &FacePlane,
        boundaries: &[Input],
        tolerance: Tolerance,
    ) -> Result<(), BoundaryCoverageError> {
        let (projection, rounding) = projection(plane);
        self.exact &= rounding == 0.0;
        let region = |triangles: &[Triangle]| region(&projection, triangles, tolerance);
        let faces = region(&plane.faces)?;
        let face_margin = margin(&faces, rounding);
        self.surface.add(faces.area(), face_margin);

        // Accumulated boundary by boundary: what the next boundary shares
        // with the union so far is covered twice or more.
        let mut union = Region::empty();
        let mut overlapped = Region::empty();
        let mut parts: Vec<(usize, Region, f64)> = Vec::new();
        for (&index, triangles) in &plane.boundaries {
            let part = region(triangles)?;
            let part_margin = margin(&part, rounding + boundaries[index].deviation.unwrap_or(0.0));
            self.boundaries[index].add(part.area(), part_margin);
            let common = union.intersection(&part, tolerance).map_err(overlay)?;
            overlapped = overlapped.union(&common, tolerance).map_err(overlay)?;
            union = union.union(&part, tolerance).map_err(overlay)?;
            parts.push((index, part, part_margin));
        }
        let plane_margin = face_margin + parts.iter().map(|(_, _, margin)| margin).sum::<f64>();
        // The uncovered part is measured directly and the covered part is
        // the rest of the faces, so a face its boundaries cover whole is
        // covered by exactly its area rather than by an overlay's rounding
        // of it.
        let outside = faces.difference(&union, tolerance).map_err(overlay)?;
        let uncovered = dust(outside.area());
        self.uncovered.add(uncovered, plane_margin);
        self.covered
            .add((faces.area() - uncovered).max(0.0), plane_margin);
        let shared = faces
            .intersection(&overlapped, tolerance)
            .map_err(overlay)?;
        self.overlap.add(dust(shared.area()), plane_margin);

        for (i, (first, first_part, first_margin)) in parts.iter().enumerate() {
            for (second, second_part, second_margin) in &parts[i + 1..] {
                let common = first_part
                    .intersection(second_part, tolerance)
                    .and_then(|common| faces.intersection(&common, tolerance))
                    .map_err(overlay)?;
                let area = dust(common.area());
                let pair_margin = face_margin + first_margin + second_margin;
                if area > 0.0 || (pair_margin > 0.0 && !common.is_empty()) {
                    self.pairs
                        .entry((*first, *second))
                        .or_default()
                        .add(area, pair_margin);
                }
            }
        }
        Ok(())
    }
}

/// An area measured with a margin on either side, summed over planes.
#[derive(Clone, Copy, Debug, Default)]
struct Bounds {
    value: f64,
    margin: f64,
}

impl Bounds {
    fn add(&mut self, value: f64, margin: f64) {
        self.value += value;
        self.margin += margin;
    }

    fn upper(&self) -> f64 {
        self.value + self.margin
    }

    fn interval(&self, cap: f64) -> Result<SurfaceAreaInterval, BoundaryCoverageError> {
        let value = self.value.max(0.0);
        if self.margin > 0.0 {
            SurfaceAreaInterval::try_new(
                (value - self.margin).max(0.0),
                (value + self.margin).min(cap),
            )
        } else {
            SurfaceAreaInterval::exact(value)
        }
    }
}

/// The body's triangles grouped by oriented plane, in first-seen order.
fn face_planes(body: &[Triangle]) -> Vec<FacePlane> {
    let mut planes: Vec<FacePlane> = Vec::new();
    for triangle in body {
        if area3(triangle) <= DEGENERATE_M2 {
            continue;
        }
        let normal = unit_normal(triangle);
        let found = planes.iter_mut().find(|plane| {
            dot(plane.normal, normal) >= 1.0 - FACE_NORMAL_TOLERANCE
                && farthest_corner(plane, triangle) <= FACE_PLANE_METRES
        });
        match found {
            Some(plane) => plane.faces.push(*triangle),
            None => planes.push(FacePlane {
                normal,
                offset: dot(normal, triangle[0].to_array()),
                faces: vec![*triangle],
                boundaries: BTreeMap::new(),
            }),
        }
    }
    planes
}

/// How far the farthest corner of `triangle` lies from `plane`.
fn farthest_corner(plane: &FacePlane, triangle: &Triangle) -> f64 {
    triangle
        .iter()
        .map(|corner| (dot(plane.normal, corner.to_array()) - plane.offset).abs())
        .fold(0.0, f64::max)
}

/// The projection into `plane` and how far rounding can move a projected
/// point: zero along a coordinate axis every face point shares.
fn projection(plane: &FacePlane) -> (Projection, f64) {
    let points = || {
        plane
            .faces
            .iter()
            .chain(plane.boundaries.values().flatten())
            .flatten()
            .map(Point3::to_array)
    };
    let axis = (0..3)
        .max_by(|a, b| plane.normal[*a].abs().total_cmp(&plane.normal[*b].abs()))
        .unwrap_or(2);
    // Dropping a coordinate projects orthogonally without rounding when the
    // faces all share it; a boundary point off the plane only moves along it.
    let first = plane.faces[0][0].to_array()[axis];
    #[allow(clippy::float_cmp)]
    if plane
        .faces
        .iter()
        .flatten()
        .all(|point| point.to_array()[axis] == first)
    {
        return (Projection::Axis(axis), 0.0);
    }
    let origin = plane.faces[0][0].to_array();
    let mut least = [0.0; 3];
    let least_axis = (0..3)
        .min_by(|a, b| plane.normal[*a].abs().total_cmp(&plane.normal[*b].abs()))
        .unwrap_or(0);
    least[least_axis] = 1.0;
    let u = normalise(cross(plane.normal, least));
    let v = cross(plane.normal, u);
    let reach = points()
        .map(|point| norm(sub(point, origin)))
        .fold(0.0, f64::max);
    // Each coordinate is a three-term dot product of a rounded difference
    // with a rounded unit axis: a few units of rounding in the reach. The
    // overlay then snaps the rounded coordinates to its integer grid
    // (axiolid/kernel#173), which moves them by far more.
    let rounding = (8.0 * f64::EPSILON + OVERLAY_SNAP) * reach.max(1.0);
    (Projection::Frame { origin, u, v }, rounding)
}

/// The union of `triangles` projected into the plane, as a region.
fn region(
    projection: &Projection,
    triangles: &[Triangle],
    tolerance: Tolerance,
) -> Result<Region, BoundaryCoverageError> {
    let rings: Vec<Ring> = triangles
        .iter()
        .filter_map(|triangle| {
            let mut points: Vec<Point2> = triangle
                .iter()
                .map(|point| projection.project(point.to_array()))
                .collect();
            let [a, b, c] = [points[0], points[1], points[2]];
            let signed = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
            // A shadow without area (a face seen edge-on) covers nothing.
            if signed.abs() <= 2.0 * DEGENERATE_M2 || collinear(&points) {
                return None;
            }
            if signed < 0.0 {
                points.reverse();
            }
            Some(Ring { points })
        })
        .collect();
    if rings.is_empty() {
        return Ok(Region::empty());
    }
    let polygons = union_soup(&rings, tolerance).map_err(overlay)?;
    Region::new(polygons, tolerance).map_err(overlay)
}

/// The area a region's boundary can sweep when every point of it may lie
/// `width` away: a band of that half-width along its rings.
fn margin(region: &Region, width: f64) -> f64 {
    if width <= 0.0 {
        return 0.0;
    }
    let perimeter: f64 = region.boundary_rings().iter().map(ring_perimeter).sum();
    2.0 * perimeter * width + std::f64::consts::PI * width * width
}

fn dust(area: f64) -> f64 {
    if area <= AREA_EPSILON_M2 { 0.0 } else { area }
}

#[allow(clippy::needless_pass_by_value)]
fn overlay(error: axiolid_overlay::OverlayError) -> BoundaryCoverageError {
    BoundaryCoverageError::Unavailable(format!("overlay: {error:?}"))
}

fn area3(triangle: &Triangle) -> f64 {
    let [a, b, c] = triangle.map(|point| point.to_array());
    norm(cross(sub(b, a), sub(c, a))) * 0.5
}

fn unit_normal(triangle: &Triangle) -> [f64; 3] {
    let [a, b, c] = triangle.map(|point| point.to_array());
    normalise(cross(sub(b, a), sub(c, a)))
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalise(a: [f64; 3]) -> [f64; 3] {
    let length = norm(a);
    [a[0] / length, a[1] / length, a[2] / length]
}
