//! A terrain surface as a triangulated irregular network (TIN), and the
//! service serving such surfaces.
//!
//! The elevation at a plan point is the plane of the triangle holding it
//! (barycentric interpolation), widened by the surface's stated accuracy
//! and the interpolation's rounding. A point on an edge or a vertex is
//! held by every triangle meeting there, which agree on a TIN; triangles
//! that disagree there fold the surface over the point, which is refused.
//! Triangles without plan area (vertical faces) hold no point.
//!
//! The mean along a ring is exact for the TIN: each ring edge is cut where
//! it crosses a triangle edge, the elevation is linear between two cuts,
//! and the trapezoids are summed and divided by the ring's plan length. A
//! piece between two cuts lies wholly in one triangle or wholly outside
//! every one, so a piece whose midpoint no triangle holds leaves the ring
//! partly outside the surface, refused by that midpoint. A ring stated
//! within a deviation `d` of the true outline widens the mean by `d` times
//! the steepest gradient of the triangles it crosses.

use std::collections::BTreeMap;

use super::{
    TERRAIN_LOCATOR_PREFIX, TerrainElevation, TerrainError, TerrainFrame, TerrainKind,
    TerrainOutlineRequest, TerrainPointRequest, TerrainService, TerrainSurfaceStatement,
};

/// The largest terrain file a host should read, in bytes.
pub const TERRAIN_FILE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;

/// Barycentric tolerance: a point this far outside an edge, relative to the
/// triangle, still lies on it.
const ON_EDGE: f64 = 1e-12;

/// Elevations at one plan point closer than this, in metres plus a relative
/// part, are one elevation: two triangles meeting there agree.
const AGREE_METRES: f64 = 1e-6;

/// The relative rounding allowed an interpolated elevation, times the sum of
/// the barycentric weights' magnitudes.
const ROUNDING: f64 = 1e-12;

/// A triangle edge parallel to a ring edge and this close to its line, in
/// metres, runs along it.
const COLLINEAR_METRES: f64 = 1e-9;

/// One terrain surface: points and triangles, the coordinates they are
/// given in, the stated accuracy and the file they were read from.
#[derive(Clone, Debug)]
pub struct TerrainSurface {
    kind: TerrainKind,
    name: String,
    frame: TerrainFrame,
    crs: Option<String>,
    accuracy_metres: f64,
    origin: String,
    sha256: String,
    /// The plan centre every point is stored relative to, so differences
    /// of map coordinates keep their precision.
    centre: [f64; 2],
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    grid: Grid,
}

