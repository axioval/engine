//! Circulation maps: where a path of a given width runs within one space.
//!
//! ADR 0004: this module measures. It reports the free area eroded by half
//! the path's width from both sides, the skeleton of the inner bound, and
//! which pieces come near each entrance and component. Whether that meets a
//! rule is the capability's decision.
//!
//! - The free area is the placement scene's: the scope's footprint less what
//!   every obstacle occupies in the band from the floor up by the request's
//!   height.
//! - **Pieces** are the polygons of the free area eroded from inside
//!   (`Region::erode_inner` of the footprint less `dilate_outer` of the
//!   obstacles, as the circle placement search builds it): every point of
//!   one is a centre where a disc as wide as the path fits, and a polygon
//!   is connected, so a path runs between any two of its points.
//! - **Possible pieces** are the polygons of the erosion from outside
//!   (`erode_outer` less `dilate_inner`) for half the width less
//!   [`KNIFE_EDGE_METRES`]. It holds every centre of a disc as wide as the
//!   path with that margin around it, so a path, which is connected, lies
//!   inside one of its polygons; a gap exactly as wide as the path keeps a
//!   sliver instead of being snapped shut. A path between two different
//!   possible pieces therefore does not exist.
//! - A subject (entrance or component) is near a piece when the piece meets
//!   the subject's plan footprint grown by half the width plus the
//!   tolerance: grown from inside (`dilate_inner`) for a proven contact,
//!   from outside (`dilate_outer`) for a possible one.
//! - The skeleton is `axiolid_route::skeleton` of each piece, with pruning
//!   factor 1.5 and a boundary spacing of a tenth of the width. Its
//!   nodes lie in their piece (decided exactly by the kernel); their
//!   positions are approximate. Each node's half width is its distance to
//!   the free area's boundary, widened by the overlay's grid snapping.
//!
//! - Swept door sectors (the request's) are obstacles over the whole band,
//!   bracketed by `SwingSector::plan_bounds`: pieces keep clear of the
//!   circumscribed polygons, possible pieces only of the inscribed ones,
//!   and a node's half width runs from the free area less the former to
//!   the free area less the latter. An entrance's own swing is never among
//!   them: the request drops it.

use axiolid_core::{Point2, Point3, Tolerance, Vec2};
use axiolid_mesh::TriMesh;
use axiolid_overlay::{Polygon, Region};
use axiolid_route::{NodeKind, Skeleton, skeleton};
use axioval_engine::{
    CirculationContact, CirculationMap, CirculationNode, CirculationNodeKind, CirculationRequest,
    EntranceReach, FreeSpaceError, GeometryFidelity, LengthInterval,
};
use axioval_ir::{Evidence, ObjectId};

use crate::free_space::AxiolidFreeSpaceService;
use crate::free_space::plan_region;
use crate::geometry::{mesh_extent, triangles};
use crate::placement::{self, KNIFE_EDGE_METRES, Reached, Scene};
use crate::planar::budget::{Cost, Work, disc_cost, overlay_cost};
use crate::planar::{self, Morphology, PlanarRefusal};
use crate::walkable::{self, Plan};

