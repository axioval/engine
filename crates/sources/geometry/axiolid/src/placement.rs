//! Exact placement search in a scope's configuration space.
//!
//! A shape fits at centre `p` exactly when it lies in the scope footprint and
//! meets no obstacle footprint in its height band. The set of such centres is
//! the configuration space: the scope eroded by the shape, minus every
//! obstacle dilated by the reflected shape. A point in it is a witness; an
//! empty one proves that no placement exists. Both Minkowski steps are
//! Axiolid's; this module only composes them and decides which verdict each
//! result can support.
//!
//! - A rectangle at a fixed orientation is convex, so both steps are exact up
//!   to one rounding of each vertex. A witness is re-verified by direct
//!   overlap, and "no placement" also needs the rectangle shrunk by
//!   [`KNIFE_EDGE_METRES`] to fit nowhere, so rounding cannot turn a
//!   knife-edge fit into a false proof.
//! - A circle uses one-sided disc approximations: a region contained in the
//!   exact one can only prove a witness, a region containing it can only prove
//!   absence. Between the two the search refuses.
//! - A rectangle at any orientation searches angle intervals. A fixed check at
//!   an interval's middle can find a witness, and the middle rectangle shrunk
//!   by how far a rotation within the interval moves any corner can prove the
//!   whole interval empty. Undecided intervals are split until a budget runs
//!   out, and then the search refuses.
//!
//! A reach from entrances limits the centre to where the shape meets the
//! pieces of a path from an entrance: a witness meets the surely reached
//! pieces (re-verified by overlap with the shape, or with its inscribed
//! polygon for a circle), and an absence misses the possibly reached ones
//! even for the shape grown by [`KNIFE_EDGE_METRES`].
//!
//! A frame-offset domain limits the centre to a box of offsets in an anchor
//! frame (a [`Window`]). The configuration space is intersected with that
//! box: witnesses come from the box as computed and must also pass the
//! contract's own domain test, and proofs of absence use the box grown by
//! [`KNIFE_EDGE_METRES`], so rounding its corners cannot hide a centre on
//! its edge. For a fixed orientation this is exact up to the same margins.

use std::cell::OnceCell;
use std::f64::consts::PI;

use axiolid_core::{Point2, Tolerance};
use axiolid_overlay::{Polygon, Region, Ring, union_soup};
use axioval_engine::FreeSpaceError;

use crate::geometry::Triangle;
use crate::planar::{projected_polygons, refused_ring};

/// Fits closer than this are not told apart from misses when proving that no
/// placement exists. Such a verdict refuses instead.
pub(crate) const KNIFE_EDGE_METRES: f64 = 1.0e-6;

/// Overlap areas below this are contact, not intersection.
const CONTACT_AREA_M2: f64 = 1.0e-9;

/// Fixed-orientation checks an any-orientation search may spend.
const ANGLE_BUDGET: usize = 512;

/// Sides of the polygon that verifies a circle witness.
const VERIFY_SIDES: u32 = 64;

fn unavailable(context: &str, error: impl std::fmt::Debug) -> FreeSpaceError {
    FreeSpaceError::Unavailable(format!("{context}: {error:?}"))
}

/// How far a shadow the overlay refuses is grown before it joins an outer
/// footprint: far above its linear tolerance and the rounding of the box's
/// corners, far below [`KNIFE_EDGE_METRES`].
const SLIVER_MARGIN_METRES: f64 = 1.0e-8;

/// The plan footprint of a triangle set, bracketed around the shadows the
/// overlay refuses (#377): `(inner, outer)`, `inner` inside the union of
/// every projected triangle and `outer` around it.
///
/// A face standing upright up to rounding (a wall whose top corners lie
/// some ulps off its bottom ones in plan) projects to a sliver whose short
/// side is within the overlay's linear tolerance. The overlay refuses such
/// a ring (`RepeatedVertex`), and its corners are not one point, so no
/// exact cleaning removes it. `inner` leaves those slivers out, which only
/// shrinks it; `outer` takes each as a box around it, the sliver's longest
/// edge for one side and grown by [`SLIVER_MARGIN_METRES`] all round, which
/// only grows it. Without such shadows both are the one union of every
/// projected triangle.
pub(crate) fn bracketed_footprint(
    triangles: &[Triangle],
    tolerance: Tolerance,
) -> Result<(Region, Region), FreeSpaceError> {
    let (kept, slivers): (Vec<Ring>, Vec<Ring>) = projected_polygons(triangles)
        .into_iter()
        .map(|polygon| polygon.outer)
        .partition(|ring| !refused_ring(ring, tolerance));
    let inner = union(&kept, tolerance)?;
    if slivers.is_empty() {
        return Ok((inner.clone(), inner));
    }
    let mut grown = kept;
    grown.extend(slivers.iter().map(sliver_box));
    Ok((inner, union(&grown, tolerance)?))
}

