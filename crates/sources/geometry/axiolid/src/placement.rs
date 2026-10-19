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

use std::cell::{OnceCell, RefCell};
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
    /// The obstacles bounded rather than measured (#401): in `obstacles`
    /// within their bounds, never in `sure`.
    pub(crate) bounded: Vec<axioval_ir::ObjectId>,
    /// `scope` less `obstacles` (`[0]`) and `room` less `sure` (`[1]`),
    /// each taken once a search first erodes it: every shape and angle
    /// erodes the same difference ([`Self::free_for`]).
    pub(crate) clear: [OnceCell<Region>; 2],
    /// The largest disc in each of `clear`, estimated
    /// ([`Self::inscribed`]), `None` where the difference is refused;
    /// taken once.
    pub(crate) inscribed: [OnceCell<Option<Inscribed>>; 2],
    /// The regions a circle search erodes or dilates, simplified on the
    /// side it may use them ([`Self::simple`]); taken once.
    pub(crate) simple: [OnceCell<Option<Region>>; 4],
    /// The regions a rectangle search asks its cheap proofs over,
    /// simplified coarsely on the side each proof may use them
    /// ([`Self::coarse`]); taken once.
    pub(crate) coarse: [OnceCell<Option<Region>>; 6],
    /// The centres [`Self::free_for`] found, by the shape's corners and
    /// side: a space's searches over one scene ask the same rectangles
    /// at the same angles (#427).
    pub(crate) found: Found,
}

/// Centres a scene's rectangle searches found ([`Scene::found`]): whether
/// for an absence, the shape's corners' bits, and the region.
pub(crate) type Found = RefCell<Vec<(bool, Vec<u64>, Region)>>;

/// How many corners the regions a scene keeps in [`Scene::found`] may
/// have together (some 4 MB); past it, more are computed but not kept.
pub(crate) const FOUND_POINTS: usize = 250_000;