/// The free area eroded by a path's half width `radius`, from both sides:
/// pieces inside the exact erosion (`erode_inner` of the scope less the
/// obstacles `dilate_outer`ed) and possible pieces containing it
/// (`erode_outer` of the room by a knife edge less, less the sure
/// obstacles `dilate_inner`ed). Each disc morphology and difference is
/// charged to `work` before it is built and refused past its budget
/// (#434): a floor merged with the space beside it, among many
/// tessellated obstacles, may ask the overlay for gigabytes.
pub(crate) fn eroded_free(
    scene: &Scene,
    radius: f64,
    work: &Work,
) -> Result<(Region, Region), FreeSpaceError> {
    let t = scene.tolerance;
    let err = |e| unavailable("disc morphology", e);
    let charge = |what: &'static str, cost: Cost| {
        work.charge(what, cost)
            .map_err(|refusal| FreeSpaceError::Unavailable(refusal.to_string()))
    };
    let morphology =
        |region: &Region, radius: f64, erosion: bool| disc_cost(region.polygons(), radius, erosion);
    let less = |a: &Region, b: &Region| overlay_cost(&[a.polygons(), b.polygons()]);
    // Each step is built while its admission is held.
    let mut inner = {
        let _step = charge(
            "erosion of the floor by the path's disc",
            morphology(&scene.scope, radius, true),
        )?;
        scene.scope.erode_inner(radius, t).map_err(err)?
    };
    if !scene.obstacles.is_empty() && !inner.is_empty() {
        let grown = {
            let _step = charge(
                "dilation of the obstacles by the path's disc",
                morphology(&scene.obstacles, radius, false),
            )?;
            scene.obstacles.dilate_outer(radius, t).map_err(err)?
        };
        let _step = charge("floor less the dilated obstacles", less(&inner, &grown))?;
        inner = inner.difference(&grown, t).map_err(err)?;
    }
    // For a radius a knife edge smaller, so that a gap exactly as wide
    // as the path keeps a sliver rather than being snapped shut.
    let shrunk = radius - KNIFE_EDGE_METRES;
    let mut outer = {
        let _step = charge(
            "erosion of the floor by the path's disc",
            morphology(&scene.room, shrunk, true),
        )?;
        scene.room.erode_outer(shrunk, t).map_err(err)?
    };
    if !scene.sure.is_empty() && !outer.is_empty() {
        let grown = {
            let _step = charge(
                "dilation of the obstacles by the path's disc",
                morphology(&scene.sure, shrunk, false),
            )?;
            scene.sure.dilate_inner(shrunk, t).map_err(err)?
        };
        let _step = charge("floor less the dilated obstacles", less(&outer, &grown))?;
        outer = outer.difference(&grown, t).map_err(err)?;
    }
    Ok((inner, outer))
}

/// The skeleton's pruning factor: keeps corridors, junctions and dead ends
/// and drops the spurs into right-angled and sharper corners.
const PRUNE: f64 = 1.5;

/// Boundary samples per path width: the kernel asks for a spacing under an
/// eighth of the narrowest width.
const SAMPLES_PER_WIDTH: f64 = 10.0;

/// Most boundary samples one piece may take; a longer boundary is sampled
/// more coarsely. The kernel's triangulation grows faster than linearly.
const MOST_SAMPLES: f64 = 4_000.0;

/// The skeleton of `polygon` at `spacing`, or why the kernel refused it.
fn piece_skeleton(polygon: &Polygon, spacing: f64) -> Result<Skeleton, String> {
    skeleton(std::slice::from_ref(polygon), spacing, PRUNE).map_err(|error| format!("{error:?}"))
}

/// The connected parts of a skeleton with at least one edge.
fn components(shape: &Skeleton) -> usize {
    fn root(part: &mut [usize], mut node: usize) -> usize {
        while part[node] != node {
            part[node] = part[part[node]];
            node = part[node];
        }
        node
    }
    let mut part: Vec<usize> = (0..shape.nodes.len()).collect();
    for &(a, b) in &shape.edges {
        let (a, b) = (root(&mut part, a), root(&mut part, b));
        part[a] = b;
    }
    let mut roots: Vec<usize> = shape
        .edges
        .iter()
        .map(|&(a, _)| root(&mut part, a))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    roots.len()
}

fn unavailable(context: &str, error: impl std::fmt::Debug) -> FreeSpaceError {
    FreeSpaceError::Unavailable(format!("{context}: {error:?}"))
}

/// [`unavailable`] for an operation held to the overlay budget (#432): an
/// overlay refusal reads as it always did, a budget's by name.
#[allow(clippy::needless_pass_by_value)]
fn refused(context: &str, error: PlanarRefusal) -> FreeSpaceError {
    FreeSpaceError::Unavailable(format!("{context}: {}", error.described()))
}

/// `region` grown by a disc of `radius`, from inside (`inner`) or outside,
/// held to the disc morphology budget (#432).
fn grown(
    region: &Region,
    radius: f64,
    inner: bool,
    t: Tolerance,
) -> Result<Region, FreeSpaceError> {
    let kind = if inner {
        Morphology::DilateInner
    } else {
        Morphology::DilateOuter
    };
    planar::morphology(region, kind, radius, t).map_err(|e| refused("disc morphology", e))
}

fn rings(polygon: &Polygon) -> impl Iterator<Item = &[Point2]> {
    std::iter::once(polygon.outer.points.as_slice())
        .chain(polygon.holes.iter().map(|ring| ring.points.as_slice()))
}

fn edges(points: &[Point2]) -> impl Iterator<Item = (Point2, Point2)> + '_ {
    (0..points.len()).map(move |i| (points[i], points[(i + 1) % points.len()]))
}

fn segment_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.x - (a.x + t * dx)).hypot(p.y - (a.y + t * dy))
}