/// A counter-clockwise rectangle holding a projected triangle: along its
/// longest edge, grown by [`SLIVER_MARGIN_METRES`] (or by the rounding of
/// its coordinates where that is larger) on every side, so the rounding of
/// its corners cannot leave a point of the triangle outside.
pub(crate) fn sliver_box(ring: &Ring) -> Ring {
    let points = &ring.points;
    let longest = (0..points.len())
        .max_by(|&a, &b| {
            let length = |i: usize| (points[(i + 1) % points.len()] - points[i]).length();
            length(a).total_cmp(&length(b))
        })
        .unwrap_or(0);
    let origin = points[longest];
    let edge = points[(longest + 1) % points.len()] - origin;
    let length = edge.length();
    // A sliver of no length at all is a point: an axis box holds it.
    let along = if length > 0.0 {
        edge / length
    } else {
        Point2::X
    };
    let across = along.perp();
    let magnitude = points
        .iter()
        .flat_map(|point| [point.x.abs(), point.y.abs()])
        .fold(0.0, f64::max);
    let margin = SLIVER_MARGIN_METRES.max(64.0 * f64::EPSILON * magnitude);
    let (mut s, mut t) = (
        (f64::INFINITY, f64::NEG_INFINITY),
        (f64::INFINITY, f64::NEG_INFINITY),
    );
    for point in points {
        let offset = *point - origin;
        let (a, b) = (offset.dot(along), offset.dot(across));
        s = (s.0.min(a), s.1.max(a));
        t = (t.0.min(b), t.1.max(b));
    }
    let (s, t) = ((s.0 - margin, s.1 + margin), (t.0 - margin, t.1 + margin));
    let corner = |a: f64, b: f64| origin + along * a + across * b;
    Ring {
        points: vec![
            corner(s.0, t.0),
            corner(s.1, t.0),
            corner(s.1, t.1),
            corner(s.0, t.1),
        ],
    }
}

/// The union of counter-clockwise rings, as a validated region.
pub(crate) fn union(rings: &[Ring], tolerance: Tolerance) -> Result<Region, FreeSpaceError> {
    if rings.is_empty() {
        return Ok(Region::empty());
    }
    let polygons = union_soup(rings, tolerance).map_err(|e| unavailable("footprint", e))?;
    Region::new(polygons, tolerance).map_err(|e| unavailable("footprint", e))
}

/// A search scene: where a centre may lie, and what it must avoid.
///
/// Both are bracketed. `scope` lies inside the scope's true footprint and
/// `room` contains it; `obstacles` contains what the obstacles occupy and
/// `sure` lies inside it. They differ by swept door sectors, whose
/// footprints are polygonised from both sides, and by tessellated bodies,
/// grown and shrunk by their chord deviation. A witness must lie in
/// `scope` and avoid `obstacles`; an absence is proven in `room` against
/// `sure`.
pub(crate) struct Scene {
    pub(crate) scope: Region,
    pub(crate) room: Region,
    pub(crate) obstacles: Region,
    pub(crate) sure: Region,
    pub(crate) tolerance: Tolerance,
    /// The frame-offset box the centre must also lie in, if any.
    pub(crate) window: Option<Window>,
    /// Where a path from the entrances runs, if the shape must be reached.
    pub(crate) reach: Option<Reached>,
    /// The tessellated bodies bracketed by their chord deviation, with it.
    pub(crate) bracketed: Vec<(axioval_ir::ObjectId, f64)>,
    /// `scope` less `obstacles` (`[0]`) and `room` less `sure` (`[1]`),
    /// each taken once a search first erodes it: every shape and angle
    /// erodes the same difference ([`Self::free_for`]).
    pub(crate) clear: [OnceCell<Region>; 2],
}

/// The pieces of the free area eroded by half a path's width that come
/// near an entrance: `inner` inside the exact ones that surely do (a path
/// runs from the entrance into any shape meeting it), `outer` containing
/// every one that may (a shape missing it is not reached).
pub(crate) struct Reached {
    pub(crate) inner: Region,
    pub(crate) outer: Region,
}

/// A box of centres in an anchor frame: offsets along its right axis in
/// `across` and along its forward axis in `along`.
pub(crate) struct Window {
    pub(crate) origin: Point2,
    pub(crate) right: Axis,
    pub(crate) forward: Axis,
    pub(crate) across: (f64, f64),
    pub(crate) along: (f64, f64),
    /// The contract's own domain test for a centre, so that no witness is
    /// offered that the evidence would reject.
    pub(crate) admits: Box<dyn Fn(Point2) -> bool>,
}

impl Window {
    fn at(&self, a: f64, b: f64) -> Point2 {
        Point2::new(
            self.origin.x + a * self.right.x + b * self.forward.x,
            self.origin.y + a * self.right.y + b * self.forward.y,
        )
    }

    /// The box grown by `grow` on every side, or `None` when it has no area.
    fn region(&self, grow: f64, tolerance: Tolerance) -> Result<Option<Region>, FreeSpaceError> {
        let (a0, a1) = (self.across.0 - grow, self.across.1 + grow);
        let (b0, b1) = (self.along.0 - grow, self.along.1 + grow);
        if a1 - a0 <= KNIFE_EDGE_METRES || b1 - b0 <= KNIFE_EDGE_METRES {
            return Ok(None);
        }
        let mut points = vec![
            self.at(a0, b0),
            self.at(a1, b0),
            self.at(a1, b1),
            self.at(a0, b1),
        ];
        // Counter-clockwise needs forward to the left of right.
        if self.right.x * self.forward.y - self.right.y * self.forward.x < 0.0 {
            points.reverse();
        }
        Region::new(
            vec![Polygon {
                outer: Ring { points },
                holes: Vec::new(),
            }],
            tolerance,
        )
        .map(Some)
        .map_err(|e| unavailable("offset box", e))
    }

    /// The point of the box nearest `p` in offsets.
    fn clamp(&self, p: Point2) -> Point2 {
        let (dx, dy) = (p.x - self.origin.x, p.y - self.origin.y);
        let a = (dx * self.right.x + dy * self.right.y).clamp(self.across.0, self.across.1);
        let b = (dx * self.forward.x + dy * self.forward.y).clamp(self.along.0, self.along.1);
        self.at(a, b)
    }
}

/// A plan unit vector.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Axis {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl Axis {
    pub(crate) fn at(angle: f64) -> Self {
        Self {
            x: angle.cos(),
            y: angle.sin(),
        }
    }
    /// The axis a quarter turn counter-clockwise.
    pub(crate) fn left(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }
}