impl TerrainSurface {
    /// A surface of `kind`, named `name` in its source, read from the file
    /// `origin` whose bytes have the lowercase hex SHA-256 `sha256`.
    ///
    /// `points` are `[x, y, z]` in metres in `frame` (easting, northing and
    /// height for a map surface), and `triangles` index them. Every point
    /// must be finite, every triangle name three distinct points, and at
    /// least one triangle have plan area. `accuracy_metres` is the stated
    /// vertical accuracy, a finite non-negative length; `crs` the map
    /// coordinate reference system the source names, for a map surface
    /// only.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        kind: TerrainKind,
        name: impl Into<String>,
        frame: TerrainFrame,
        crs: Option<String>,
        accuracy_metres: f64,
        (origin, sha256): (impl Into<String>, impl Into<String>),
        points: &[[f64; 3]],
        triangles: Vec<[u32; 3]>,
    ) -> Result<Self, TerrainError> {
        let name = name.into();
        if !accuracy_metres.is_finite() || accuracy_metres < 0.0 {
            return Err(TerrainError::Invalid(format!(
                "a vertical accuracy of {accuracy_metres} m is not a non-negative length"
            )));
        }
        if crs.is_some() && frame != TerrainFrame::Map {
            return Err(TerrainError::Invalid(
                "a surface in model coordinates names a map coordinate reference system".into(),
            ));
        }
        if let Some(index) = points
            .iter()
            .position(|point| !point.iter().all(|value| value.is_finite()))
        {
            return Err(TerrainError::Invalid(format!(
                "point {} of surface `{name}` is not finite",
                index + 1
            )));
        }
        let count = u32::try_from(points.len())
            .map_err(|_| TerrainError::Invalid("the surface has too many points".into()))?;
        for (index, triangle) in triangles.iter().enumerate() {
            let [a, b, c] = *triangle;
            if a >= count || b >= count || c >= count || a == b || b == c || a == c {
                return Err(TerrainError::Invalid(format!(
                    "triangle {} of surface `{name}` does not name three distinct points",
                    index + 1
                )));
            }
        }
        let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for point in points {
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        let centre = if points.is_empty() {
            [0.0; 2]
        } else {
            [
                low[0] + (high[0] - low[0]) / 2.0,
                low[1] + (high[1] - low[1]) / 2.0,
            ]
        };
        let points: Vec<[f64; 3]> = points
            .iter()
            .map(|point| [point[0] - centre[0], point[1] - centre[1], point[2]])
            .collect();
        let triangles: Vec<[u32; 3]> = triangles
            .into_iter()
            .filter(|triangle| plan_area(&points, *triangle) != 0.0)
            .collect();
        if triangles.is_empty() {
            return Err(TerrainError::Invalid(format!(
                "surface `{name}` has no triangle with plan area"
            )));
        }
        let grid = Grid::new(&points, &triangles);
        Ok(Self {
            kind,
            name,
            frame,
            crs: crs.filter(|crs| !crs.trim().is_empty()),
            accuracy_metres,
            origin: origin.into(),
            sha256: sha256.into(),
            centre,
            points,
            triangles,
            grid,
        })
    }

    /// Which ground the surface is.
    #[must_use]
    pub fn kind(&self) -> TerrainKind {
        self.kind
    }

    /// The surface's name in its source.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The coordinates the surface is given in.
    #[must_use]
    pub fn frame(&self) -> TerrainFrame {
        self.frame
    }

    /// The map coordinate reference system the source names, if any.
    #[must_use]
    pub fn crs(&self) -> Option<&str> {
        self.crs.as_deref()
    }

    /// The stated vertical accuracy, in metres.
    #[must_use]
    pub fn accuracy_metres(&self) -> f64 {
        self.accuracy_metres
    }

    /// The file the surface was read from.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The lowercase hex SHA-256 of the file's bytes.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// How many points the surface has.
    #[must_use]
    pub fn point_count(&self) -> usize {
        self.points.len()
    }

    /// How many triangles with plan area the surface has.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Where the surface comes from:
    /// `axioval:terrain;kind=<kind>;sha256=<digest>#surface=<name>`.
    #[must_use]
    pub fn locator(&self) -> String {
        format!(
            "{TERRAIN_LOCATOR_PREFIX};kind={};sha256={}#surface={}",
            self.kind, self.sha256, self.name
        )
    }

    /// What a service states about the surface.
    #[must_use]
    pub fn statement(&self) -> TerrainSurfaceStatement {
        TerrainSurfaceStatement {
            kind: self.kind,
            name: self.name.clone(),
            frame: self.frame,
            crs: self.crs.clone(),
            accuracy_metres: self.accuracy_metres,
            locator: self.locator(),
        }
    }

    /// The elevation at a plan point (in the surface's coordinates), with
    /// the rounding of its interpolation.
    pub fn elevation_at(&self, point: [f64; 2]) -> Result<(f64, f64), TerrainError> {
        let local = [point[0] - self.centre[0], point[1] - self.centre[1]];
        let (_, elevation, slack) = self.locate(local, point)?;
        Ok((elevation, slack))
    }

    /// The triangle holding `local` and the elevation there with its
    /// rounding; `point` names it in a refusal.
    fn locate(&self, local: [f64; 2], point: [f64; 2]) -> Result<(u32, f64, f64), TerrainError> {
        let mut found: Option<(u32, f64, f64)> = None;
        for &index in self.grid.at(local) {
            let triangle = self.triangles[index as usize];
            let Some(weights) = barycentric(&self.points, triangle, local) else {
                continue;
            };
            let (elevation, slack) = self.interpolate(triangle, weights);
            match found {
                None => found = Some((index, elevation, slack)),
                Some((_, first, first_slack)) => {
                    let allowed = AGREE_METRES + slack + first_slack;
                    if (elevation - first).abs() > allowed {
                        return Err(TerrainError::Folded {
                            kind: self.kind,
                            surface: self.name.clone(),
                            point,
                            elevations: [first.min(elevation), first.max(elevation)],
                        });
                    }
                    let spread = (elevation - first).abs();
                    found = Some((index, first, first_slack.max(slack + spread)));
                }
            }
        }
        found.ok_or_else(|| TerrainError::Outside {
            kind: self.kind,
            surface: self.name.clone(),
            point,
        })
    }

    /// The elevation of `triangle`'s plane at the barycentric `weights`,
    /// and a bound on its rounding.
    fn interpolate(&self, triangle: [u32; 3], weights: [f64; 3]) -> (f64, f64) {
        let z = triangle.map(|index| self.points[index as usize][2]);
        let elevation = weights[0].mul_add(z[0], weights[1].mul_add(z[1], weights[2] * z[2]));
        let magnitude = z.iter().fold(1.0_f64, |most, value| most.max(value.abs()));
        let weight: f64 = weights.iter().map(|weight| weight.abs()).sum();
        (elevation, ROUNDING * magnitude * weight.max(1.0))
    }

    /// The steepest gradient of `triangle`'s plane, rise over plan run.
    fn gradient(&self, triangle: [u32; 3]) -> f64 {
        let [a, b, c] = triangle.map(|index| self.points[index as usize]);
        let (ux, uy, uz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        let (vx, vy, vz) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
        let determinant = ux * vy - uy * vx;
        let dx = (uz * vy - vz * uy) / determinant;
        let dy = (ux * vz - vx * uz) / determinant;
        dx.hypot(dy)
    }

    /// The mean elevation along `rings` (in the surface's coordinates),
    /// weighted by plan length, with the rounding of its arithmetic and the
    /// steepest gradient of the triangles the rings cross.
    pub fn mean_along(&self, rings: &[Vec<[f64; 2]>]) -> Result<(f64, f64, f64), TerrainError> {
        let mut integral = 0.0_f64;
        let mut length = 0.0_f64;
        let mut slack = 0.0_f64;
        let mut steepest = 0.0_f64;
        let mut magnitude = 1.0_f64;
        for ring in rings {
            for index in 0..ring.len() {
                let (from, to) = (ring[index], ring[(index + 1) % ring.len()]);
                let a = [from[0] - self.centre[0], from[1] - self.centre[1]];
                let b = [to[0] - self.centre[0], to[1] - self.centre[1]];
                let span = (b[0] - a[0]).hypot(b[1] - a[1]);
                if span == 0.0 {
                    continue;
                }
                for (start, end) in self.cuts(a, b) {
                    let at = |t: f64| {
                        [
                            (b[0] - a[0]).mul_add(t, a[0]),
                            (b[1] - a[1]).mul_add(t, a[1]),
                        ]
                    };
                    let middle = at(f64::midpoint(start, end));
                    let named = [middle[0] + self.centre[0], middle[1] + self.centre[1]];
                    let (triangle, _, _) = self.locate(middle, named)?;
                    let triangle = self.triangles[triangle as usize];
                    let mut ends = [0.0; 2];
                    for (slot, t) in ends.iter_mut().zip([start, end]) {
                        let weights = plane_weights(&self.points, triangle, at(t));
                        let (elevation, rounding) = self.interpolate(triangle, weights);
                        *slot = elevation;
                        slack = slack.max(rounding);
                        magnitude = magnitude.max(elevation.abs());
                    }
                    let piece = (end - start) * span;
                    integral += f64::midpoint(ends[0], ends[1]) * piece;
                    steepest = steepest.max(self.gradient(triangle));
                }
                length += span;
            }
        }
        if length <= 0.0 {
            return Err(TerrainError::InvalidMeasurement);
        }
        let mean = integral / length;
        Ok((mean, slack + ROUNDING * magnitude * 64.0, steepest))
    }

    /// The pieces `[start, end]` of the segment `a`–`b` (local coordinates)
    /// between the parameters where it crosses an edge of a triangle near
    /// it, in order.
    #[allow(clippy::many_single_char_names)]
    fn cuts(&self, a: [f64; 2], b: [f64; 2]) -> Vec<(f64, f64)> {
        let mut at = vec![0.0, 1.0];
        let d = [b[0] - a[0], b[1] - a[1]];
        let squared = d[0].mul_add(d[0], d[1] * d[1]);
        for index in self.grid.along(a, b) {
            let triangle = self.triangles[index as usize];
            for edge in 0..3 {
                let p = self.points[triangle[edge] as usize];
                let q = self.points[triangle[(edge + 1) % 3] as usize];
                let e = [q[0] - p[0], q[1] - p[1]];
                let w = [p[0] - a[0], p[1] - a[1]];
                let denominator = cross(d, e);
                let scale = (squared * e[0].mul_add(e[0], e[1] * e[1])).sqrt();
                if denominator.abs() <= ON_EDGE * scale {
                    // Parallel: where it runs along the segment, its ends cut.
                    if cross(w, d).abs() / squared.sqrt() <= COLLINEAR_METRES {
                        for end in [p, q] {
                            at.push(((end[0] - a[0]) * d[0] + (end[1] - a[1]) * d[1]) / squared);
                        }
                    }
                    continue;
                }
                let t = cross(w, e) / denominator;
                let s = cross(w, d) / denominator;
                if (-ON_EDGE..=1.0 + ON_EDGE).contains(&s) {
                    at.push(t);
                }
            }
        }
        at.retain(|t| (0.0..=1.0).contains(t));
        at.sort_by(f64::total_cmp);
        at.dedup_by(|later, earlier| *later - *earlier <= ON_EDGE);
        if at.last().is_some_and(|last| *last < 1.0) {
            at.push(1.0);
        }
        at.windows(2)
            .filter(|pair| pair[1] > pair[0])
            .map(|pair| (pair[0], pair[1]))
            .collect()
    }
}

fn cross(u: [f64; 2], v: [f64; 2]) -> f64 {
    u[0].mul_add(v[1], -(u[1] * v[0]))
}

/// Twice the signed plan area of `triangle`.
fn plan_area(points: &[[f64; 3]], triangle: [u32; 3]) -> f64 {
    let [a, b, c] = triangle.map(|index| points[index as usize]);
    cross([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]])
}