/// Distance from `p` to the boundary of `region`.
fn boundary_distance(p: Point2, region: &Region) -> f64 {
    region
        .polygons()
        .iter()
        .flat_map(rings)
        .flat_map(edges)
        .map(|(a, b)| segment_distance(p, a, b))
        .fold(f64::INFINITY, f64::min)
}

/// Whether `p` lies inside `points` by the crossing rule.
fn crosses(p: Point2, points: &[Point2]) -> bool {
    let mut inside = false;
    for (a, b) in edges(points) {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            inside = !inside;
        }
    }
    inside
}

/// Plan distance from `p` to `region`, zero inside. Only used to choose a
/// node near a subject, never to decide anything.
fn region_distance(p: Point2, region: &Region) -> f64 {
    let inside = region
        .polygons()
        .iter()
        .any(|polygon| rings(polygon).filter(|ring| crosses(p, ring)).count() % 2 == 1);
    if inside {
        0.0
    } else {
        boundary_distance(p, region)
    }
}

fn perimeter(polygon: &Polygon) -> f64 {
    rings(polygon)
        .flat_map(edges)
        .map(|(a, b)| (b.x - a.x).hypot(b.y - a.y))
        .sum()
}

/// Whether two regions share a part of positive area.
fn meets(a: &Region, b: &Region, tolerance: Tolerance) -> Result<bool, FreeSpaceError> {
    if a.is_empty() || b.is_empty() {
        return Ok(false);
    }
    Ok(!planar::intersection(a, b, tolerance)
        .map_err(|e| refused("contact", e))?
        .is_empty())
}

fn piece(polygon: &Polygon, tolerance: Tolerance) -> Result<Region, FreeSpaceError> {
    Region::new(vec![polygon.clone()], tolerance).map_err(|e| unavailable("piece", e))
}