/// A `width` × `depth` rectangle centred on `centre`, width along `right` and
/// depth along `right.left()`, counter-clockwise.
pub(crate) fn rectangle(centre: Point2, width: f64, depth: f64, right: Axis) -> Ring {
    let forward = right.left();
    let (w, d) = (width / 2.0, depth / 2.0);
    let corner = |a: f64, b: f64| {
        Point2::new(
            centre.x + a * right.x + b * forward.x,
            centre.y + a * right.y + b * forward.y,
        )
    };
    Ring {
        points: vec![corner(-w, -d), corner(w, -d), corner(w, d), corner(-w, d)],
    }
}

/// A regular polygon inside a circle of `radius`, its vertices on it.
fn inscribed(centre: Point2, radius: f64) -> Ring {
    let sides = f64::from(VERIFY_SIDES);
    Ring {
        points: (0..VERIFY_SIDES)
            .map(|i| {
                let angle = 2.0 * PI * f64::from(i) / sides;
                Point2::new(
                    centre.x + radius * angle.cos(),
                    centre.y + radius * angle.sin(),
                )
            })
            .collect(),
    }
}

/// A regular polygon around a circle of `radius`, so the circle lies inside.
fn circumscribed(centre: Point2, radius: f64) -> Ring {
    let sides = f64::from(VERIFY_SIDES);
    let outer = radius / (PI / sides).cos();
    Ring {
        points: (0..VERIFY_SIDES)
            .map(|i| {
                let angle = 2.0 * PI * f64::from(i) / sides;
                Point2::new(
                    centre.x + outer * angle.cos(),
                    centre.y + outer * angle.sin(),
                )
            })
            .collect(),
    }
}

impl Scene {
    /// Centres where the convex, centrally symmetric `shape` (centred on the
    /// origin) fits. Symmetry makes the reflected shape the shape itself.
    /// A witness is sought among the centres clear of every possible
    /// obstacle; an absence (`absence`) is proven among those clear of the
    /// sure ones.
    ///
    /// The centres are the scope eroded by the shape less the obstacles
    /// summed with it. An obstacle region of more than
    /// [`OBSTACLE_SUM_POINTS`] corners (a floor's walls united into one
    /// polygon with a hole per room) is not summed: that one arrangement
    /// outgrew 12 GB on a large floor of m14 (#359). The scope less the
    /// obstacles is eroded by the shape instead, the same centres but for
    /// those where the shape only touches an obstacle, which it admits: a
    /// witness from them is still verified by direct overlap
    /// ([`Self::fits`]), and an absence proven where they are empty holds
    /// all the more.
    fn free_for(&self, shape: &Ring, absence: bool) -> Result<Region, FreeSpaceError> {
        let (scope, obstacles) = if absence {
            (&self.room, &self.sure)
        } else {
            (&self.scope, &self.obstacles)
        };
        if region_points(obstacles) > OBSTACLE_SUM_POINTS {
            return self
                .clear(absence)?
                .minkowski_erosion(shape, self.tolerance)
                .map_err(|e| unavailable("erosion", e));
        }
        summed_free(scope, obstacles, shape, self.tolerance)
    }

    /// The scope less the obstacles ([`clear_of`]): `scope` less
    /// `obstacles` for a witness, `room` less `sure` for an absence
    /// (`absence`). Taken once per scene; a refusal is not kept, so each
    /// search asking for it is refused alike.
    fn clear(&self, absence: bool) -> Result<&Region, FreeSpaceError> {
        let (scope, obstacles) = if absence {
            (&self.room, &self.sure)
        } else {
            (&self.scope, &self.obstacles)
        };
        let cell = &self.clear[usize::from(absence)];
        if let Some(clear) = cell.get() {
            return Ok(clear);
        }
        let clear = clear_of(scope, obstacles, self.tolerance)?;
        Ok(cell.get_or_init(|| clear))
    }

    /// Whether no rotation of a centred rectangle whose half-sizes are at
    /// least `half` fits anywhere clear: `scope` less `obstacles` for a
    /// witness, `room` less `sure` for an absence (`absence`), eroded by a
    /// regular octagon inscribed in the circle of radius `half` less
    /// [`KNIFE_EDGE_METRES`], is empty. Every such rectangle holds that
    /// circle grown by the margin, so the centres it may take lie in the
    /// erosion shrunk by the margin, which rounding cannot empty. `false`
    /// where the erosion is refused or leaves anything: the angle search
    /// then decides.
    fn fits_nowhere(&self, absence: bool, half: f64) -> bool {
        let radius = (half - KNIFE_EDGE_METRES) * (1.0 - 1.0e-9);
        if radius <= 0.0 {
            return false;
        }
        let Ok(clear) = self.clear(absence) else {
            return false;
        };
        clear.is_empty()
            || clear
                .minkowski_erosion(&octagon(radius), self.tolerance)
                .is_ok_and(|free| free.is_empty())
    }

    /// Whether `shape` fits somewhere in the room clear of the sure
    /// obstacles: the room less them, eroded by it, is not empty. `false`
    /// where the erosion is refused or leaves nothing.
    fn fits_somewhere(&self, shape: &Ring) -> bool {
        let Ok(clear) = self.clear(true) else {
            return false;
        };
        !clear.is_empty()
            && clear
                .minkowski_erosion(shape, self.tolerance)
                .is_ok_and(|free| !free.is_empty())
    }
}

/// A regular octagon about the origin with its corners `corner` from it,
/// counter-clockwise.
fn octagon(corner: f64) -> Ring {
    Ring {
        points: (0..8)
            .map(|i| {
                let angle = PI * f64::from(i) / 4.0;
                Point2::new(corner * angle.cos(), corner * angle.sin())
            })
            .collect(),
    }
}