/// The barycentric weights of `point` in `triangle`'s plan, whatever side
/// it lies on.
fn plane_weights(points: &[[f64; 3]], triangle: [u32; 3], point: [f64; 2]) -> [f64; 3] {
    let [a, b, c] = triangle.map(|index| points[index as usize]);
    let whole = plan_area(points, triangle);
    let part = |p: [f64; 3], q: [f64; 3]| {
        cross(
            [p[0] - point[0], p[1] - point[1]],
            [q[0] - point[0], q[1] - point[1]],
        )
    };
    [part(b, c) / whole, part(c, a) / whole, part(a, b) / whole]
}

/// The barycentric weights of `point` in `triangle`, when it holds the
/// point (its edges and corners included).
fn barycentric(points: &[[f64; 3]], triangle: [u32; 3], point: [f64; 2]) -> Option<[f64; 3]> {
    let weights = plane_weights(points, triangle, point);
    weights
        .iter()
        .all(|weight| *weight >= -ON_EDGE)
        .then_some(weights)
}

/// A uniform plan grid over the surface's bounding box, each cell listing
/// the triangles whose plan bounding boxes meet it.
#[derive(Clone, Debug)]
struct Grid {
    low: [f64; 2],
    cell: f64,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(points: &[[f64; 3]], triangles: &[[u32; 3]]) -> Self {
        let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for triangle in triangles {
            for index in triangle {
                let point = points[*index as usize];
                for axis in 0..2 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
        }
        let (width, height) = ((high[0] - low[0]).max(0.0), (high[1] - low[1]).max(0.0));
        #[allow(clippy::cast_precision_loss)]
        let target = (triangles.len() as f64 / 2.0).clamp(1.0, 1_000_000.0);
        let cell = (width.max(f64::MIN_POSITIVE) * height.max(f64::MIN_POSITIVE) / target)
            .sqrt()
            .max(width.max(height) / 4096.0)
            .max(1e-9);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let count = |extent: f64| ((extent / cell).floor() as usize + 1).min(4096);
        let (columns, rows) = (count(width), count(height));
        let mut cells = vec![Vec::new(); columns * rows];
        let mut grid = Self {
            low,
            cell,
            columns,
            rows,
            cells: Vec::new(),
        };
        for (index, triangle) in triangles.iter().enumerate() {
            let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for corner in triangle {
                let point = points[*corner as usize];
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            let (c0, r0) = grid.cell_of(min);
            let (c1, r1) = grid.cell_of(max);
            #[allow(clippy::cast_possible_truncation)]
            for row in r0..=r1 {
                for column in c0..=c1 {
                    cells[row * columns + column].push(index as u32);
                }
            }
        }
        grid.cells = cells;
        grid
    }

    /// The cell holding `point`, clamped to the grid.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn cell_of(&self, point: [f64; 2]) -> (usize, usize) {
        let index = |value: f64, low: f64, count: usize| {
            (((value - low) / self.cell).floor().max(0.0) as usize).min(count - 1)
        };
        (
            index(point[0], self.low[0], self.columns),
            index(point[1], self.low[1], self.rows),
        )
    }

    /// Whether `point` lies within the grid's box, grown by a cell.
    fn holds(&self, point: [f64; 2]) -> bool {
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (
            self.columns as f64 * self.cell,
            self.rows as f64 * self.cell,
        );
        point[0] >= self.low[0] - self.cell
            && point[1] >= self.low[1] - self.cell
            && point[0] <= self.low[0] + width + self.cell
            && point[1] <= self.low[1] + height + self.cell
    }

    /// The triangles that may hold `point`: those of its cell and, on a
    /// cell's border, its neighbours'.
    fn at(&self, point: [f64; 2]) -> &[u32] {
        if !self.holds(point) {
            return &[];
        }
        let (column, row) = self.cell_of(point);
        &self.cells[row * self.columns + column]
    }

    /// Every triangle whose cell meets the segment `a`–`b`'s bounding box,
    /// grown by a cell, sorted and once.
    #[allow(clippy::many_single_char_names, clippy::similar_names)]
    fn along(&self, a: [f64; 2], b: [f64; 2]) -> Vec<u32> {
        let min = [a[0].min(b[0]) - self.cell, a[1].min(b[1]) - self.cell];
        let max = [a[0].max(b[0]) + self.cell, a[1].max(b[1]) + self.cell];
        let (c0, r0) = self.cell_of(min);
        let (c1, r1) = self.cell_of(max);
        let mut found: Vec<u32> = Vec::new();
        // A long diagonal segment is walked in pieces a few cells long, so
        // its cells, not its whole bounding box, are visited.
        let span = (b[0] - a[0]).hypot(b[1] - a[1]);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pieces = ((span / (4.0 * self.cell)).ceil() as usize).clamp(1, 1 << 20);
        if (c1 - c0 + 1) * (r1 - r0 + 1) <= 64 || pieces == 1 {
            for row in r0..=r1 {
                for column in c0..=c1 {
                    found.extend(&self.cells[row * self.columns + column]);
                }
            }
        } else {
            #[allow(clippy::cast_precision_loss)]
            for piece in 0..pieces {
                let (s, e) = (
                    piece as f64 / pieces as f64,
                    (piece + 1) as f64 / pieces as f64,
                );
                let p = [
                    (b[0] - a[0]).mul_add(s, a[0]),
                    (b[1] - a[1]).mul_add(s, a[1]),
                ];
                let q = [
                    (b[0] - a[0]).mul_add(e, a[0]),
                    (b[1] - a[1]).mul_add(e, a[1]),
                ];
                let (pc0, pr0) =
                    self.cell_of([p[0].min(q[0]) - self.cell, p[1].min(q[1]) - self.cell]);
                let (pc1, pr1) =
                    self.cell_of([p[0].max(q[0]) + self.cell, p[1].max(q[1]) + self.cell]);
                for row in pr0..=pr1 {
                    for column in pc0..=pc1 {
                        found.extend(&self.cells[row * self.columns + column]);
                    }
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// A terrain service over surfaces held in memory, at most one of each
/// kind.
#[derive(Clone, Debug)]
pub struct TinTerrain {
    surfaces: BTreeMap<TerrainKind, TerrainSurface>,
}

impl TinTerrain {
    /// A service over `surfaces`; two of one kind are refused, never one
    /// picked.
    pub fn try_new(surfaces: Vec<TerrainSurface>) -> Result<Self, TerrainError> {
        let mut held = BTreeMap::new();
        for surface in surfaces {
            let kind = surface.kind();
            if held.insert(kind, surface).is_some() {
                return Err(TerrainError::Invalid(format!(
                    "two {kind} terrain surfaces are supplied"
                )));
            }
        }
        Ok(Self { surfaces: held })
    }

    /// The surface of `kind`, if supplied.
    #[must_use]
    pub fn surface(&self, kind: TerrainKind) -> Option<&TerrainSurface> {
        self.surfaces.get(&kind)
    }

    fn held(&self, kind: TerrainKind) -> Result<&TerrainSurface, TerrainError> {
        self.surfaces
            .get(&kind)
            .ok_or(TerrainError::NotSupplied(kind))
    }
}

/// `[value - margin, value + margin]` rounded outward.
fn widened(value: f64, margin: f64) -> (f64, f64) {
    ((value - margin).next_down(), (value + margin).next_up())
}

impl TerrainService for TinTerrain {
    fn surfaces(&self) -> Vec<TerrainSurfaceStatement> {
        self.surfaces
            .values()
            .map(TerrainSurface::statement)
            .collect()
    }

    fn measure_elevation(
        &self,
        request: &TerrainPointRequest,
    ) -> Result<TerrainElevation, TerrainError> {
        let surface = self.held(request.kind())?;
        let (elevation, slack) = surface.elevation_at(request.point())?;
        let accuracy = surface.accuracy_metres();
        let point = request.point();
        TerrainElevation::try_new(
            surface.kind(),
            widened(elevation, accuracy + slack),
            accuracy,
            accuracy == 0.0,
            format!("{};at={},{}", surface.locator(), point[0], point[1]),
        )
    }

    fn measure_mean_elevation(
        &self,
        request: &TerrainOutlineRequest,
    ) -> Result<TerrainElevation, TerrainError> {
        let surface = self.held(request.kind())?;
        let (mean, slack, steepest) = surface.mean_along(request.rings())?;
        let accuracy = surface.accuracy_metres();
        let deviation = request.deviation_metres();
        // A true outline within `deviation` of the stated one in plan meets
        // ground at most that far times the steepest gradient higher or
        // lower.
        let shift = deviation * steepest;
        TerrainElevation::try_new(
            surface.kind(),
            widened(mean, accuracy + slack + shift),
            accuracy,
            accuracy == 0.0 && deviation == 0.0,
            format!("{};mean-along={}", surface.locator(), request.rings().len()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::TerrainServiceHandle;
    use std::sync::Arc;

    /// A plane rising 0.1 m per metre along x from 100 m at x 0, over
    /// x, y in [-50, 50], split into a grid of `n` by `n` squares.
    fn sloped(n: u32, accuracy: f64) -> TerrainSurface {
        let mut points = Vec::new();
        #[allow(clippy::cast_precision_loss)]
        for row in 0..=n {
            for column in 0..=n {
                let x = -50.0 + 100.0 * f64::from(column) / f64::from(n);
                let y = -50.0 + 100.0 * f64::from(row) / f64::from(n);
                points.push([x, y, 0.1f64.mul_add(x, 100.0)]);
            }
        }
        let mut triangles = Vec::new();
        for row in 0..n {
            for column in 0..n {
                let at = |r: u32, c: u32| r * (n + 1) + c;
                triangles.push([
                    at(row, column),
                    at(row, column + 1),
                    at(row + 1, column + 1),
                ]);
                triangles.push([
                    at(row, column),
                    at(row + 1, column + 1),
                    at(row + 1, column),
                ]);
            }
        }
        TerrainSurface::try_new(
            TerrainKind::Original,
            "ground",
            TerrainFrame::Model,
            None,
            accuracy,
            ("ground.xml", "ab"),
            &points,
            triangles,
        )
        .unwrap()
    }

    fn service(surface: TerrainSurface) -> TerrainServiceHandle {
        TerrainServiceHandle::new(Arc::new(TinTerrain::try_new(vec![surface]).unwrap()))
    }

    #[test]
    fn the_elevation_at_a_point_carries_the_stated_accuracy() {
        let terrain = service(sloped(4, 0.05));
        for point in [
            [0.0, 0.0],
            [12.5, -7.0],
            [25.0, 25.0],
            [50.0, 50.0],
            [-50.0, 3.0],
        ] {
            let request = TerrainPointRequest::try_new(TerrainKind::Original, point).unwrap();
            let elevation = terrain.measure_elevation(&request).unwrap();
            let expected = 0.1f64.mul_add(point[0], 100.0);
            assert!(elevation.lower_metres() <= expected - 0.05, "{elevation:?}");
            assert!(elevation.upper_metres() >= expected + 0.05, "{elevation:?}");
            assert!(elevation.upper_metres() - elevation.lower_metres() < 0.1 + 1e-9);
            assert!(!elevation.is_exact());
            assert!(
                elevation
                    .locator()
                    .starts_with("axioval:terrain;kind=original;sha256=ab#surface=ground"),
                "{}",
                elevation.locator()
            );
        }
    }

    #[test]
    fn a_stated_accuracy_of_zero_answers_exactly() {
        let terrain = service(sloped(2, 0.0));
        let request = TerrainPointRequest::try_new(TerrainKind::Original, [10.0, 0.0]).unwrap();
        let elevation = terrain.measure_elevation(&request).unwrap();
        assert!(elevation.is_exact());
        assert!((elevation.lower_metres() - 101.0).abs() < 1e-9);
        assert!((elevation.upper_metres() - 101.0).abs() < 1e-9);
    }

    #[test]
    fn a_point_outside_the_surface_is_refused_by_name() {
        let terrain = service(sloped(4, 0.05));
        let request = TerrainPointRequest::try_new(TerrainKind::Original, [60.0, 0.0]).unwrap();
        let refused = terrain.measure_elevation(&request).unwrap_err();
        assert_eq!(
            refused,
            TerrainError::Outside {
                kind: TerrainKind::Original,
                surface: "ground".into(),
                point: [60.0, 0.0],
            }
        );
        assert_eq!(
            refused.to_string(),
            "the plan point (60, 0) lies outside the original terrain surface `ground`"
        );
        let finished = TerrainPointRequest::try_new(TerrainKind::Finished, [0.0, 0.0]).unwrap();
        assert_eq!(
            terrain.measure_elevation(&finished),
            Err(TerrainError::NotSupplied(TerrainKind::Finished))
        );
    }

    #[test]
    fn a_surface_folding_over_a_point_is_refused() {
        let points = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 5.0],
            [10.0, 0.0, 5.0],
            [0.0, 10.0, 5.0],
        ];
        let surface = TerrainSurface::try_new(
            TerrainKind::Finished,
            "folded",
            TerrainFrame::Model,
            None,
            0.1,
            ("f.xml", "cd"),
            &points,
            vec![[0, 1, 2], [3, 4, 5]],
        )
        .unwrap();
        let request = TerrainPointRequest::try_new(TerrainKind::Finished, [1.0, 1.0]).unwrap();
        assert!(matches!(
            service(surface).measure_elevation(&request),
            Err(TerrainError::Folded { .. })
        ));
    }

    #[test]
    fn the_mean_along_a_ring_is_exact_for_the_surface_and_widened_by_the_accuracy() {
        let terrain = service(sloped(7, 0.05));
        // A 10 m square from x 0 to 10: the mean of x along its outline is
        // 5, so the mean elevation is 100.5.
        let square = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]];
        let request =
            TerrainOutlineRequest::try_new(TerrainKind::Original, square.clone(), 0.0).unwrap();
        let mean = terrain.measure_mean_elevation(&request).unwrap();
        assert!(
            mean.lower_metres() <= 100.45 && mean.lower_metres() > 100.45 - 1e-7,
            "{mean:?}"
        );
        assert!(
            mean.upper_metres() >= 100.55 && mean.upper_metres() < 100.55 + 1e-7,
            "{mean:?}"
        );
        // An L-shape weights each edge by its length.
        let ell = vec![vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 5.0],
            [5.0, 5.0],
            [5.0, 20.0],
            [0.0, 20.0],
        ]];
        let request = TerrainOutlineRequest::try_new(TerrainKind::Original, ell, 0.0).unwrap();
        let mean = terrain.measure_mean_elevation(&request).unwrap();
        // Edges: 20 at mean x 10, 5 at x 20, 15 at x 12.5, 15 at x 5,
        // 5 at x 2.5, 20 at x 0, over 80 m.
        let x = (10.0 * 20.0 + 20.0 * 5.0 + 12.5 * 15.0 + 5.0 * 15.0 + 2.5 * 5.0) / 80.0;
        let expected = 0.1f64.mul_add(x, 100.0);
        assert!((f64::midpoint(mean.lower_metres(), mean.upper_metres()) - expected).abs() < 1e-9);
        // A tessellated outline widens it by its deviation times the slope.
        let request = TerrainOutlineRequest::try_new(TerrainKind::Original, square, 0.2).unwrap();
        let mean = terrain.measure_mean_elevation(&request).unwrap();
        assert!(mean.upper_metres() - mean.lower_metres() >= 0.1 + 2.0 * 0.02 - 1e-9);
    }

    #[test]
    fn an_outline_reaching_past_the_surface_is_refused_by_a_point_outside() {
        let terrain = service(sloped(4, 0.05));
        let ring = vec![vec![[40.0, 0.0], [60.0, 0.0], [60.0, 10.0], [40.0, 10.0]]];
        let request = TerrainOutlineRequest::try_new(TerrainKind::Original, ring, 0.0).unwrap();
        let refused = terrain.measure_mean_elevation(&request).unwrap_err();
        let TerrainError::Outside { point, .. } = refused else {
            panic!("{refused:?}");
        };
        assert!(point[0] > 50.0, "{point:?}");
    }

    #[test]
    fn a_surface_needs_a_triangle_with_plan_area_and_an_accuracy() {
        let points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]];
        let surface = |accuracy, triangles: Vec<[u32; 3]>| {
            TerrainSurface::try_new(
                TerrainKind::Original,
                "s",
                TerrainFrame::Model,
                None,
                accuracy,
                ("s.xml", "ef"),
                &points,
                triangles,
            )
        };
        assert!(matches!(
            surface(0.1, vec![[0, 1, 2]]),
            Err(TerrainError::Invalid(_))
        ));
        assert!(matches!(
            surface(0.1, vec![[0, 1, 3]]),
            Err(TerrainError::Invalid(_))
        ));
        assert!(matches!(
            surface(f64::NAN, vec![[0, 1, 2]]),
            Err(TerrainError::Invalid(_))
        ));
        assert!(matches!(
            TinTerrain::try_new(vec![sloped(1, 0.1), sloped(1, 0.2)]),
            Err(TerrainError::Invalid(_))
        ));
    }
}