impl AxiolidFreeSpaceService {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn circulation_map(
        &self,
        request: &CirculationRequest,
    ) -> Result<CirculationMap, FreeSpaceError> {
        let t = crate::free_space::tolerance()?;
        let radius = request.width_metres() / 2.0;
        let reach = radius + request.tolerance_metres();
        // The placement scene of the same scope, obstacles, doors and band,
        // shared with the placement searches of the check (#408).
        let derived = self.derived_scene(
            (request.scope(), request.merged_scopes()),
            (request.obstacles(), request.swept_doors()),
            request.band(),
            t,
        )?;
        let kept = derived.kept()?;
        let (scene, floor) = (kept.bare(), kept.floor);
        // The free area bracketed: the sure scope less every possible
        // obstacle (`free`), and the possible scope less the sure ones only
        // (`roomy`). They differ by the swept sectors' polygonisation and
        // by tessellations grown and shrunk by their chord deviation.
        let less = |scope: &Region, obstacles: &Region| {
            if obstacles.is_empty() {
                Ok(scope.clone())
            } else {
                planar::difference(scope, obstacles, t).map_err(|e| refused("free area", e))
            }
        };
        let free = less(&scene.scope, &scene.obstacles)?;
        let roomy = less(&scene.room, &scene.sure)?;
        let (inner, outer) = eroded_free(&scene, radius, &Work::default())?;
        let pieces: Vec<Region> = inner
            .polygons()
            .iter()
            .map(|polygon| piece(polygon, t))
            .collect::<Result<_, _>>()?;
        let possible: Vec<Region> = outer
            .polygons()
            .iter()
            .map(|polygon| piece(polygon, t))
            .collect::<Result<_, _>>()?;

        // Overlay output is snapped to a grid finer than this, relative to
        // the extent (axiolid/kernel#173).
        let extent = scene
            .scope
            .polygons()
            .iter()
            .flat_map(|polygon| polygon.outer.points.iter())
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(1.0, f64::max);
        let margin = KNIFE_EDGE_METRES.max(1.0e-7 * extent);

        let mut nodes = Vec::new();
        let mut links = Vec::new();
        let mut unmapped = Vec::new();
        let width = request.width_metres();
        let mut spacing: f64 = width / SAMPLES_PER_WIDTH;
        for (index, polygon) in inner.polygons().iter().enumerate() {
            let floor_spacing = perimeter(polygon) / MOST_SAMPLES;
            let first = (width / SAMPLES_PER_WIDTH).max(floor_spacing);
            let mut shape = match piece_skeleton(polygon, first) {
                Ok(found) => found,
                Err(why) => {
                    unmapped.push((index, format!("no skeleton: {why}")));
                    continue;
                }
            };
            let mut used = first;
            // A corridor of the erosion narrower than eight spacings can
            // break the skeleton apart; one finer sampling, down to half
            // its narrowest width, joins it again.
            let narrowest = shape
                .nodes
                .iter()
                .map(|node| node.clearance.0)
                .fold(f64::INFINITY, f64::min);
            let finer = narrowest.max(floor_spacing);
            if components(&shape) > 1
                && finer.is_finite()
                && finer < 0.9 * used
                && let Ok(joined) = piece_skeleton(polygon, finer)
                && components(&joined) < components(&shape)
            {
                shape = joined;
                used = finer;
            }
            spacing = spacing.max(used);
            let base = nodes.len();
            for node in &shape.nodes {
                let kind = match node.kind {
                    NodeKind::End => CirculationNodeKind::End,
                    NodeKind::Path => CirculationNodeKind::Path,
                    NodeKind::Junction => CirculationNodeKind::Junction,
                    NodeKind::Isolated => CirculationNodeKind::Isolated,
                    other => return Err(unavailable("skeleton node", other)),
                };
                // The node lies in `free`, which lies in `roomy`: the free
                // area's true boundary lies between theirs.
                let (near, far) = (
                    boundary_distance(node.point, &free),
                    boundary_distance(node.point, &roomy),
                );
                let half_width =
                    LengthInterval::try_new((near - margin).max(0.0), far.max(near) + margin)
                        .map_err(|e| unavailable("half width", e))?;
                nodes.push(CirculationNode::try_new(
                    [node.point.x, node.point.y, floor],
                    kind,
                    index,
                    half_width,
                )?);
            }
            links.extend(shape.edges.iter().map(|&(a, b)| (base + a, base + b)));
        }

        let mut contacts = Vec::new();
        let mut bracketed = scene.bracketed.clone();
        for subject in request.subjects() {
            let SubjectFootprint {
                inside,
                around,
                deviation,
            } = self.subject_footprint(&subject, t)?;
            if let Some(d) = deviation
                && !bracketed.iter().any(|(object, _)| *object == subject)
            {
                bracketed.push((subject.clone(), d));
            }
            let near = grown(&inside, reach, true, t)?;
            let around = around?;
            let far = grown(&around, reach, false, t)?;
            let mut reached = Vec::new();
            for (index, region) in pieces.iter().enumerate() {
                if meets(region, &near, t)? {
                    let nearest = nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, node)| node.piece() == index)
                        .map(|(at, node)| {
                            let [x, y, _] = node.point();
                            (region_distance(Point2::new(x, y), &around), at)
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                        .map(|(_, at)| at);
                    reached.push((index, nearest));
                }
            }
            let mut maybe = Vec::new();
            for (index, region) in possible.iter().enumerate() {
                if meets(region, &far, t)? {
                    maybe.push(index);
                }
            }
            contacts.push(CirculationContact::new(subject, reached, maybe));
        }

        let mut locator = format!("axiolid:circulation:{}", request.scope().local_id);
        for merged in request.merged_scopes() {
            locator.push('+');
            locator.push_str(&merged.to_string());
        }
        locator.push_str(&crate::free_space::within_deviation(&bracketed));
        locator.push_str(&crate::free_space::within_bounds(&scene.bounded));
        locator.push_str(&crate::free_space::without_shape(&scene.shapeless));
        let evidence = Evidence::exact(request.scope().source.clone(), locator);
        CirculationMap::try_new(
            request.clone(),
            pieces.len(),
            possible.len(),
            nodes,
            links,
            spacing,
            contacts,
            unmapped,
            evidence,
        )
    }