/// Obstacle regions with more corners than this are not summed with the
/// shape ([`Scene::free_for`]).
const OBSTACLE_SUM_POINTS: usize = 500;

/// The corners of every ring of `region`.
fn region_points(region: &Region) -> usize {
    region
        .polygons()
        .iter()
        .map(|polygon| {
            polygon.outer.points.len()
                + polygon
                    .holes
                    .iter()
                    .map(|hole| hole.points.len())
                    .sum::<usize>()
        })
        .sum()
}

/// `scope` eroded by `shape`, less `obstacles` summed with it.
fn summed_free(
    scope: &Region,
    obstacles: &Region,
    shape: &Ring,
    tolerance: Tolerance,
) -> Result<Region, FreeSpaceError> {
    {
        let room = scope
            .minkowski_erosion(shape, tolerance)
            .map_err(|e| unavailable("erosion", e))?;
        if room.is_empty() || obstacles.is_empty() {
            return Ok(room);
        }
        let blocked = obstacles
            .minkowski_sum(shape, tolerance)
            .map_err(|e| unavailable("dilation", e))?;
        room.difference(&blocked, tolerance)
            .map_err(|e| unavailable("difference", e))
    }
}

/// `scope` less `obstacles`, which [`eroded_free`] erodes.
fn clear_of(
    scope: &Region,
    obstacles: &Region,
    tolerance: Tolerance,
) -> Result<Region, FreeSpaceError> {
    if obstacles.is_empty() {
        return Ok(scope.clone());
    }
    scope
        .difference(obstacles, tolerance)
        .map_err(|e| unavailable("difference", e))
}

/// `scope` less `obstacles`, eroded by `shape`: [`summed_free`]'s centres
/// and those where the shape only touches an obstacle. A scene takes the
/// difference once and erodes it per shape ([`Scene::free_for`]).
#[cfg(test)]
fn eroded_free(
    scope: &Region,
    obstacles: &Region,
    shape: &Ring,
    tolerance: Tolerance,
) -> Result<Region, FreeSpaceError> {
    clear_of(scope, obstacles, tolerance)?
        .minkowski_erosion(shape, tolerance)
        .map_err(|e| unavailable("erosion", e))
}

impl Scene {
    /// The part of `free` a witness may come from: inside the offset box as
    /// computed. A box without area keeps `free`; its candidates are clamped
    /// onto the box instead.
    fn for_witness(&self, free: Region) -> Result<Region, FreeSpaceError> {
        match &self.window {
            None => Ok(free),
            Some(window) => match window.region(0.0, self.tolerance)? {
                Some(inside) if !free.is_empty() => free
                    .intersection(&inside, self.tolerance)
                    .map_err(|e| unavailable("offset box", e)),
                _ => Ok(free),
            },
        }
    }

    /// Whether no centre of `free` lies in the domain: `free` meets not even
    /// the offset box grown by the knife-edge margin.
    fn nowhere_in_domain(&self, free: &Region) -> Result<bool, FreeSpaceError> {
        if free.is_empty() {
            return Ok(true);
        }
        let Some(window) = &self.window else {
            return Ok(false);
        };
        let grown = window
            .region(KNIFE_EDGE_METRES, self.tolerance)?
            .ok_or_else(|| FreeSpaceError::Unavailable("offset box has no area".into()))?;
        Ok(free
            .intersection(&grown, self.tolerance)
            .map_err(|e| unavailable("offset box", e))?
            .is_empty())
    }

    /// The centres of `free` at which the convex, centrally symmetric
    /// `shape` meets the reached region: the inner one for a witness, the
    /// outer one for an absence (the caller grows the shape by the
    /// knife-edge margin). Without a reach, `free` itself.
    fn reaching(
        &self,
        free: Region,
        shape: &Ring,
        absence: bool,
    ) -> Result<Region, FreeSpaceError> {
        let Some(reach) = &self.reach else {
            return Ok(free);
        };
        let target = if absence { &reach.outer } else { &reach.inner };
        if target.is_empty() || free.is_empty() {
            return Ok(Region::empty());
        }
        let near = target
            .minkowski_sum(shape, self.tolerance)
            .map_err(|e| unavailable("reach", e))?;
        free.intersection(&near, self.tolerance)
            .map_err(|e| unavailable("reach", e))
    }

    /// As [`Self::reaching`] for a disc of `radius`: centres within it of
    /// the inner region (from inside) for a witness, of the outer one
    /// (from outside, grown by the knife-edge margin) for an absence.
    fn reaching_disc(
        &self,
        free: Region,
        radius: f64,
        absence: bool,
    ) -> Result<Region, FreeSpaceError> {
        let Some(reach) = &self.reach else {
            return Ok(free);
        };
        let target = if absence { &reach.outer } else { &reach.inner };
        if target.is_empty() || free.is_empty() {
            return Ok(Region::empty());
        }
        let near = if absence {
            target.dilate_outer(radius + KNIFE_EDGE_METRES, self.tolerance)
        } else {
            target.dilate_inner(radius, self.tolerance)
        }
        .map_err(|e| unavailable("reach", e))?;
        free.intersection(&near, self.tolerance)
            .map_err(|e| unavailable("reach", e))
    }

    /// Whether `ring`, inside the shape, shares area with the surely
    /// reached region; always without a reach.
    fn reaches(&self, ring: Ring) -> Result<bool, FreeSpaceError> {
        let Some(reach) = &self.reach else {
            return Ok(true);
        };
        if reach.inner.is_empty() {
            return Ok(false);
        }
        let body = Region::new(
            vec![Polygon {
                outer: ring,
                holes: Vec::new(),
            }],
            self.tolerance,
        )
        .map_err(|e| unavailable("reach", e))?;
        Ok(body
            .intersection(&reach.inner, self.tolerance)
            .map_err(|e| unavailable("reach", e))?
            .area()
            > CONTACT_AREA_M2)
    }

