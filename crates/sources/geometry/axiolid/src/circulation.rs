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
//! Door swings are not subtracted: a door's leaf and its sweep are not
//! known (the source reports no swing), so only what the obstacles occupy
//! counts.

use axiolid_core::{Point2, Tolerance};
use axiolid_overlay::{Polygon, Region};
use axiolid_route::{NodeKind, Skeleton, skeleton};
use axioval_engine::{
    CirculationContact, CirculationMap, CirculationNode, CirculationNodeKind, CirculationRequest,
    FreeSpaceError, LengthInterval,
};
use axioval_ir::{Evidence, ObjectId};

use crate::free_space::AxiolidFreeSpaceService;
use crate::geometry::triangles;
use crate::placement::{self, KNIFE_EDGE_METRES};

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
    Ok(!a
        .intersection(b, tolerance)
        .map_err(|e| unavailable("contact", e))?
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
        let err = |e| unavailable("disc morphology", e);
        let radius = request.width_metres() / 2.0;
        let reach = radius + request.tolerance_metres();
        let (scene, floor) = self.floor_scene(
            request.scope(),
            &[],
            request.obstacles(),
            request.band(),
            request.width_metres() + request.tolerance_metres(),
            t,
        )?;
        let free = if scene.obstacles.is_empty() {
            scene.scope.clone()
        } else {
            scene
                .scope
                .difference(&scene.obstacles, t)
                .map_err(|e| unavailable("free area", e))?
        };
        let mut inner = scene.scope.erode_inner(radius, t).map_err(err)?;
        if !scene.obstacles.is_empty() && !inner.is_empty() {
            inner = inner
                .difference(&scene.obstacles.dilate_outer(radius, t).map_err(err)?, t)
                .map_err(err)?;
        }
        // For a radius a knife edge smaller, so that a gap exactly as wide
        // as the path keeps a sliver rather than being snapped shut.
        let shrunk = radius - KNIFE_EDGE_METRES;
        let mut outer = scene.scope.erode_outer(shrunk, t).map_err(err)?;
        if !scene.obstacles.is_empty() && !outer.is_empty() {
            outer = outer
                .difference(&scene.obstacles.dilate_inner(shrunk, t).map_err(err)?, t)
                .map_err(err)?;
        }
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
                let distance = boundary_distance(node.point, &free);
                let half_width =
                    LengthInterval::try_new((distance - margin).max(0.0), distance + margin)
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
        for subject in request.subjects() {
            let footprint = self.subject_footprint(&subject, t)?;
            let near = footprint.dilate_inner(reach, t).map_err(err)?;
            let far = footprint.dilate_outer(reach, t).map_err(err)?;
            let mut reached = Vec::new();
            for (index, region) in pieces.iter().enumerate() {
                if meets(region, &near, t)? {
                    let nearest = nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, node)| node.piece() == index)
                        .map(|(at, node)| {
                            let [x, y, _] = node.point();
                            (region_distance(Point2::new(x, y), &footprint), at)
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

        let evidence = Evidence::exact(
            request.scope().source.clone(),
            format!("axiolid:circulation:{}", request.scope().local_id),
        );
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

    /// The plan footprint of an entrance or component.
    fn subject_footprint(
        &self,
        subject: &ObjectId,
        tolerance: Tolerance,
    ) -> Result<Region, FreeSpaceError> {
        if self.geometry().has_no_body(subject) {
            return Err(FreeSpaceError::Unavailable(format!(
                "{subject} has no body, so where a path meets it is unknown"
            )));
        }
        let mesh = self
            .geometry()
            .mesh(subject)
            .ok_or_else(|| FreeSpaceError::MissingGeometry(Box::new(subject.clone())))?;
        if self.geometry().is_tessellated(subject) {
            return Err(FreeSpaceError::InexactPlacementEvidence);
        }
        let footprint = placement::footprint(&triangles(mesh), tolerance)?;
        if footprint.is_empty() {
            return Err(FreeSpaceError::MissingGeometry(Box::new(subject.clone())));
        }
        Ok(footprint)
    }
}