    /// The pieces of `scene`'s free area eroded by half `reach`'s width
    /// that come near one of its entrances: surely (pieces inside the exact
    /// erosion meeting an entrance's footprint grown from inside) and
    /// possibly (pieces containing it meeting the footprint grown from
    /// outside), as a circulation map's contacts are decided.
    pub(crate) fn reached(
        &self,
        scene: &Scene,
        reach: &EntranceReach,
    ) -> Result<Reached, FreeSpaceError> {
        let t = scene.tolerance;
        let radius = reach.width_metres() / 2.0;
        let near = radius + reach.tolerance_metres();
        let (inner, outer) = eroded_free(scene, radius, &scene.work)?;
        let mut sure = Vec::new();
        let mut maybe = Vec::new();
        let mut bracketed = Vec::new();
        for entrance in reach.entrances() {
            let SubjectFootprint {
                inside,
                around,
                deviation,
            } = self.subject_footprint(entrance, t)?;
            if let Some(d) = deviation {
                bracketed.push((entrance.clone(), d));
            }
            let grown_in = grown(&inside, near, true, t)?;
            let grown_out = grown(&around?, near, false, t)?;
            for polygon in inner.polygons() {
                if !sure.contains(polygon) && meets(&piece(polygon, t)?, &grown_in, t)? {
                    sure.push(polygon.clone());
                }
            }
            for polygon in outer.polygons() {
                if !maybe.contains(polygon) && meets(&piece(polygon, t)?, &grown_out, t)? {
                    maybe.push(polygon.clone());
                }
            }
        }
        let region = |polygons: Vec<Polygon>| {
            if polygons.is_empty() {
                Ok(Region::empty())
            } else {
                Region::new(polygons, t).map_err(|e| unavailable("reached pieces", e))
            }
        };
        Ok(Reached {
            inner: region(sure)?,
            outer: region(maybe)?,
            bracketed,
        })
    }

    /// The plan footprint of an entrance or component, bracketed around the
    /// shadows the overlay refuses (`placement::bracketed_footprint`,
    /// #377): `inside` grown from inside for what it surely meets,
    /// `around` from outside for what it may (an error where a sliver has
    /// no box: only what needs the outer footprint is refused).
    ///
    /// A tessellated subject is also bracketed by its chord deviation `d`,
    /// as a tessellated obstacle is (#302, [`Measuring::floor_scene`]):
    /// `around` is the mesh's footprint grown by `d`, and `inside` the
    /// eroded core of the plan points whose vertical column of height `2d`
    /// about the body's middle lies in the mesh, empty where the body is
    /// no taller than that (nothing is then surely reached through it).
    /// An opening is measured by its void (#383), bracketed alike when that
    /// is tessellated.
    fn subject_footprint(
        &self,
        subject: &ObjectId,
        tolerance: Tolerance,
    ) -> Result<SubjectFootprint, FreeSpaceError> {
        let missing = || FreeSpaceError::MissingGeometry(Box::new(subject.clone()));
        let exact = |mesh: &TriMesh| {
            let (inside, around) = placement::bracketed_footprint(&triangles(mesh), tolerance)?;
            if around.as_ref().is_ok_and(Region::is_empty) {
                return Err(missing());
            }
            Ok(SubjectFootprint {
                inside,
                around,
                deviation: None,
            })
        };
        if self.geometry().has_no_body(subject) {
            // An opening is reached through its void (#383): the solid of
            // space it keeps free, asked for by name as an extent is.
            let void = match self.geometry().void(subject) {
                Some(Ok(void)) => void,
                Some(Err(reason)) => {
                    return Err(FreeSpaceError::Unavailable(format!(
                        "{subject}'s void is unmeasured ({reason}), so where a path meets it is unknown"
                    )));
                }
                None => {
                    return Err(FreeSpaceError::Unavailable(format!(
                        "{subject} has no body, so where a path meets it is unknown"
                    )));
                }
            };
            // A tessellated void is bracketed by its chord deviation as a
            // tessellated body is, its column read from the void itself.
            let d = match self.geometry().void_fidelity(subject) {
                Ok(GeometryFidelity::Exact) => return exact(void),
                Ok(fidelity) => Some(fidelity.deviation_metres()),
                Err(_) => None,
            }
            .filter(|d| *d > 0.0)
            .ok_or(FreeSpaceError::InexactPlacementEvidence)?;
            return Self::tessellated_footprint(subject, void, d, tolerance, |level, half| {
                walkable::solid_column(subject, void, level, half)
            });
        }
        let mesh = self.geometry().mesh(subject).ok_or_else(missing)?;
        if !self.geometry().is_tessellated(subject) {
            return exact(mesh);
        }
        let d = self
            .geometry()
            .deviation(subject)
            .filter(|d| *d > 0.0)
            .ok_or(FreeSpaceError::InexactPlacementEvidence)?;
        Self::tessellated_footprint(subject, mesh, d, tolerance, |level, half| {
            self.mesh_column(subject, level, half)
        })
    }

