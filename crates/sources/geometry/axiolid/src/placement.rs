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
//! A frame-offset domain limits the centre to a box of offsets in an anchor
//! frame (a [`Window`]). The configuration space is intersected with that
//! box: witnesses come from the box as computed and must also pass the
//! contract's own domain test, and proofs of absence use the box grown by
//! [`KNIFE_EDGE_METRES`], so rounding its corners cannot hide a centre on
//! its edge. For a fixed orientation this is exact up to the same margins.

use std::f64::consts::PI;

use axiolid_core::{Point2, Tolerance};
use axiolid_overlay::{Polygon, Region, Ring, union_soup};
use axioval_engine::FreeSpaceError;

use crate::geometry::Triangle;
use crate::planar::projected_polygons;

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

/// The plan footprint of a triangle set, as a validated region.
pub(crate) fn footprint(
    triangles: &[Triangle],
    tolerance: Tolerance,
) -> Result<Region, FreeSpaceError> {
    let rings: Vec<Ring> = projected_polygons(triangles)
        .into_iter()
        .map(|polygon| polygon.outer)
        .collect();
    union(&rings, tolerance)
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
/// What the obstacles occupy is bracketed: `obstacles` contains it and
/// `sure` lies inside it. They differ only by swept door sectors, whose
/// footprints are polygonised from both sides. A witness must avoid
/// `obstacles`; an absence is proven against `sure`.
pub(crate) struct Scene {
    pub(crate) scope: Region,
    pub(crate) obstacles: Region,
    pub(crate) sure: Region,
    pub(crate) tolerance: Tolerance,
    /// The frame-offset box the centre must also lie in, if any.
    pub(crate) window: Option<Window>,
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
    fn free_for(&self, shape: &Ring, absence: bool) -> Result<Region, FreeSpaceError> {
        let obstacles = if absence { &self.sure } else { &self.obstacles };
        let room = self
            .scope
            .minkowski_erosion(shape, self.tolerance)
            .map_err(|e| unavailable("erosion", e))?;
        if room.is_empty() || obstacles.is_empty() {
            return Ok(room);
        }
        let blocked = obstacles
            .minkowski_sum(shape, self.tolerance)
            .map_err(|e| unavailable("dilation", e))?;
        room.difference(&blocked, self.tolerance)
            .map_err(|e| unavailable("difference", e))
    }

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

    /// A centre in `free` at which `place` verifies, if any candidate does.
    fn witness(
        &self,
        free: &Region,
        place: impl Fn(Point2) -> Ring,
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
            if self.fits(place(candidate))? {
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
}

/// A rectangle whose width follows `right`.
pub(crate) fn fixed_rectangle(
    scene: &Scene,
    width: f64,
    depth: f64,
    right: Axis,
) -> Result<Search, FreeSpaceError> {
    let origin = Point2::new(0.0, 0.0);
    let free =
        scene.for_witness(scene.free_for(&rectangle(origin, width, depth, right), false)?)?;
    if !free.is_empty() || scene.window.is_some() {
        if let Some(centre) = scene.witness(&free, |c| rectangle(c, width, depth, right))? {
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
    if scene.nowhere_in_domain(&scene.free_for(&rectangle(origin, w, d, right), true)?)? {
        return Ok(Search::Nowhere);
    }
    Err(FreeSpaceError::Unavailable(if free.is_empty() {
        "the rectangle fits only within the knife-edge margin".into()
    } else {
        "no candidate in the free region verifies".into()
    }))
}

/// A rectangle at any rotation about the vertical.
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
    let mut spent = 0usize;
    while let Some((low, high)) = open.pop() {
        if spent >= ANGLE_BUDGET {
            return Err(FreeSpaceError::Unavailable(
                "orientation search budget exhausted before a verdict".into(),
            ));
        }
        spent += 1;
        let middle = f64::midpoint(low, high);
        let right = Axis::at(middle);
        let free = scene.free_for(&rectangle(origin, width, depth, right), false)?;
        if !free.is_empty()
            && let Some(centre) = scene.witness(&free, |c| rectangle(c, width, depth, right))?
        {
            return Ok(Search::Found { centre, right });
        }
        // Turning by at most half the interval moves any point of the
        // rectangle by at most the half-diagonal times that angle, so a
        // rectangle that deep inside the middle one is inside every rotation.
        let shrink = half_diagonal * (high - low) / 2.0 + KNIFE_EDGE_METRES;
        let (w, d) = (width - 2.0 * shrink, depth - 2.0 * shrink);
        if w > 0.0
            && d > 0.0
            && scene
                .free_for(&rectangle(origin, w, d, right), true)?
                .is_empty()
        {
            continue;
        }
        open.push((low, middle));
        open.push((middle, high));
    }
    Ok(Search::Nowhere)
}

/// A vertical cylinder of `radius`.
pub(crate) fn circle(scene: &Scene, radius: f64) -> Result<Search, FreeSpaceError> {
    let t = scene.tolerance;
    let err = |e| unavailable("disc morphology", e);
    // Contained in the exact free region: any point in it is a true centre.
    let inner = scene.scope.erode_inner(radius, t).map_err(err)?;
    let inner = if scene.obstacles.is_empty() || inner.is_empty() {
        inner
    } else {
        inner
            .difference(&scene.obstacles.dilate_outer(radius, t).map_err(err)?, t)
            .map_err(err)?
    };
    let inner = scene.for_witness(inner)?;
    if (!inner.is_empty() || scene.window.is_some())
        && let Some(centre) = scene.witness(&inner, |c| circumscribed(c, radius))?
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
    let outer = scene.scope.erode_outer(shrunk, t).map_err(err)?;
    let outer = if scene.sure.is_empty() || outer.is_empty() {
        outer
    } else {
        outer
            .difference(&scene.sure.dilate_inner(shrunk, t).map_err(err)?, t)
            .map_err(err)?
    };
    if scene.nowhere_in_domain(&outer)? {
        return Ok(Search::Nowhere);
    }
    Err(FreeSpaceError::Unavailable(
        "the circle's fit lies within the disc approximation band".into(),
    ))
}