/// Keeps `free`, found for an absence (`absence`) or a witness by the
/// shape whose corners have `bits`, in `found` while it is within
/// [`FOUND_POINTS`] and not kept already.
pub(crate) fn keep_found(found: &Found, absence: bool, bits: &[u64], free: &Region) {
    let mut found = found.borrow_mut();
    if found
        .iter()
        .any(|(side, corners, _)| *side == absence && corners == bits)
    {
        return;
    }
    let kept: usize = found
        .iter()
        .map(|(_, _, region)| region_points(region))
        .sum();
    if kept + region_points(free) <= FOUND_POINTS {
        found.push((absence, bits.to_vec(), free.clone()));
    }
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
    ///
    /// Each region is kept with the scene ([`Self::found`]) by the
    /// shape's corners, so a search asking for it again, at the same angle
    /// and size, takes it as it was found; a refusal is not kept.
    fn free_for(&self, shape: &Ring, absence: bool) -> Result<Region, FreeSpaceError> {
        let bits: Vec<u64> = shape
            .points
            .iter()
            .flat_map(|p| [p.x.to_bits(), p.y.to_bits()])
            .collect();
        if let Some((_, _, free)) = self
            .found
            .borrow()
            .iter()
            .find(|(side, corners, _)| *side == absence && *corners == bits)
        {
            return Ok(free.clone());
        }
        let (scope, obstacles) = if absence {
            (&self.room, &self.sure)
        } else {
            (&self.scope, &self.obstacles)
        };
        let free = if region_points(obstacles) > OBSTACLE_SUM_POINTS {
            self.clear(absence)?
                .minkowski_erosion(shape, self.tolerance)
                .map_err(|e| unavailable("erosion", e))?
        } else {
            summed_free(scope, obstacles, shape, self.tolerance)?
        };
        keep_found(&self.found, absence, &bits, &free);
        Ok(free)
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

    /// One of the regions a circle search erodes or dilates, simplified
    /// ([`simplified`], #400) on the side the search may use it: the
    /// witnesses' clear part from inside (`0`), the absence's from around
    /// (`1`), the obstacles from around (`2`) and the sure ones from
    /// inside (`3`). `None` where nothing simplifies. Taken once.
    fn simple(&self, which: usize) -> Option<&Region> {
        self.simple[which]
            .get_or_init(|| {
                let (region, side) = match which {
                    0 => (self.clear(false).ok()?, Side::Inside),
                    1 => (self.clear(true).ok()?, Side::Around),
                    2 => (&self.obstacles, Side::Around),
                    _ => (&self.sure, Side::Inside),
                };
                simplified(region, SIMPLIFY_WITHIN_METRES, side, self.tolerance)
            })
            .as_ref()
    }

    /// One of the regions a rectangle search's cheap proofs ask over
    /// (#427), simplified ([`simplified`]) by up to
    /// [`COARSE_WITHIN_METRES`] on the side each proof may use it: the
    /// sure obstacles from inside (`0`, for an absence), the obstacles
    /// from inside (`1`, for no witness at any angle) and the sure ones
    /// from around (`2`, for a fit somewhere); and the room less `0`
    /// (`3`), the scope less `1` (`4`) and the room less `2` (`5`), which
    /// contain the clear parts ([`Self::clear`]) or lie inside them.
    /// `None` where it keeps more than three quarters of the region's
    /// corners (the proof would cost about what the exact one does), the
    /// overlay refuses it or nothing is left. Taken once.
    fn coarse(&self, which: usize) -> Option<&Region> {
        self.coarse[which]
            .get_or_init(|| {
                if which >= 3 {
                    let scope = if which == 4 { &self.scope } else { &self.room };
                    let part = self.coarse(which - 3)?;
                    return clear_of(scope, part, self.tolerance).ok();
                }
                let (region, side) = match which {
                    0 => (&self.sure, Side::Inside),
                    1 => (&self.obstacles, Side::Inside),
                    _ => (&self.sure, Side::Around),
                };
                simplified(region, COARSE_WITHIN_METRES, side, self.tolerance).filter(|coarse| {
                    !coarse.is_empty() && 4 * region_points(coarse) <= 3 * region_points(region)
                })
            })
            .as_ref()
    }

    /// The largest disc in the clear part ([`Self::clear`]) of a witness,
    /// or of an absence (`absence`), estimated by [`inscribed_disc`]: a
    /// hint, never evidence; [`Inscribed::NONE`] where the part has no
    /// area, `None` where the difference is refused. Taken once per scene.
    pub(crate) fn inscribed(&self, absence: bool) -> Option<Inscribed> {
        *self.inscribed[usize::from(absence)].get_or_init(|| {
            self.clear(absence).ok().map(|clear| {
                inscribed_disc(clear, INSCRIBED_PRECISION_METRES).unwrap_or(Inscribed::NONE)
            })
        })
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
    ///
    /// It is asked first over the coarse clear part ([`Self::coarse`]),
    /// which contains the exact one: empty there, it is empty here too.
    fn fits_nowhere(&self, absence: bool, half: f64) -> bool {
        let radius = (half - KNIFE_EDGE_METRES) * (1.0 - 1.0e-9);
        if radius <= 0.0 {
            return false;
        }
        let octagon = octagon(radius);
        if self
            .coarse(if absence { 3 } else { 4 })
            .is_some_and(|coarse| {
                coarse
                    .minkowski_erosion(&octagon, self.tolerance)
                    .is_ok_and(|free| free.is_empty())
            })
        {
            return true;
        }
        let Ok(clear) = self.clear(absence) else {
            return false;
        };
        clear.is_empty()
            || clear
                .minkowski_erosion(&octagon, self.tolerance)
                .is_ok_and(|free| free.is_empty())
    }

    /// Whether the centres an absence must rule out for the shrunk `shape`
    /// ([`Self::free_for`] with `absence`, then [`Self::reaching`] with
    /// `grown` and [`Self::nowhere_in_domain`]) are surely none, asked
    /// over the sure obstacles simplified from inside ([`Self::coarse`],
    /// #427): fewer obstacles leave more centres, so where those are none,
    /// the exact ones are none too. Summed with the shape as
    /// [`Self::free_for`] sums, or, past [`OBSTACLE_SUM_POINTS`] corners,
    /// the room less them eroded, as it erodes; so a proof found here is
    /// one the exact regions give. `false` where nothing simplifies, the
    /// overlay refuses or anything is left: the search then asks over the
    /// regions as built, as it always did.
    ///
    /// A piece of curved furniture tessellated into some 280 corners of a
    /// few millimetres took the overlay more than half a second to sum
    /// with a 1.8 m rectangle, for each angle interval: every corner's
    /// hull overlaps every other's.
    fn surely_absent(&self, shape: &Ring, grown: &Ring) -> bool {
        let free = if region_points(&self.sure) > OBSTACLE_SUM_POINTS {
            let Some(clear) = self.coarse(3) else {
                return false;
            };
            clear
                .minkowski_erosion(shape, self.tolerance)
                .map_err(|e| unavailable("erosion", e))
        } else {
            let Some(sure) = self.coarse(0) else {
                return false;
            };
            summed_free(&self.room, sure, shape, self.tolerance)
        };
        free.and_then(|free| self.reaching(free, grown, true))
            .and_then(|free| self.nowhere_in_domain(&free))
            .unwrap_or(false)
    }

    /// Whether `shape` fits somewhere in the room clear of the sure
    /// obstacles: the room less them, eroded by it, is not empty. `false`
    /// where the erosion is refused or leaves nothing.
    ///
    /// It is asked first in the room less the sure obstacles simplified
    /// from around ([`Self::coarse`]), which lies inside the exact one.
    fn fits_somewhere(&self, shape: &Ring) -> bool {
        if self.coarse(5).is_some_and(|coarse| {
            !coarse.is_empty()
                && coarse
                    .minkowski_erosion(shape, self.tolerance)
                    .is_ok_and(|free| !free.is_empty())
        }) {
            return true;
        }
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

/// How far a corner a rectangle search's cheap proofs leave out may lie
/// from the chord between its neighbours ([`Scene::coarse`]): a proof
/// with less than this to spare is asked again over the regions as built.
const COARSE_WITHIN_METRES: f64 = 1.0e-3;

/// Obstacle regions with more corners than this are not summed with the
/// shape ([`Scene::free_for`]).
const OBSTACLE_SUM_POINTS: usize = 500;

/// Which side of a region its simplification ([`simplified`]) lies on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// Inside the region: corners are only cut off.
    Inside,
    /// Around the region: notches are only filled.
    Around,
}

/// How far a corner a circle search leaves out may lie from the chord
/// between its neighbours ([`Scene::simple`]).
const SIMPLIFY_WITHIN_METRES: f64 = 1.0e-4;

/// How many times [`simplified`] passes over the rings at most.
const SIMPLIFY_PASSES: usize = 8;

/// `region` with corners lying within `within` of the chord between their
/// neighbours left out, each only where that keeps the result on `side` of
/// the region (#400): for [`Side::Inside`] a corner whose removal cuts off
/// the triangle it spans, for [`Side::Around`] one whose removal fills it,
/// and only where no other corner of the region lies in that triangle, so
/// no ring comes to cross another. `None` where nothing is left out, the
/// overlay refuses the result or does not find it on its side.
///
/// A disc morphology's arrangement grows with every edge, however short,
/// and overlay output along tessellated or flush bodies is jagged with
/// edges of a millimetre and less: a free floor of 600 corners, 500 of
/// them on edges under a centimetre, took the overlay seconds and
/// gigabytes per erosion.
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub(crate) fn simplified(
    region: &Region,
    within: f64,
    side: Side,
    tolerance: Tolerance,
) -> Option<Region> {
    let cross =
        |o: Point2, a: Point2, b: Point2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    // Every corner, sorted by x, for the triangles' emptiness.
    let mut corners: Vec<Point2> = region
        .polygons()
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .flat_map(|ring| ring.points.iter().copied())
        .collect();
    corners.sort_by(|a, b| a.x.total_cmp(&b.x));
    let empty = |u: Point2, v: Point2, w: Point2| {
        let (low, high) = (u.x.min(v.x).min(w.x), u.x.max(v.x).max(w.x));
        let (bottom, top) = (u.y.min(v.y).min(w.y), u.y.max(v.y).max(w.y));
        let first = corners.partition_point(|p| p.x < low);
        let turn = cross(u, v, w);
        corners[first..]
            .iter()
            .take_while(|p| p.x <= high)
            .filter(|p| p.y >= bottom && p.y <= top)
            .filter(|&&p| p != u && p != v && p != w)
            .all(|&p| {
                // Strictly outside the closed triangle on some edge.
                let sides = [cross(u, v, p), cross(v, w, p), cross(w, u, p)];
                sides.iter().any(|s| s * turn < 0.0)
            })
    };
    let mut removed = 0usize;
    let mut polygons = Vec::new();
    for polygon in region.polygons() {
        let mut rings = Vec::new();
        for (index, ring) in std::iter::once(&polygon.outer)
            .chain(&polygon.holes)
            .enumerate()
        {
            let mut points = ring.points.clone();
            let area: f64 = (0..points.len())
                .map(|i| {
                    cross(
                        Point2::new(0.0, 0.0),
                        points[i],
                        points[(i + 1) % points.len()],
                    )
                })
                .sum();
            // The region lies left of an outer ring wound anticlockwise
            // and of a hole wound clockwise.
            let left = (area > 0.0) == (index == 0);
            for _ in 0..SIMPLIFY_PASSES {
                // One pass leaves out no two neighbouring corners, so each
                // triangle is spanned by corners the pass keeps.
                let n = points.len();
                let mut keep = vec![true; n];
                let mut count = n;
                let mut i = 0;
                while i < n && count > 3 {
                    let (before, after) = ((i + n - 1) % n, (i + 1) % n);
                    if !(keep[before] && keep[after]) {
                        i += 1;
                        continue;
                    }
                    let (u, v, w) = (points[before], points[i], points[after]);
                    let turn = cross(u, v, w);
                    let convex = if left { turn > 0.0 } else { turn < 0.0 };
                    let chord = (w.x - u.x).hypot(w.y - u.y);
                    let removable = chord > 0.0
                        && turn.abs() / chord <= within
                        && match side {
                            Side::Inside => convex || turn == 0.0,
                            Side::Around => !convex,
                        }
                        && empty(u, v, w);
                    if removable {
                        keep[i] = false;
                        count -= 1;
                        removed += 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if count == n {
                    break;
                }
                points = points
                    .into_iter()
                    .zip(keep)
                    .filter_map(|(point, kept)| kept.then_some(point))
                    .collect();
            }
            if points.len() < 3 {
                return None;
            }
            rings.push(Ring { points });
        }
        let outer = rings.remove(0);
        polygons.push(Polygon {
            outer,
            holes: rings,
        });
    }
    if removed == 0 {
        return None;
    }
    let simple = Region::new(polygons, tolerance).ok()?;
    let (inner, outer) = match side {
        Side::Inside => (&simple, region),
        Side::Around => (region, &simple),
    };
    inner
        .difference(outer, tolerance)
        .is_ok_and(|beyond| beyond.is_empty())
        .then_some(simple)
}

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
    let shrunk = rectangle(origin, w, d, right);
    if scene.surely_absent(&shrunk, &grown) {
        return Ok(Search::Nowhere);
    }
    let shrunk = scene.free_for(&shrunk, true)?;
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
                let inner = rectangle(origin, w, d, right);
                if scene.surely_absent(&inner, &grown) {
                    return Ok(true);
                }
                scene
                    .free_for(&inner, true)
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
///
/// Where the largest disc of the witnesses' clear part is estimated
/// smaller than this one ([`Scene::inscribed`]), no witness is expected:
/// the absence is asked first, and the witness only where it is not
/// proven (#400). The answer is the one asking the witness first gives:
/// a witness's region lies inside the absence's, so a proven absence
/// leaves no witness, and a refused absence still yields to a witness.
///
/// Each search is asked first over the scene's regions simplified on the
/// side it may use them ([`Scene::simple`]): a witness from inside the
/// free region, an absence around it, so either verdict holds. Only where
/// that leaves the circle undecided, or is refused, is it asked again over
/// the regions as built, so the verdicts are theirs (#400).
pub(crate) fn circle(scene: &Scene, radius: f64) -> Result<Search, FreeSpaceError> {
    let simplified = std::cell::Cell::new(false);
    let fast = circle_decided(scene, radius, Some(&simplified));
    if matches!(fast, Ok(Search::Found { .. } | Search::Nowhere)) || !simplified.get() {
        return fast;
    }
    match circle_decided(scene, radius, None) {
        // Undecided either way: the regions as built may be past the
        // budget the simplified ones kept within.
        Err(_) if matches!(fast, Ok(Search::Undecided(_))) => fast,
        exact => exact,
    }
}

/// The most corners a region a circle search erodes or dilates by a disc
/// may have: a safety net, not a cost model (#394, #402). Since
/// axiolid-overlay 0.3.16 a disc morphology no longer builds one hull per
/// edge against every neighbour, and a toothed floor of 1,202 corners is
/// eroded in about 0.3 s, but the cost still grows faster than the edges
/// where dense teeth line a hole: at 2,400 corners such an erosion takes
/// about 2.5 s, at 5,000 about 17 s and 300 MB, at 10,000 over two
/// minutes. A search past it is refused with the count.
pub(crate) const MORPHOLOGY_POINTS: usize = 5_000;

/// `region` where a disc search may erode or dilate it, else the refusal
/// naming its corners ([`MORPHOLOGY_POINTS`]).
fn within_budget<'a>(region: &'a Region, what: &str) -> Result<&'a Region, FreeSpaceError> {
    let corners = region_points(region);
    if corners > MORPHOLOGY_POINTS {
        return Err(FreeSpaceError::Unavailable(format!(
            "{what} has {corners} corners, more than the {MORPHOLOGY_POINTS} a disc search \
             erodes or dilates"
        )));
    }
    Ok(region)
}

/// [`circle`] over the regions as built, or with `simple`, over the
/// simplified ones where there are, noting there whether any was.
fn circle_decided(
    scene: &Scene,
    radius: f64,
    simple: Option<&std::cell::Cell<bool>>,
) -> Result<Search, FreeSpaceError> {
    let found = |centre| Search::Found {
        centre,
        right: Axis::at(0.0),
    };
    let band = "the circle's fit lies within the disc approximation band";
    let unlikely = scene
        .inscribed(false)
        .is_some_and(|estimate| estimate.bound + INSCRIBED_SLACK_METRES < radius);
    if unlikely {
        return match circle_absent(scene, radius, simple) {
            Ok(true) => Ok(Search::Nowhere),
            Ok(false) => {
                Ok(circle_witness(scene, radius, simple)?.map_or(Search::Undecided(band), found))
            }
            Err(refused) => circle_witness(scene, radius, simple)?
                .map(found)
                .ok_or(refused),
        };
    }
    if let Some(centre) = circle_witness(scene, radius, simple)? {
        return Ok(found(centre));
    }
    if circle_absent(scene, radius, simple)? {
        return Ok(Search::Nowhere);
    }
    Ok(Search::Undecided(band))
}

/// How far below a circle's radius the estimated largest disc must lie
/// for [`circle`] to ask the absence first: far above the estimate's
/// rounding.
const INSCRIBED_SLACK_METRES: f64 = 1.0e-6;

/// The radii at which [`circle`] is expected to place a disc and to prove
/// one absent, from the largest disc estimated in the witnesses' clear
/// part (`found`) and bounded in the absence's (`bound`), as
/// [`Scene::inscribed`] gives them. A witness is verified with the
/// circumscribed polygon, which reaches `1 / cos(π / 64)` of the radius;
/// an absence erodes by the inscribed polygon of the radius less the
/// knife edge, whose sides lie `cos(π / 64)` of it from the centre.
pub(crate) fn searched_radii(found: f64, bound: f64) -> (f64, f64) {
    let apothem = (PI / f64::from(VERIFY_SIDES)).cos();
    let fits = (found * apothem - INSCRIBED_SLACK_METRES).max(0.0);
    // An absence is asked first around simplified regions, whose largest
    // disc may exceed the region's by the corners filled.
    let exceeds = (bound + 2.0 * SIMPLIFY_WITHIN_METRES + INSCRIBED_SLACK_METRES) / apothem
        + KNIFE_EDGE_METRES;
    (fits, exceeds.max(fits))
}

/// A centre at which a disc of `radius` verifiably fits, if any.
fn circle_witness(
    scene: &Scene,
    radius: f64,
    simple: Option<&std::cell::Cell<bool>>,
) -> Result<Option<Point2>, FreeSpaceError> {
    let t = scene.tolerance;
    let err = |e| unavailable("disc morphology", e);
    // The estimated pole of the clear part, where it lies far enough
    // inside, is tried first (#400): verified by direct overlap like any
    // candidate, it spares the erosion.
    let apothem = (PI / f64::from(VERIFY_SIDES)).cos();
    if scene.window.is_none()
        && let Some(estimate) = scene.inscribed(false)
        && estimate.found * apothem > radius + INSCRIBED_SLACK_METRES
        && scene.fits(circumscribed(estimate.centre, radius))?
        && scene.reaches(inscribed(estimate.centre, radius))?
    {
        return Ok(Some(estimate.centre));
    }
    // Contained in the exact free region: any point in it is a true centre.
    let inner = if region_points(&scene.obstacles) > OBSTACLE_SUM_POINTS {
        let clear = match simplest(scene, 0, simple) {
            Some(clear) => clear,
            None => scene.clear(false)?,
        };
        within_budget(clear, "the free floor")?
            .erode_inner(radius, t)
            .map_err(err)?
    } else {
        let inner = within_budget(&scene.scope, "the floor")?
            .erode_inner(radius, t)
            .map_err(err)?;
        let obstacles = simplest(scene, 2, simple).unwrap_or(&scene.obstacles);
        if obstacles.is_empty() || inner.is_empty() {
            inner
        } else {
            inner
                .difference(
                    &within_budget(obstacles, "the obstacles' footprint")?
                        .dilate_outer(radius, t)
                        .map_err(err)?,
                    t,
                )
                .map_err(err)?
        }
    };
    let inner = scene.for_witness(scene.reaching_disc(inner, radius, false)?)?;
    if inner.is_empty() && scene.window.is_none() {
        return Ok(None);
    }
    scene.witness(
        &inner,
        |c| circumscribed(c, radius),
        |c| inscribed(c, radius),
    )
}

/// Whether no disc of `radius` fits anywhere: the region containing the
/// exact free region of a slightly smaller disc is empty. The margin keeps
/// a fit by contact, whose free region is a single point, from reading as
/// none.
fn circle_absent(
    scene: &Scene,
    radius: f64,
    simple: Option<&std::cell::Cell<bool>>,
) -> Result<bool, FreeSpaceError> {
    let t = scene.tolerance;
    let err = |e| unavailable("disc morphology", e);
    let shrunk = radius - KNIFE_EDGE_METRES;
    if shrunk <= 0.0 {
        return Err(FreeSpaceError::Unavailable(
            "the circle is too small to prove absent".into(),
        ));
    }
    let outer = if region_points(&scene.sure) > OBSTACLE_SUM_POINTS {
        let clear = match simplest(scene, 1, simple) {
            Some(clear) => clear,
            None => scene.clear(true)?,
        };
        within_budget(clear, "the free floor")?
            .erode_outer(shrunk, t)
            .map_err(err)?
    } else {
        let outer = within_budget(&scene.room, "the floor")?
            .erode_outer(shrunk, t)
            .map_err(err)?;
        let sure = simplest(scene, 3, simple).unwrap_or(&scene.sure);
        if sure.is_empty() || outer.is_empty() {
            outer
        } else {
            outer
                .difference(
                    &within_budget(sure, "the obstacles' footprint")?
                        .dilate_inner(shrunk, t)
                        .map_err(err)?,
                    t,
                )
                .map_err(err)?
        }
    };
    scene.nowhere_in_domain(&scene.reaching_disc(outer, radius, true)?)
}

/// The scene's simplified region `which` ([`Scene::simple`]) where a
/// search asks for simplified regions (`simple`) and there is one, noting
/// it there.
fn simplest<'a>(
    scene: &'a Scene,
    which: usize,
    simple: Option<&std::cell::Cell<bool>>,
) -> Option<&'a Region> {
    let used = simple?;
    let region = scene.simple(which)?;
    used.set(true);
    Some(region)
}

/// Cells [`inscribed_disc`] measures at most.
const INSCRIBED_CELLS: usize = 4096;

/// How closely [`Scene::inscribed`] estimates the largest disc's radius.
const INSCRIBED_PRECISION_METRES: f64 = 1.0e-4;

/// The largest disc in a region, estimated ([`inscribed_disc`]): a hint
/// for a search, never evidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Inscribed {
    /// A point lying `found` inside the region, up to rounding.
    pub(crate) centre: Point2,
    /// A radius the region holds a disc of, up to rounding.
    pub(crate) found: f64,
    /// A radius no disc in the region exceeds, up to rounding.
    pub(crate) bound: f64,
}