    /// The footprint of a subject tessellated within `d` of its true shape
    /// (`mesh`), bracketed as [`Self::subject_footprint`] describes, with
    /// `column` the plan section and shadow of the mesh's solid column of
    /// half height `half` about `level`.
    fn tessellated_footprint(
        subject: &ObjectId,
        mesh: &TriMesh,
        d: f64,
        tolerance: Tolerance,
        column: impl FnOnce(f64, f64) -> Result<(Plan, Plan), String>,
    ) -> Result<SubjectFootprint, FreeSpaceError> {
        let missing = || FreeSpaceError::MissingGeometry(Box::new(subject.clone()));
        let (low, high) = mesh_extent(mesh).ok_or_else(missing)?;
        // Morphology runs in a plan frame at the body's corner, as a
        // tessellated obstacle's does: at georeferenced coordinates the
        // overlay refuses the slivers its own rounding makes.
        let far = low[0].abs().max(low[1].abs()) >= 1.0e5;
        let origin = if far {
            Vec2::new(low[0].floor(), low[1].floor())
        } else {
            Vec2::new(0.0, 0.0)
        };
        let moved: Vec<_> = triangles(mesh)
            .into_iter()
            .map(|face| face.map(|p| Point3::new(p.x - origin.x, p.y - origin.y, p.z)))
            .collect();
        let region_error = |e| unavailable("bracketed footprint", e);
        let back = |region: Region| {
            if region.is_empty() || !far {
                Ok(region)
            } else {
                region.translate(origin).map_err(region_error)
            }
        };
        let (_, around) = placement::bracketed_footprint(&moved, tolerance)?;
        let around = around.and_then(|footprint| {
            if footprint.is_empty() {
                return Err(missing());
            }
            let grown = planar::morphology(&footprint, Morphology::DilateOuter, d, tolerance)
                .map_err(|e| refused("disc morphology", e))?;
            back(grown)
        });
        let level = f64::midpoint(low[2], high[2]);
        let mut inside = Region::empty();
        if level - d > low[2] && level + d < high[2] {
            let (section, shadow) = column(level, d).map_err(FreeSpaceError::Unavailable)?;
            let core = plan_region(&section, origin, false, tolerance)?
                .difference(&plan_region(&shadow, origin, true, tolerance)?, tolerance)
                .map_err(region_error)?;
            if !core.is_empty() {
                let eroded = planar::morphology(&core, Morphology::ErodeInner, d, tolerance)
                    .map_err(|e| refused("disc morphology", e))?;
                inside = back(eroded)?;
            }
        }
        Ok(SubjectFootprint {
            inside,
            around,
            deviation: Some(d),
        })
    }
}

/// A subject's plan footprint bracketed ([`AxiolidFreeSpaceService::subject_footprint`]).
struct SubjectFootprint {
    inside: Region,
    around: Result<Region, FreeSpaceError>,
    /// The chord deviation it was bracketed by, for a tessellated subject.
    deviation: Option<f64>,
}

#[cfg(test)]
mod budget_tests {
    use axiolid_core::{Point2, Tolerance};
    use axiolid_overlay::{Polygon, Region, Ring};
    use axioval_engine::FreeSpaceError;

    use crate::planar::budget::{DISC_PAIRS, REGION_PAIRS};
    use crate::planar::sliver_polygons;

    /// A subject's footprint grown by the reach, and a contact or free
    /// area between regions, past their budgets are refused by name before
    /// they are built (#432).
    #[test]
    fn contacts_past_their_budgets_are_refused_by_name() {
        let t = Tolerance::new(1e-9, 1e-9).unwrap();
        let crowded = Region::new(sliver_polygons(), t).unwrap();
        let p = Point2::new;
        let room = Region::new(
            vec![Polygon {
                outer: Ring {
                    points: vec![p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)],
                },
                holes: Vec::new(),
            }],
            t,
        )
        .unwrap();
        let message = |error: FreeSpaceError| match error {
            FreeSpaceError::Unavailable(message) => message,
            other => panic!("{other:?}"),
        };
        for inner in [true, false] {
            let refused = message(super::grown(&crowded, 0.5, inner, t).unwrap_err());
            assert!(
                refused.starts_with("disc morphology: ")
                    && refused.ends_with(&format!(
                        "past the disc morphology budget of {DISC_PAIRS} pairs"
                    )),
                "{refused}"
            );
        }
        let refused = message(super::meets(&room, &crowded, t).unwrap_err());
        assert!(
            refused.starts_with("contact: ")
                && refused.ends_with(&format!("past the overlay budget of {REGION_PAIRS} pairs")),
            "{refused}"
        );
        assert!(super::meets(&room, &room, t).unwrap());
    }
}