    /// Whether `ring` lies in the scope and meets no obstacle, up to contact.
    fn fits(&self, ring: Ring) -> Result<bool, FreeSpaceError> {
        let body = Region::new(
            vec![Polygon {
                outer: ring,
                holes: Vec::new(),
            }],
            self.tolerance,
        )
        .map_err(|e| unavailable("witness", e))?;
        let outside = body
            .difference(&self.scope, self.tolerance)
            .map_err(|e| unavailable("witness", e))?
            .area();
        if outside > CONTACT_AREA_M2 {
            return Ok(false);
        }
        let blocked = body
            .intersection(&self.obstacles, self.tolerance)
            .map_err(|e| unavailable("witness", e))?
            .area();
        Ok(blocked <= CONTACT_AREA_M2)
    }

    /// A centre in `free` at which `place` fits and `touch` (inside the
    /// shape) is reached, if any candidate does.
    fn witness(
        &self,
        free: &Region,
        place: impl Fn(Point2) -> Ring,
        touch: impl Fn(Point2) -> Ring,
    ) -> Result<Option<Point2>, FreeSpaceError> {
        let mut offered = candidates(free);
        if let Some(window) = &self.window {
            // Candidates of a box without area lie beside it; move them onto
            // it. Every candidate must pass the contract's domain test.
            offered = offered.into_iter().map(|c| window.clamp(c)).collect();
            offered.push(window.clamp(window.origin));
            offered.retain(|c| (window.admits)(*c));
        }
        for candidate in offered {
            if self.fits(place(candidate))? && self.reaches(touch(candidate))? {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }
}

/// Interior candidates of a region: each outer ring's centroid, then the
/// centroids of its fan triangles, then the middle of every interval in
/// which a line halfway between two vertex heights crosses the polygon.
/// Rounded boundary points are never offered.
///
/// The last kind reaches every polygon, holes included: between two
/// consecutive vertex heights a polygon is a union of trapezoids, and the
/// line through their middle crosses each of them.
fn candidates(region: &Region) -> Vec<Point2> {
    let mut out = centroids(region);
    for polygon in region.polygons() {
        let rings: Vec<&Ring> = std::iter::once(&polygon.outer)
            .chain(polygon.holes.iter())
            .collect();
        let mut heights: Vec<f64> = rings
            .iter()
            .flat_map(|ring| ring.points.iter().map(|p| p.y))
            .collect();
        heights.sort_by(f64::total_cmp);
        heights.dedup();
        for pair in heights.windows(2) {
            let y = f64::midpoint(pair[0], pair[1]);
            let mut crossings: Vec<f64> = Vec::new();
            for ring in &rings {
                let points = &ring.points;
                for i in 0..points.len() {
                    let (a, b) = (points[i], points[(i + 1) % points.len()]);
                    if (a.y < y) != (b.y < y) {
                        crossings.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                    }
                }
            }
            crossings.sort_by(f64::total_cmp);
            for inside in crossings.chunks_exact(2) {
                out.push(Point2::new(f64::midpoint(inside[0], inside[1]), y));
            }
        }
    }
    out
}

/// Each outer ring's centroid and the centroids of its fan triangles.
fn centroids(region: &Region) -> Vec<Point2> {
    let mut out = Vec::new();
    for polygon in region.polygons() {
        let points = &polygon.outer.points;
        if points.len() < 3 {
            continue;
        }
        let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            let cross = a.x * b.y - b.x * a.y;
            area += cross;
            cx += (a.x + b.x) * cross;
            cy += (a.y + b.y) * cross;
        }
        if area.abs() > 0.0 {
            out.push(Point2::new(cx / (3.0 * area), cy / (3.0 * area)));
        }
        for i in 1..points.len() - 1 {
            let (a, b, c) = (points[0], points[i], points[i + 1]);
            out.push(Point2::new(
                (a.x + b.x + c.x) / 3.0,
                (a.y + b.y + c.y) / 3.0,
            ));
        }
    }
    out
}

/// What a search established.
#[derive(Debug)]
pub(crate) enum Search {
    /// The shape fits with its centre here, at this angle of its width axis.
    Found { centre: Point2, right: Axis },
    /// No placement exists.
    Nowhere,
    /// Neither: no witness verifies and no absence is proven, for the
    /// reason given. Within a margin or a bracket, never a guess.
    Undecided(&'static str),
}

/// A rectangle whose width follows `right`.
pub(crate) fn fixed_rectangle(
    scene: &Scene,
    width: f64,
    depth: f64,
    right: Axis,
) -> Result<Search, FreeSpaceError> {
    let origin = Point2::new(0.0, 0.0);
    let shape = rectangle(origin, width, depth, right);
    let free =
        scene.for_witness(scene.reaching(scene.free_for(&shape, false)?, &shape, false)?)?;
    if !free.is_empty() || scene.window.is_some() {
        let place = |c| rectangle(c, width, depth, right);
        if let Some(centre) = scene.witness(&free, place, place)? {
            return Ok(Search::Found { centre, right });
        }
    }
    let (w, d) = (
        width - 2.0 * KNIFE_EDGE_METRES,
        depth - 2.0 * KNIFE_EDGE_METRES,
    );
    if w <= 0.0 || d <= 0.0 {
        return Err(FreeSpaceError::Unavailable(
            "the rectangle is too small to prove absent".into(),
        ));
    }
    let grown = rectangle(
        origin,
        width + 2.0 * KNIFE_EDGE_METRES,
        depth + 2.0 * KNIFE_EDGE_METRES,
        right,
    );
    let shrunk = scene.free_for(&rectangle(origin, w, d, right), true)?;
    if scene.nowhere_in_domain(&scene.reaching(shrunk, &grown, true)?)? {
        return Ok(Search::Nowhere);
    }
    Ok(Search::Undecided(if free.is_empty() {
        "the rectangle fits only within the knife-edge margin"
    } else {
        "no candidate in the free region verifies"
    }))
}

/// A rectangle at any rotation about the vertical.
///
/// The angle intervals are searched depth first, the higher half of a
/// split first, so an interval never proven empty is split towards the end
/// of the period until the budget runs out. After the first interval, taken
/// as it always was, two proofs over every angle at once may end the
/// search with the verdict it would reach: every interval proven empty
/// ([`Scene::fits_nowhere`]), or no witness at any angle while the
/// rectangle at the period's end, grown by [`KNIFE_EDGE_METRES`], fits
/// where no absence can exclude it, so that the descent towards it spends
/// the budget ([`Scene::fits_somewhere`]). Both need no reach, which
/// narrows each interval's centres apart. Later intervals take the absence
/// first: where it is proven, the witness region, which lies inside it, is
/// empty too.
pub(crate) fn any_rectangle(
    scene: &Scene,
    width: f64,
    depth: f64,
) -> Result<Search, FreeSpaceError> {
    // A rectangle repeats every half turn, a square every quarter turn.
    let period = if (width - depth).abs() <= f64::EPSILON * width.max(depth) {
        PI / 2.0
    } else {
        PI
    };
    if scene.window.is_some() {
        // Offset witnesses keep the anchor's axes; the contract refuses a box
        // at any orientation in such a domain.
        return Err(FreeSpaceError::OrientationDomainConflict);
    }
    let half_diagonal = width.hypot(depth) / 2.0;
    let origin = Point2::new(0.0, 0.0);
    let mut open: Vec<(f64, f64)> = (0..8)
        .map(|i| (period * f64::from(i) / 8.0, period * f64::from(i + 1) / 8.0))
        .collect();
    // Turning by at most half an interval moves any point of the rectangle
    // by at most the half-diagonal times that angle, so a rectangle that
    // deep inside the middle one is inside every rotation.
    let shrink_by =
        |(low, high): (f64, f64)| half_diagonal * (high - low) / 2.0 + KNIFE_EDGE_METRES;
    let first_shrink = shrink_by(open[0]);
    // Whether no witness region holds anything at any angle; asked after
    // the first interval, as the proofs over every angle.
    let mut witnessless = false;
    let mut spent = 0usize;
    while let Some((low, high)) = open.pop() {
        if spent >= ANGLE_BUDGET {
            return Ok(Search::Undecided(
                "orientation search budget exhausted before a verdict",
            ));
        }
        if spent == 1 && scene.reach.is_none() {
            // Every first interval's shrunk rectangle holds the circle of
            // its smaller half-size, so where that fits nowhere, each is
            // proven empty.
            let (w, d) = (width - 2.0 * first_shrink, depth - 2.0 * first_shrink);
            if w > 0.0 && d > 0.0 && scene.fits_nowhere(true, w.min(d) / 2.0) {
                return Ok(Search::Nowhere);
            }
            witnessless = scene.fits_nowhere(false, width.min(depth) / 2.0);
            // Every shrunk rectangle of an interval reaching the period's
            // end lies inside the rectangle there shrunk by the margin;
            // where that, grown by the margin, fits clear of the sure
            // obstacles, no such interval is ever proven empty.
            let end = rectangle(
                origin,
                width + 2.0 * KNIFE_EDGE_METRES,
                depth + 2.0 * KNIFE_EDGE_METRES,
                Axis::at(period),
            );
            if witnessless && scene.fits_somewhere(&end) {
                return Ok(Search::Undecided(
                    "orientation search budget exhausted before a verdict",
                ));
            }
        }
        spent += 1;
        let middle = f64::midpoint(low, high);
        let right = Axis::at(middle);
        let shrink = shrink_by((low, high));
        let (w, d) = (width - 2.0 * shrink, depth - 2.0 * shrink);
        // Every rotation within the interval lies inside the middle
        // rectangle grown by the shrink, so it meets the reached region
        // only where that one does.
        let grown = rectangle(origin, width + 2.0 * shrink, depth + 2.0 * shrink, right);
        let absence = || {
            (w > 0.0 && d > 0.0).then(|| {
                scene
                    .free_for(&rectangle(origin, w, d, right), true)
                    .and_then(|free| scene.reaching(free, &grown, true))
                    .map(|free| free.is_empty())
            })
        };
        // The first interval takes the witness first; a later one the
        // absence, and the witness only where that is not proven. A
        // refusal of the absence is raised only where no witness is found,
        // as when it came second.
        let first = spent == 1;
        let mut absent = if first { None } else { absence() };
        if matches!(absent, Some(Ok(true))) {
            continue;
        }
        if !witnessless {
            let shape = rectangle(origin, width, depth, right);
            let free = scene.reaching(scene.free_for(&shape, false)?, &shape, false)?;
            let place = |c| rectangle(c, width, depth, right);
            if !free.is_empty()
                && let Some(centre) = scene.witness(&free, place, place)?
            {
                return Ok(Search::Found { centre, right });
            }
        }
        if first {
            absent = absence();
        }
        match absent {
            Some(Ok(true)) => continue,
            Some(Err(refused)) => return Err(refused),
            _ => {}
        }
        open.push((low, middle));
        open.push((middle, high));
    }
    Ok(Search::Nowhere)
}

/// A vertical cylinder of `radius`.
///
/// Past [`OBSTACLE_SUM_POINTS`] corners of obstacles the disc erodes the
/// scope less the obstacles ([`Scene::clear`], taken once per scene)
/// rather than the obstacles being dilated (#393): dilating a floor's
/// wall network by exact arcs is most of a large model's search, and the
/// clear part is no larger than the scope. The erosion also admits
/// centres where the disc only touches an obstacle, so it only widens
/// what an absence must rule out, and witnesses are verified by overlap
/// either way. Smaller scenes are searched as before, bit for bit.
pub(crate) fn circle(scene: &Scene, radius: f64) -> Result<Search, FreeSpaceError> {
    let t = scene.tolerance;
    let err = |e| unavailable("disc morphology", e);
    // Contained in the exact free region: any point in it is a true centre.
    let inner = if region_points(&scene.obstacles) > OBSTACLE_SUM_POINTS {
        scene.clear(false)?.erode_inner(radius, t).map_err(err)?
    } else {
        let inner = scene.scope.erode_inner(radius, t).map_err(err)?;
        if scene.obstacles.is_empty() || inner.is_empty() {
            inner
        } else {
            inner
                .difference(&scene.obstacles.dilate_outer(radius, t).map_err(err)?, t)
                .map_err(err)?
        }
    };
    let inner = scene.for_witness(scene.reaching_disc(inner, radius, false)?)?;
    if (!inner.is_empty() || scene.window.is_some())
        && let Some(centre) = scene.witness(
            &inner,
            |c| circumscribed(c, radius),
            |c| inscribed(c, radius),
        )?
    {
        return Ok(Search::Found {
            centre,
            right: Axis::at(0.0),
        });
    }
    // Containing the exact free region of a slightly smaller circle: if even
    // that is empty, no centre exists. The margin keeps a fit by contact,
    // whose free region is a single point, from reading as none.
    let shrunk = radius - KNIFE_EDGE_METRES;
    if shrunk <= 0.0 {
        return Err(FreeSpaceError::Unavailable(
            "the circle is too small to prove absent".into(),
        ));
    }
    let outer = if region_points(&scene.sure) > OBSTACLE_SUM_POINTS {
        scene.clear(true)?.erode_outer(shrunk, t).map_err(err)?
    } else {
        let outer = scene.room.erode_outer(shrunk, t).map_err(err)?;
        if scene.sure.is_empty() || outer.is_empty() {
            outer
        } else {
            outer
                .difference(&scene.sure.dilate_inner(shrunk, t).map_err(err)?, t)
                .map_err(err)?
        }
    };
    if scene.nowhere_in_domain(&scene.reaching_disc(outer, radius, true)?)? {
        return Ok(Search::Nowhere);
    }
    Ok(Search::Undecided(
        "the circle's fit lies within the disc approximation band",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> Polygon {
        Polygon {
            outer: Ring {
                points: vec![
                    Point2::new(x, y),
                    Point2::new(x + side, y),
                    Point2::new(x + side, y + side),
                    Point2::new(x, y + side),
                ],
            },
            holes: Vec::new(),
        }
    }

    /// A floor's walls as one region with a hole per room: a frame round a
    /// square of `rooms` x `rooms` rooms of 2.9 m behind 0.1 m
    /// walls, each room's corners cut so the region has many corners.
    fn walls(rooms: u32, tolerance: Tolerance) -> Region {
        let side = 3.0 * f64::from(rooms) + 0.1;
        let mut holes = Vec::new();
        for i in 0..rooms {
            for j in 0..rooms {
                let (x, y) = (0.1 + 3.0 * f64::from(i), 0.1 + 3.0 * f64::from(j));
                // A room with each corner cut: eight corners.
                let c = 0.2;
                holes.push(Ring {
                    points: vec![
                        Point2::new(x + c, y),
                        Point2::new(x, y + c),
                        Point2::new(x, y + 2.9 - c),
                        Point2::new(x + c, y + 2.9),
                        Point2::new(x + 2.9 - c, y + 2.9),
                        Point2::new(x + 2.9, y + 2.9 - c),
                        Point2::new(x + 2.9, y + c),
                        Point2::new(x + 2.9 - c, y),
                    ],
                });
            }
        }
        let outer = square(-0.0, -0.0, side).outer;
        Region::new(vec![Polygon { outer, holes }], tolerance).unwrap()
    }

    /// Eroding the scope less the obstacles (#359) gives the centres the
    /// sum gives, up to the overlay's snapping: on a floor of walls, for a
    /// room and a hall-sized scope, for a rectangle turned and not.
    #[test]
    fn eroding_the_free_part_matches_summing_the_obstacles() {
        let tolerance = Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let obstacles = walls(6, tolerance);
        for scope in [square(3.05, 3.05, 3.0), square(-1.0, -1.0, 20.0)] {
            let scope = Region::new(vec![scope], tolerance).unwrap();
            for turn in [0.0, 0.3] {
                let shape = rectangle(Point2::new(0.0, 0.0), 0.9, 1.2, Axis::at(turn));
                let summed = summed_free(&scope, &obstacles, &shape, tolerance).unwrap();
                let eroded = eroded_free(&scope, &obstacles, &shape, tolerance).unwrap();
                assert!(!summed.is_empty());
                let gap = summed.difference(&eroded, tolerance).unwrap().area()
                    + eroded.difference(&summed, tolerance).unwrap().area();
                assert!(gap < 1.0e-9, "{gap}");
            }
        }
        // A region of more corners than are summed: the floor of m14 that
        // outgrew memory had about 3,200.
        assert!(region_points(&walls(18, tolerance)) > OBSTACLE_SUM_POINTS);
    }

    /// The angle search as it ran before #371: every interval takes its
    /// witness, then its absence, until one is found, all are proven empty
    /// or the budget runs out.
    fn any_rectangle_before(
        scene: &Scene,
        width: f64,
        depth: f64,
    ) -> Result<Search, FreeSpaceError> {
        let period = if (width - depth).abs() <= f64::EPSILON * width.max(depth) {
            PI / 2.0
        } else {
            PI
        };
        let half_diagonal = width.hypot(depth) / 2.0;
        let origin = Point2::new(0.0, 0.0);
        let mut open: Vec<(f64, f64)> = (0..8)
            .map(|i| (period * f64::from(i) / 8.0, period * f64::from(i + 1) / 8.0))
            .collect();
        let mut spent = 0usize;
        while let Some((low, high)) = open.pop() {
            if spent >= ANGLE_BUDGET {
                return Ok(Search::Undecided(
                    "orientation search budget exhausted before a verdict",
                ));
            }
            spent += 1;
            let middle = f64::midpoint(low, high);
            let right = Axis::at(middle);
            let shape = rectangle(origin, width, depth, right);
            let free = scene.reaching(scene.free_for(&shape, false)?, &shape, false)?;
            let place = |c| rectangle(c, width, depth, right);
            if !free.is_empty()
                && let Some(centre) = scene.witness(&free, place, place)?
            {
                return Ok(Search::Found { centre, right });
            }
            let shrink = half_diagonal * (high - low) / 2.0 + KNIFE_EDGE_METRES;
            let (w, d) = (width - 2.0 * shrink, depth - 2.0 * shrink);
            let grown = rectangle(origin, width + 2.0 * shrink, depth + 2.0 * shrink, right);
            if w > 0.0
                && d > 0.0
                && scene
                    .reaching(
                        scene.free_for(&rectangle(origin, w, d, right), true)?,
                        &grown,
                        true,
                    )?
                    .is_empty()
            {
                continue;
            }
            open.push((low, middle));
            open.push((middle, high));
        }
        Ok(Search::Nowhere)
    }

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Polygon {
        Polygon {
            outer: Ring {
                points: vec![
                    Point2::new(x, y),
                    Point2::new(x + w, y),
                    Point2::new(x + w, y + h),
                    Point2::new(x, y + h),
                ],
            },
            holes: Vec::new(),
        }
    }

    /// A scene of a scope and room `room`, possible obstacles `obstacles`
    /// and sure ones `sure`, all at `(x, y)` plus the offset.
    fn scene(
        (x, y): (f64, f64),
        room: (f64, f64),
        obstacles: &[(f64, f64, f64, f64)],
        sure: &[(f64, f64, f64, f64)],
    ) -> Scene {
        let tolerance = Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let region = |parts: &[(f64, f64, f64, f64)]| {
            let rings: Vec<Ring> = parts
                .iter()
                .map(|&(a, b, w, h)| rect(x + a, y + b, w, h).outer)
                .collect();
            union(&rings, tolerance).unwrap()
        };
        let scope = region(&[(0.0, 0.0, room.0, room.1)]);
        Scene {
            room: scope.clone(),
            scope,
            obstacles: region(obstacles),
            sure: region(sure),
            tolerance,
            window: None,
            reach: None,
            bracketed: Vec::new(),
            clear: Default::default(),
        }
    }

    /// A named room with its possible and its sure obstacles.
    type Case = (&'static str, (f64, f64), Vec<Rect>, Vec<Rect>);

    /// A rectangle by its corner and size.
    type Rect = (f64, f64, f64, f64);

    /// The search proves over every angle at once only what the angle
    /// search proves interval by interval (#371): on rooms that fit, that
    /// fit nowhere, and that leave the verdict open, a square and an oblong
    /// get the verdict, and a found centre and axis, the search before it
    /// gave, near the origin and at georeferenced coordinates.
    #[test]
    fn the_search_over_every_angle_gives_the_verdicts_of_the_angle_search() {
        let cases: Vec<Case> = vec![
            ("an empty room", (3.0, 3.0), vec![], vec![]),
            (
                "a room with a column",
                (3.0, 3.0),
                vec![(1.0, 1.0, 0.4, 0.4)],
                vec![(1.0, 1.0, 0.4, 0.4)],
            ),
            (
                "a covered room",
                (3.0, 3.0),
                vec![(-0.1, -0.1, 3.2, 3.2)],
                vec![(-0.1, -0.1, 3.2, 3.2)],
            ),
            ("a corridor too narrow", (0.8, 4.0), vec![], vec![]),
            ("a corridor just wide enough", (0.95, 4.0), vec![], vec![]),
            (
                "a room covered only possibly",
                (3.0, 3.0),
                vec![(-0.1, -0.1, 3.2, 3.2)],
                vec![],
            ),
            (
                "a room possibly blocked in the middle",
                (0.95, 0.95),
                vec![(0.4, 0.4, 0.15, 0.15)],
                vec![],
            ),
            (
                "a room surely blocked in the middle",
                (0.95, 0.95),
                vec![(0.4, 0.4, 0.15, 0.15)],
                vec![(0.4, 0.4, 0.15, 0.15)],
            ),
        ];
        for offset in [(0.0, 0.0), (643_692.0, 5_649_156.0)] {
            for (name, room, obstacles, sure) in &cases {
                for (width, depth) in [(0.9, 0.9), (0.9, 1.2)] {
                    let before = format!(
                        "{:?}",
                        any_rectangle_before(&scene(offset, *room, obstacles, sure), width, depth)
                    );
                    let now = format!(
                        "{:?}",
                        any_rectangle(&scene(offset, *room, obstacles, sure), width, depth)
                    );
                    assert_eq!(now, before, "{name} at {offset:?}, {width} x {depth}");
                }
            }
        }
    }
}