impl Inscribed {
    /// No disc: a region without area.
    pub(crate) const NONE: Self = Self {
        centre: Point2::new(0.0, 0.0),
        found: 0.0,
        bound: 0.0,
    };
}

/// The largest disc inside `region`, estimated in floating point: a
/// point some radius inside the region and a radius no point lies farther
/// inside than, each up to the rounding of the distances.
///
/// Square cells over the region's box are measured at their centres
/// (the signed distance to the nearest ring edge, positive inside) and
/// split where the centre's distance plus the half diagonal, which bounds
/// every point of the cell since the distance is 1-Lipschitz, could still
/// beat the best centre by more than `precision` (the pole of
/// inaccessibility). Once [`INSCRIBED_CELLS`] cells are measured, the
/// bound is the largest any unsplit cell allows. A region without area
/// holds no disc: `None`.
#[allow(clippy::too_many_lines)]
pub(crate) fn inscribed_disc(region: &Region, precision: f64) -> Option<Inscribed> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

    struct Cell {
        centre: Point2,
        half: f64,
        distance: f64,
        most: f64,
    }
    impl PartialEq for Cell {
        fn eq(&self, other: &Self) -> bool {
            self.most.total_cmp(&other.most) == Ordering::Equal
        }
    }
    impl Eq for Cell {}
    impl PartialOrd for Cell {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Cell {
        fn cmp(&self, other: &Self) -> Ordering {
            self.most.total_cmp(&other.most)
        }
    }

    let rings: Vec<&[Point2]> = region
        .polygons()
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .map(|ring| ring.points.as_slice())
        .filter(|points| points.len() >= 3)
        .collect();
    let (mut low, mut high) = (
        Point2::new(f64::INFINITY, f64::INFINITY),
        Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    for point in rings.iter().copied().flatten() {
        low = Point2::new(low.x.min(point.x), low.y.min(point.y));
        high = Point2::new(high.x.max(point.x), high.y.max(point.y));
    }
    let side = (high.x - low.x).min(high.y - low.y);
    if !(side.is_finite() && side > 0.0) {
        return None;
    }
    // Measured about the box's corner: the differences stay small at
    // georeferenced coordinates.
    let rings: Vec<Vec<Point2>> = rings
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|p| Point2::new(p.x - low.x, p.y - low.y))
                .collect()
        })
        .collect();
    let signed = |p: Point2| {
        let mut inside = false;
        let mut nearest = f64::INFINITY;
        for ring in &rings {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                {
                    inside = !inside;
                }
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let length = dx * dx + dy * dy;
                let t = if length > 0.0 {
                    (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (ex, ey) = (a.x + t * dx - p.x, a.y + t * dy - p.y);
                nearest = nearest.min(ex * ex + ey * ey);
            }
        }
        let distance = nearest.sqrt();
        if inside { distance } else { -distance }
    };
    let cell = |centre: Point2, half: f64| {
        let distance = signed(centre);
        Cell {
            centre,
            half,
            distance,
            most: distance + half * std::f64::consts::SQRT_2,
        }
    };
    let (width, height) = (high.x - low.x, high.y - low.y);
    // A thin box is covered by at most a quarter of the cells at first.
    #[allow(clippy::cast_precision_loss)]
    let side = side.max((width * height / (INSCRIBED_CELLS / 4) as f64).sqrt());
    let half = side / 2.0;
    let mut queue = BinaryHeap::new();
    let mut x = 0.0;
    while x < width {
        let mut y = 0.0;
        while y < height {
            queue.push(cell(Point2::new(x + half, y + half), half));
            y += side;
        }
        x += side;
    }
    let mut measured = queue.len() + 1;
    let first = cell(Point2::new(width / 2.0, height / 2.0), 0.0);
    let (mut best, mut at) = (first.distance, first.centre);
    let found = |best: f64, at: Point2, bound: f64| {
        (best > 0.0).then(|| Inscribed {
            centre: Point2::new(at.x + low.x, at.y + low.y),
            found: best,
            bound: bound.max(best),
        })
    };
    while let Some(next) = queue.pop() {
        if next.distance > best {
            (best, at) = (next.distance, next.centre);
        }
        if next.most - best <= precision {
            continue;
        }
        if measured >= INSCRIBED_CELLS {
            // The largest bound left: the queue is ordered by it.
            return found(best, at, next.most);
        }
        let h = next.half / 2.0;
        for (dx, dy) in [(-h, -h), (h, -h), (-h, h), (h, h)] {
            let centre = Point2::new(next.centre.x + dx, next.centre.y + dy);
            queue.push(cell(centre, h));
        }
        measured += 4;
    }
    found(best, at, best + precision)
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
            bounded: Vec::new(),
            clear: Default::default(),
            inscribed: Default::default(),
            simple: Default::default(),
            coarse: Default::default(),
            found: Found::default(),
        }
    }

    /// A ring of `sides` corners round `(x, y)`, every other one `notch`
    /// further in: a tessellated round table with a jagged outline.
    fn table(x: f64, y: f64, radius: f64, sides: u32, notch: f64) -> Ring {
        Ring {
            points: (0..sides)
                .map(|i| {
                    let angle = 2.0 * PI * f64::from(i) / f64::from(sides);
                    let r = if i % 2 == 0 { radius } else { radius - notch };
                    Point2::new(x + r * angle.cos(), y + r * angle.sin())
                })
                .collect(),
        }
    }

    /// A box `w` x `h` centred on `(x, y)`, turned by `turn` radians.
    fn turned(x: f64, y: f64, w: f64, h: f64, turn: f64) -> Ring {
        rectangle(Point2::new(x, y), w, h, Axis::at(turn))
    }

    /// The quarter disc a 0.9 m leaf hinged at `(x, y)` sweeps from along
    /// +x down to -y, polygonised around it (`around`) or inside it.
    fn swing(x: f64, y: f64, around: bool) -> Ring {
        let sides = 16;
        let radius = if around {
            0.9 / (PI / 4.0 / f64::from(sides)).cos()
        } else {
            0.9
        };
        let mut points = vec![Point2::new(x, y)];
        points.extend((0..=sides).map(|i| {
            let angle = -PI / 2.0 + PI / 2.0 * f64::from(i) / f64::from(sides);
            Point2::new(x + radius * angle.cos(), y + radius * angle.sin())
        }));
        Ring { points }
    }

    /// An L-shaped room (6 x 4 m, and 3 x 3 m north of its west half),
    /// furnished with twenty obstacles: a round table tessellated into 120
    /// jagged corners, two coarser ones, sixteen boxes (some turned)
    /// and a door swinging in from the north wall, its sector around the
    /// swing among the obstacles and inside it among the sure ones. With
    /// `reach`, a path from that door reaches a strip along the west wall.
    fn furnished(offset: (f64, f64), reach: bool) -> Scene {
        let tolerance = Tolerance::new(1.0e-9, 1.0e-9).unwrap();
        let (x, y) = offset;
        let at = |ring: Ring| Ring {
            points: ring
                .points
                .iter()
                .map(|p| Point2::new(p.x + x, p.y + y))
                .collect(),
        };
        let scope = union(
            &[
                at(rect(0.0, 0.0, 6.0, 4.0).outer),
                at(rect(0.0, 4.0, 3.0, 3.0).outer),
            ],
            tolerance,
        )
        .unwrap();
        let mut furniture = vec![
            table(1.6, 1.7, 0.6, 120, 3.0e-4),
            table(4.6, 2.4, 0.45, 12, 0.0),
            table(1.4, 5.6, 0.35, 10, 0.0),
        ];
        for (cx, cy, w, h, turn) in [
            (0.3, 3.3, 0.6, 1.2, 0.0),
            (5.7, 0.6, 0.6, 1.2, 0.0),
            (3.0, 0.25, 1.6, 0.5, 0.0),
            (5.6, 3.6, 0.8, 0.8, 0.35),
            (2.7, 3.0, 0.5, 0.5, 0.8),
            (0.5, 0.4, 0.8, 0.6, 0.0),
            (2.6, 6.6, 0.8, 0.8, 0.0),
            (0.3, 6.5, 0.6, 1.0, 0.0),
            (4.0, 3.7, 1.2, 0.6, -0.2),
            (3.6, 1.6, 0.4, 0.4, 0.6),
            (2.6, 4.9, 0.3, 0.9, 0.0),
            (0.25, 4.8, 0.5, 0.6, 0.0),
            (5.0, 1.2, 0.3, 0.3, 0.0),
            (2.0, 0.3, 0.4, 0.6, 0.1),
            (1.2, 3.75, 0.9, 0.5, 0.0),
            (4.9, 0.25, 0.9, 0.5, 0.0),
        ] {
            furniture.push(turned(cx, cy, w, h, turn));
        }
        let mut possible: Vec<Ring> = furniture.iter().cloned().map(at).collect();
        let mut certain = possible.clone();
        possible.push(at(swing(1.9, 7.0, true)));
        certain.push(at(swing(1.9, 7.0, false)));
        let reach = reach.then(|| {
            let strip = |grow: f64| {
                union(
                    &[at(rect(-grow, 2.0, 0.9 + 2.0 * grow, 5.0).outer)],
                    tolerance,
                )
                .unwrap()
            };
            Reached {
                inner: strip(0.0),
                outer: strip(0.01),
            }
        });
        Scene {
            room: scope.clone(),
            scope,
            obstacles: union(&possible, tolerance).unwrap(),
            sure: union(&certain, tolerance).unwrap(),
            tolerance,
            window: None,
            reach,
            bracketed: Vec::new(),
            bounded: Vec::new(),
            clear: Default::default(),
            inscribed: Default::default(),
            simple: Default::default(),
            coarse: Default::default(),
            found: Found::default(),
        }
    }

    /// A rectangle at a fixed orientation as it was searched before #427:
    /// the witness, then the absence over the regions as built.
    fn fixed_rectangle_before(
        scene: &Scene,
        width: f64,
        depth: f64,
        right: Axis,
    ) -> Result<Search, FreeSpaceError> {
        let origin = Point2::new(0.0, 0.0);
        let shape = rectangle(origin, width, depth, right);
        let free = scene.reaching(scene.free_for(&shape, false)?, &shape, false)?;
        let place = |c| rectangle(c, width, depth, right);
        if !free.is_empty()
            && let Some(centre) = scene.witness(&free, place, place)?
        {
            return Ok(Search::Found { centre, right });
        }
        let (w, d) = (
            width - 2.0 * KNIFE_EDGE_METRES,
            depth - 2.0 * KNIFE_EDGE_METRES,
        );
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

    /// The cheap proofs over coarse regions and the regions kept between
    /// searches (#427) change no verdict: in a furnished L-shaped room,
    /// with a path from the door and without, near the origin and at
    /// georeferenced coordinates, squares and oblongs that fit and that fit
    /// nowhere, searched one after another over one scene, get the
    /// verdict, and a found centre and axis to the bit, that the searches
    /// over the regions as built give, each over a scene of its own; and
    /// asked again, the same.
    #[test]
    fn a_furnished_room_is_searched_as_over_the_regions_as_built() {
        let geo = (643_692.0, 5_649_156.0);
        let cases = [
            ((0.0, 0.0), false, &[(0.9, 0.9), (1.5, 1.5), (1.8, 1.2)][..]),
            ((0.0, 0.0), true, &[(0.9, 0.9), (1.8, 1.2)][..]),
            (geo, false, &[(0.9, 0.9), (1.8, 1.2)][..]),
        ];
        let mut verdicts = Vec::new();
        for (offset, reach, sizes) in cases {
            let shared = furnished(offset, reach);
            // The jagged table simplifies, so the cheap proofs are asked.
            let coarse = region_points(shared.coarse(0).expect("simplified"));
            assert!(4 * coarse <= 3 * region_points(&shared.sure), "{coarse}");
            for &(width, depth) in sizes {
                let case = format!("{width} x {depth} at {offset:?}, reach {reach}");
                let before = format!(
                    "{:?}",
                    any_rectangle_before(&furnished(offset, reach), width, depth)
                );
                for _ in 0..2 {
                    let now = format!("{:?}", any_rectangle(&shared, width, depth));
                    assert_eq!(now, before, "any orientation, {case}");
                }
                let right = Axis::at(0.4);
                let before_fixed = format!(
                    "{:?}",
                    fixed_rectangle_before(&furnished(offset, reach), width, depth, right)
                );
                let now = format!("{:?}", fixed_rectangle(&shared, width, depth, right));
                assert_eq!(now, before_fixed, "turned, {case}");
                verdicts.push(before);
            }
        }
        // The sizes span both verdicts.
        assert!(
            verdicts.iter().any(|v| v.starts_with("Ok(Found")),
            "{verdicts:?}"
        );
        assert!(verdicts.iter().any(|v| v == "Ok(Nowhere)"), "{verdicts:?}");
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

    /// The largest disc is estimated within a millimetre (#400): a 4 x 3
    /// room holds one of radius 1.5, and a 3 x 3 room round a 1 x 1
    /// column one of `2 - √2` in a corner, touching both walls and the
    /// column's corner; near the origin and far from it. Nothing holds
    /// none.
    #[test]
    fn the_largest_disc_is_estimated_within_a_millimetre() {
        let precision = INSCRIBED_PRECISION_METRES;
        let column = [(1.0, 1.0, 1.0, 1.0)];
        for offset in [(0.0, 0.0), (643_692.0, 5_649_156.0)] {
            for (room, obstacles, largest) in [
                ((4.0, 3.0), &[][..], 1.5),
                ((3.0, 3.0), &column[..], 2.0 - 2.0_f64.sqrt()),
            ] {
                let scene = scene(offset, room, obstacles, obstacles);
                for absence in [false, true] {
                    let estimate = scene.inscribed(absence).expect("measured");
                    let (found, bound) = (estimate.found, estimate.bound);
                    assert!(found <= largest + 1.0e-7, "{found} {largest}");
                    assert!(bound >= largest - 1.0e-7, "{bound} {largest}");
                    // A room's centre line is a ridge of largest discs,
                    // which spends the cells before the precision.
                    assert!(bound - found <= 1.0e-3, "{found} {bound}");
                }
            }
        }
        assert_eq!(inscribed_disc(&Region::empty(), precision), None);
    }

    /// A square of `side` on `(x, y)`, anticlockwise, whose bottom and top
    /// edges are toothed `depth` deep every `step`, as overlay output along
    /// a tessellated or flush body is jagged.
    fn jagged(x: f64, y: f64, side: f64, step: f64, depth: f64) -> Ring {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = (side / step).round() as u32;
        let at = |k: u32| f64::from(k) * side / f64::from(n);
        let tooth = |k: u32| if k % 2 == 1 { depth } else { 0.0 };
        let mut points: Vec<Point2> = (0..=n)
            .map(|k| Point2::new(x + at(k), y + tooth(k)))
            .collect();
        points.extend((0..=n).map(|k| Point2::new(x + side - at(k), y + side - tooth(k))));
        Ring { points }
    }

    /// A 3 x 3 room round a jagged 1 x 1 column, possible and sure; teeth
    /// every 4 mm (past `OBSTACLE_SUM_POINTS` corners, so eroded) or 2 cm
    /// (summed).
    fn jagged_scene(offset: (f64, f64), step: f64) -> Scene {
        let mut scene = scene(offset, (3.0, 3.0), &[], &[]);
        let column = Region::new(
            vec![Polygon {
                outer: jagged(offset.0 + 1.0, offset.1 + 1.0, 1.0, step, 5.0e-5),
                holes: Vec::new(),
            }],
            scene.tolerance,
        )
        .unwrap();
        scene.obstacles = column.clone();
        scene.sure = column;
        scene
    }

    /// A jagged region simplifies on the side asked (#400): cutting off
    /// corners stays inside it, filling notches around it, each with a
    /// fraction of its corners, near the origin and far from it.
    #[test]
    fn a_jagged_region_simplifies_on_the_side_asked() {
        for offset in [(0.0, 0.0), (643_692.0, 5_649_156.0)] {
            let plain = scene(offset, (3.0, 3.0), &[], &[]);
            let scene = jagged_scene(offset, 0.004);
            let clear = scene.clear(false).unwrap();
            let t = scene.tolerance;
            for (side, which) in [(Side::Inside, 0), (Side::Around, 1)] {
                let simple = scene.simple(which).expect("simplified");
                assert!(
                    region_points(simple) * 3 < region_points(clear),
                    "{side:?}: {} of {}",
                    region_points(simple),
                    region_points(clear)
                );
                let (inner, outer) = match side {
                    Side::Inside => (simple, clear),
                    Side::Around => (clear, simple),
                };
                assert!(inner.difference(outer, t).unwrap().is_empty(), "{side:?}");
                assert!((simple.area() - clear.area()).abs() < 1.0e-3, "{side:?}");
            }
            // A plain room has nothing to leave out.
            assert!(scene.simple(2).is_some());
            assert!(plain.simple(0).is_none());
        }
    }

    /// A free floor past the old 1,000-corner budget (a 1 x 1 column
    /// toothed 0.5 mm deep along two sides, 1,202 corners) is eroded and
    /// decided both ways since axiolid-overlay 0.3.16 (#402): no disc of
    /// 1 m radius fits in the 1 m band round it, and one of 0.4 m does.
    #[test]
    fn a_floor_past_the_old_corner_budget_is_decided() {
        let mut deep = scene((0.0, 0.0), (3.0, 3.0), &[], &[]);
        let column = Region::new(
            vec![Polygon {
                outer: jagged(1.0, 1.0, 1.0, 1.0 / 600.0, 5.0e-4),
                holes: Vec::new(),
            }],
            deep.tolerance,
        )
        .unwrap();
        assert_eq!(column.polygons()[0].outer.points.len(), 1_202);
        deep.obstacles = column.clone();
        deep.sure = column;
        assert!(region_points(deep.clear(true).unwrap()) > 1_000);
        assert!(matches!(circle(&deep, 1.0), Ok(Search::Nowhere)));
        assert!(matches!(circle(&deep, 0.4), Ok(Search::Found { .. })));
    }

    /// A free floor of more corners than a disc search erodes is refused
    /// with their count (#394), not eroded until memory runs out; a disc
    /// that fits at the floor's estimated pole is still found there.
    #[test]
    fn a_floor_past_the_corner_budget_is_refused_by_name() {
        let mut scene = jagged_scene((0.0, 0.0), 0.001);
        let column = Region::new(
            vec![Polygon {
                outer: jagged(1.0, 1.0, 1.0, 1.0e-4, 5.0e-4),
                holes: Vec::new(),
            }],
            scene.tolerance,
        )
        .unwrap();
        scene.obstacles = column.clone();
        scene.sure = column;
        assert!(region_points(scene.clear(true).unwrap()) > MORPHOLOGY_POINTS);
        let refused = circle(&scene, 1.0).expect_err("refused");
        assert!(
            refused.to_string().contains("corners, more than the 5000"),
            "{refused}"
        );
        assert!(matches!(circle(&scene, 0.4), Ok(Search::Found { .. })));
    }

    /// The circle search as it ran before #400: the witness first, then
    /// the absence.
    fn circle_before(scene: &Scene, radius: f64) -> Result<Search, FreeSpaceError> {
        if let Some(centre) = circle_witness(scene, radius, None)? {
            return Ok(Search::Found {
                centre,
                right: Axis::at(0.0),
            });
        }
        if circle_absent(scene, radius, None)? {
            return Ok(Search::Nowhere);
        }
        Ok(Search::Undecided(
            "the circle's fit lies within the disc approximation band",
        ))
    }

    /// Asking the absence first where the estimate expects no witness
    /// (#400) gives the verdicts, and found centres, of asking the witness
    /// first: around the largest disc of a room, of a room round a column
    /// and of one whose obstacle is only possible, near the origin and far
    /// from it.
    #[test]
    fn a_circle_asked_absence_first_gets_the_verdicts_of_the_witness_first() {
        let column = [(1.0, 1.0, 1.0, 1.0)];
        let cases: Vec<Case> = vec![
            ("an empty room", (4.0, 3.0), vec![], vec![]),
            (
                "a room round a column",
                (3.0, 3.0),
                column.to_vec(),
                column.to_vec(),
            ),
            (
                "a room round a possible column",
                (3.0, 3.0),
                column.to_vec(),
                vec![],
            ),
        ];
        let corner = 2.0 - 2.0_f64.sqrt();
        for offset in [(0.0, 0.0), (643_692.0, 5_649_156.0)] {
            for (name, room, obstacles, sure) in &cases {
                for radius in [
                    0.4,
                    corner - 0.01,
                    corner,
                    corner + 0.01,
                    1.0,
                    1.499,
                    1.5,
                    1.6,
                ] {
                    let before = format!(
                        "{:?}",
                        circle_before(&scene(offset, *room, obstacles, sure), radius)
                    );
                    let now = format!(
                        "{:?}",
                        circle(&scene(offset, *room, obstacles, sure), radius)
                    );
                    assert_eq!(now, before, "{name} at {offset:?}, radius {radius}");
                }
            }
            // Round a jagged column the simplified regions decide alike.
            let kind = |search: Result<Search, FreeSpaceError>| match search {
                Ok(Search::Found { .. }) => "found",
                Ok(Search::Nowhere) => "nowhere",
                Ok(Search::Undecided(_)) => "undecided",
                Err(_) => "refused",
            };
            for radius in [0.4, corner - 0.01, corner + 0.01, 1.0] {
                assert_eq!(
                    kind(circle(&jagged_scene(offset, 0.02), radius)),
                    kind(circle_before(&jagged_scene(offset, 0.02), radius)),
                    "a jagged column at {offset:?}, radius {radius}"
                );
            }
        }
    }
}
