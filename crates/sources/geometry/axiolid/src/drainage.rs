//! How water runs off a surface's top to its outlets.
//!
//! Answers [`axioval_engine::ProximityService::measure_drainage`]. The top
//! is the surface's faces looking up (outward on a closed body, either way
//! on an open sheet, turned up), joined where they share an edge by exact
//! coordinates. Water falls on each face and runs down the face's steepest
//! descent; where it crosses an edge it runs on into the face beyond, or,
//! where that face drains into the same edge, along the edge (a valley) to
//! its lower end; from a vertex it runs on down any way that may be the
//! steepest. It stops where it reaches an outlet's footprint (the convex
//! hull in plan of the outlet's body), falls off the top's border, or comes
//! to rest: on a face falling less than the minimum, on a level edge, or in
//! a hollow no way leads down from.
//!
//! Every place water may be is a node (a face, an edge, a vertex), each with
//! every way water may go on from it. Where a direction is decided within
//! rounding there is one way; where it is not (a face level within rounding,
//! a flow along an edge, a footprint's edge) every way it may take is kept,
//! together with resting there. A face surely drains when all the ways from
//! it surely end in an outlet, and surely does not when none may; the
//! undrained area runs from the faces surely not draining up to every face
//! not surely draining. Nothing undecided is ever counted as draining.
//!
//! The longest path is bounded from both sides. Inside a face water runs
//! straight, so the length to the face's exit plus the run along a valley
//! to its end is linear over each part of the face draining through one
//! edge: its greatest is at a corner, or where the flow line through a
//! corner leaves the face backwards. The upper bound takes that over every
//! way on; the lower bound, over faces surely draining, is the decided run
//! to the first edge and on along a valley, and never less than the plan
//! distance to the nearest outlet. A tessellated surface states its true
//! faces only within its chord deviation, which can hide a hollow anywhere,
//! so none of its area surely drains and its path is not bounded.

use std::collections::HashMap;

use axioval_engine::{DrainageEvidence, DrainagePath, DrainageRequest, ProximityError};
use axioval_ir::Evidence;

use crate::proximity::AxiolidProximityService;

/// A face looks up when its unit normal rises more than this.
const UP: f64 = 1e-9;

/// How close a unit quantity (a direction's component, a fall) may come
/// to a decision's threshold before it is undecided.
const TURN: f64 = 1e-9;

/// How close in metres a position may come to a decision's threshold
/// before it is undecided.
const LINEAR: f64 = 1e-9;

/// How close the share of a face's flow across an edge may come to zero
/// before the flow runs along the edge. The positions are exact, so this
/// only holds the arithmetic's rounding; water running along a border
/// within it could carry off at most a sliver of that width, which the
/// areas' rounding holds.
const ALONG: f64 = 1e-12;

/// At most this many faces looking up are traced.
const MAX_FACES: usize = 200_000;

type P2 = [f64; 2];
type P3 = [f64; 3];

/// Measures the request.
pub(crate) fn measure(
    service: &AxiolidProximityService,
    request: &DrainageRequest,
) -> Result<DrainageEvidence, ProximityError> {
    let body = service.body(request.surface())?;
    let fidelity = service.fidelity(request.surface())?;
    let soup: Vec<[P3; 3]> = body
        .soup
        .items
        .iter()
        .map(|triangle| triangle.map(|point| point.to_array()))
        .collect();
    let tops = top_faces(&soup, body.solid)?;
    if tops.is_empty() {
        return Err(ProximityError::Refused(
            "the surface has no face looking up, so no water rests on it",
        ));
    }
    if tops.len() > MAX_FACES {
        return Err(ProximityError::Refused(
            "the surface has too many faces looking up to trace its drainage",
        ));
    }
    let mut exact = fidelity.is_exact();
    let mut hulls = Vec::new();
    for outlet in request.outlets() {
        let outlet_body = service.body(outlet)?;
        let outlet_fidelity = service.fidelity(outlet)?;
        exact &= outlet_fidelity.is_exact();
        let points: Vec<P2> = outlet_body
            .soup
            .items
            .iter()
            .flatten()
            .map(|point| [point.x, point.y])
            .collect();
        hulls.push(Hull::of(&points, outlet_fidelity.deviation_metres()).ok_or(
            ProximityError::Refused("an outlet has no footprint in plan to take water"),
        )?);
    }
    let tin = Tin::new(&tops, &soup)?;
    let analysis = if request.outlets().is_empty() {
        let area = tin.area();
        Analysis {
            area,
            undrained: area,
            path: DrainagePath::NoOutlet,
            decided: true,
        }
    } else if fidelity.is_exact() {
        tin.analyse(&hulls, request.minimum_fall())
    } else {
        let area = tin.area();
        exact = false;
        Analysis {
            area,
            undrained: (0.0, area.1),
            path: DrainagePath::Unbounded,
            decided: false,
        }
    };
    let outlets: Vec<String> = request.outlets().iter().map(ToString::to_string).collect();
    DrainageEvidence::try_new(
        request.clone(),
        analysis.area,
        analysis.undrained,
        analysis.path,
        Evidence {
            source: request.surface().source.clone(),
            locator: format!(
                "axiolid:drainage:{}:{}:fall={}",
                request.surface(),
                outlets.join("+"),
                request.minimum_fall()
            ),
            exact: exact && analysis.decided,
        },
    )
}

/// The relative rounding sums of areas and lengths are widened by.
const ROUNDING: f64 = 1e-12;

/// The faces of `soup` looking up, wound anticlockwise seen from above.
fn top_faces(soup: &[[P3; 3]], solid: bool) -> Result<Vec<[P3; 3]>, ProximityError> {
    let orientation = if solid {
        let volume: f64 = soup.iter().map(|[a, b, c]| dot(*a, cross(*b, *c))).sum();
        if !volume.is_finite() || volume == 0.0 {
            return Err(ProximityError::Refused(
                "the surface's body encloses no volume, so which way its faces look is unknown",
            ));
        }
        volume.signum()
    } else {
        0.0
    };
    let mut tops = Vec::new();
    for &[a, b, c] in soup {
        let normal = cross(minus(b, a), minus(c, a));
        let length = norm(normal);
        if length == 0.0 || !length.is_finite() {
            continue;
        }
        let up = normal[2] / length;
        if solid {
            if up * orientation > UP {
                tops.push(if orientation > 0.0 {
                    [a, b, c]
                } else {
                    [a, c, b]
                });
            }
        } else if up > UP {
            tops.push([a, b, c]);
        } else if up < -UP {
            tops.push([a, c, b]);
        }
    }
    Ok(tops)
}

/// What became of the water: the top's plan area, the plan area whose
/// water reaches no outlet, the longest path, and whether every face's
/// fate is decided.
struct Analysis {
    area: (f64, f64),
    undrained: (f64, f64),
    path: DrainagePath,
    decided: bool,
}

/// An outlet's footprint: the convex hull in plan of its body,
/// anticlockwise, which may be off by `margin` all round.
struct Hull {
    points: Vec<P2>,
    margin: f64,
}

impl Hull {
    /// The hull of `points`; `None` where it encloses no area.
    fn of(points: &[P2], margin: f64) -> Option<Self> {
        let mut sorted = points.to_vec();
        sorted.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        sorted.dedup();
        if sorted.len() < 3 {
            return None;
        }
        let turn =
            |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
        let mut lower: Vec<P2> = Vec::new();
        for point in &sorted {
            while lower.len() >= 2
                && turn(lower[lower.len() - 2], lower[lower.len() - 1], *point) <= 0.0
            {
                lower.pop();
            }
            lower.push(*point);
        }
        let mut upper: Vec<P2> = Vec::new();
        for point in sorted.iter().rev() {
            while upper.len() >= 2
                && turn(upper[upper.len() - 2], upper[upper.len() - 1], *point) <= 0.0
            {
                upper.pop();
            }
            upper.push(*point);
        }
        lower.pop();
        upper.pop();
        lower.extend(upper);
        let hull = Self {
            points: lower,
            margin,
        };
        (hull.points.len() >= 3 && hull.area() > LINEAR * LINEAR).then_some(hull)
    }

    fn area(&self) -> f64 {
        let n = self.points.len();
        (0..n)
            .map(|i| cross2(self.points[i], self.points[(i + 1) % n]))
            .sum::<f64>()
            / 2.0
    }

    /// The unit outward normals of the hull's edges, each with its offset.
    fn edges(&self) -> impl Iterator<Item = (P2, f64)> + '_ {
        let n = self.points.len();
        (0..n).filter_map(move |i| {
            let (a, b) = (self.points[i], self.points[(i + 1) % n]);
            let along = [b[0] - a[0], b[1] - a[1]];
            let length = along[0].hypot(along[1]);
            (length > 0.0).then(|| {
                let normal = [along[1] / length, -along[0] / length];
                (normal, dot2(normal, a))
            })
        })
    }

    /// How deep `point` lies inside: its least distance to an edge's line,
    /// negative outside.
    fn depth(&self, point: P2) -> f64 {
        self.edges()
            .map(|(normal, offset)| offset - dot2(normal, point))
            .fold(f64::INFINITY, f64::min)
    }

    /// The slack beyond which a position is decided against the hull.
    fn slack(&self) -> f64 {
        self.margin + LINEAR
    }

    /// Whether every one of `points` surely lies inside.
    fn holds(&self, points: &[P2]) -> bool {
        points.iter().all(|point| self.depth(*point) > self.slack())
    }

    /// Whether the convex hull of `points` may meet the footprint.
    fn may_meet(&self, points: &[P2]) -> bool {
        let separated = |normal: P2| {
            let project = |set: &mut dyn Iterator<Item = P2>| {
                set.map(|point| dot2(normal, point))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                        (low.min(value), high.max(value))
                    })
            };
            let (a_low, a_high) = project(&mut self.points.iter().copied());
            let (b_low, b_high) = project(&mut points.iter().copied());
            b_low - a_high > self.slack() || a_low - b_high > self.slack()
        };
        let mut axes: Vec<P2> = self.edges().map(|(normal, _)| normal).collect();
        let n = points.len();
        for i in 0..n {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let along = [b[0] - a[0], b[1] - a[1]];
            let length = along[0].hypot(along[1]);
            if length > 0.0 {
                axes.push([along[1] / length, -along[0] / length]);
            }
        }
        !axes.into_iter().any(separated)
    }

    /// The plan area of the triangle `plan` the footprint, grown by its
    /// slack, may hold.
    fn cut(&self, plan: [P2; 3]) -> f64 {
        let mut polygon = plan.to_vec();
        for (normal, offset) in self.edges() {
            let limit = offset + self.slack();
            let mut kept = Vec::with_capacity(polygon.len() + 1);
            for (index, point) in polygon.iter().enumerate() {
                let next = polygon[(index + 1) % polygon.len()];
                let (here, there) = (dot2(normal, *point) - limit, dot2(normal, next) - limit);
                if here <= 0.0 {
                    kept.push(*point);
                }
                if (here < 0.0 && there > 0.0) || (here > 0.0 && there < 0.0) {
                    let share = here / (here - there);
                    kept.push([
                        point[0] + (next[0] - point[0]) * share,
                        point[1] + (next[1] - point[1]) * share,
                    ]);
                }
            }
            polygon = kept;
            if polygon.len() < 3 {
                return 0.0;
            }
        }
        let first = polygon[0];
        polygon
            .windows(2)
            .skip(1)
            .map(|pair| {
                cross2(
                    [pair[0][0] - first[0], pair[0][1] - first[1]],
                    [pair[1][0] - first[0], pair[1][1] - first[1]],
                )
            })
            .sum::<f64>()
            .abs()
            / 2.0
    }

    /// The least share along the segment from `from` to `to` at which it may
    /// enter the footprint; `None` where it surely misses it.
    fn entry(&self, from: P2, to: P2) -> Option<f64> {
        let (mut low, mut high) = (0.0_f64, 1.0_f64);
        let along = [to[0] - from[0], to[1] - from[1]];
        for (normal, offset) in self.edges() {
            // normal . (from + u along) <= offset + slack
            let start = dot2(normal, from) - offset - self.slack();
            let rate = dot2(normal, along);
            if rate.abs() <= f64::EPSILON {
                if start > 0.0 {
                    return None;
                }
            } else if rate > 0.0 {
                high = high.min(-start / rate);
            } else {
                low = low.max(-start / rate);
            }
        }
        (low <= high).then_some(low)
    }

    /// The plan distance from `point` to the footprint, at least zero,
    /// less the footprint's margin.
    fn distance(&self, point: P2) -> f64 {
        if self.depth(point) >= 0.0 {
            return 0.0;
        }
        let n = self.points.len();
        let nearest = (0..n)
            .map(|i| segment_distance(point, self.points[i], self.points[(i + 1) % n]))
            .fold(f64::INFINITY, f64::min);
        (nearest - self.slack()).max(0.0)
    }
}

fn segment_distance(point: P2, a: P2, b: P2) -> f64 {
    let along = [b[0] - a[0], b[1] - a[1]];
    let length = dot2(along, along);
    let share = if length > 0.0 {
        (dot2([point[0] - a[0], point[1] - a[1]], along) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let foot = [a[0] + along[0] * share, a[1] + along[1] * share];
    (point[0] - foot[0]).hypot(point[1] - foot[1])
}

/// What an edge of the top is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Border {
    /// Two faces of the top meet along it.
    Inner,
    /// The top ends there: water crossing it leaves the surface.
    Edge,
    /// The top may go on there (a T-junction, a face without area beside
    /// it): where water goes is unknown.
    Unknown,
}

/// One edge of the top: its ends, the faces using it with the index of the
/// edge in each, and what it is.
struct EdgeInfo {
    ends: [usize; 2],
    faces: Vec<(usize, usize)>,
    border: Border,
}

/// One face's plane: its gradient, fall, whether it falls more than the
/// minimum, and the direction water runs on it, in plan.
#[derive(Clone, Copy)]
struct Plane {
    fall: f64,
    steep: Option<bool>,
    direction: Option<P2>,
    area: f64,
}

/// How water meets an edge of a face.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Crossing {
    /// It leaves the face over the edge.
    Out,
    /// It comes into the face over the edge.
    In,
    /// It runs along the edge, within rounding.
    Along,
}

/// The top as a triangulated network.
struct Tin {
    points: Vec<P3>,
    faces: Vec<[usize; 3]>,
    planes: Vec<Plane>,
    edges: Vec<EdgeInfo>,
    face_edges: Vec<[usize; 3]>,
    vertex_faces: Vec<Vec<(usize, usize)>>,
    vertex_edges: Vec<Vec<usize>>,
}

/// Marks the border edges of the top where it may go on: an edge a face
/// without area anywhere in the body shares, or one along which another
/// border edge runs (a T-junction). A border edge leaves the top only
/// where nothing may continue it.
fn unknown_borders(edges: &mut [EdgeInfo], points: &[P3], soup: &[[P3; 3]]) {
    let sorted = |[first, second]: [[u64; 3]; 2]| {
        if first <= second {
            [first, second]
        } else {
            [second, first]
        }
    };
    let mut slivers: std::collections::HashSet<[[u64; 3]; 2]> = std::collections::HashSet::new();
    for &[a, b, c] in soup {
        if norm(cross(minus(b, a), minus(c, a))) == 0.0 {
            for pair in [[a, b], [b, c], [c, a]] {
                slivers.insert(sorted(pair.map(key)));
            }
        }
    }
    let borders: Vec<usize> = (0..edges.len())
        .filter(|at| edges[*at].border == Border::Edge)
        .collect();
    for &at in &borders {
        let ends = edges[at].ends.map(|end| points[end]);
        let unknown = slivers.contains(&sorted(ends.map(key)))
            || borders.iter().any(|&other| {
                other != at && overlap(ends, edges[other].ends.map(|end| points[end]))
            });
        if unknown {
            edges[at].border = Border::Unknown;
        }
    }
}

/// A position's key, `-0.0` read as `0.0`.
fn key(point: P3) -> [u64; 3] {
    point.map(|value| (value + 0.0).to_bits())
}

impl Tin {
    fn new(tops: &[[P3; 3]], soup: &[[P3; 3]]) -> Result<Self, ProximityError> {
        let mut index: HashMap<[u64; 3], usize> = HashMap::new();
        let mut points = Vec::new();
        let mut vertex = |point: P3| {
            *index.entry(key(point)).or_insert_with(|| {
                points.push(point);
                points.len() - 1
            })
        };
        let faces: Vec<[usize; 3]> = tops.iter().map(|face| face.map(&mut vertex)).collect();
        let mut edge_index: HashMap<[usize; 2], usize> = HashMap::new();
        let mut edges: Vec<EdgeInfo> = Vec::new();
        let mut face_edges = Vec::with_capacity(faces.len());
        for (face, corners) in faces.iter().enumerate() {
            let mut own = [0; 3];
            for local in 0..3 {
                let (a, b) = (corners[local], corners[(local + 1) % 3]);
                let ends = [a.min(b), a.max(b)];
                let at = *edge_index.entry(ends).or_insert_with(|| {
                    edges.push(EdgeInfo {
                        ends,
                        faces: Vec::new(),
                        border: Border::Edge,
                    });
                    edges.len() - 1
                });
                edges[at].faces.push((face, local));
                own[local] = at;
            }
            face_edges.push(own);
        }
        for edge in &mut edges {
            match edge.faces.len() {
                1 => {}
                2 => {
                    let [(first, i), (second, j)] = [edge.faces[0], edge.faces[1]];
                    let from = |face: usize, local: usize| faces[face][local];
                    if from(first, i) == from(second, j) {
                        return Err(ProximityError::Refused(
                            "the surface's top folds over itself along an edge",
                        ));
                    }
                    edge.border = Border::Inner;
                }
                _ => {
                    return Err(ProximityError::Refused(
                        "more than two faces of the surface's top meet along an edge",
                    ));
                }
            }
        }
        unknown_borders(&mut edges, &points, soup);
        let mut vertex_faces = vec![Vec::new(); points.len()];
        for (face, corners) in faces.iter().enumerate() {
            for (corner, point) in corners.iter().enumerate() {
                vertex_faces[*point].push((face, corner));
            }
        }
        let mut vertex_edges = vec![Vec::new(); points.len()];
        for (at, edge) in edges.iter().enumerate() {
            for end in edge.ends {
                vertex_edges[end].push(at);
            }
        }
        let planes = Vec::new();
        Ok(Self {
            points,
            faces,
            planes,
            edges,
            face_edges,
            vertex_faces,
            vertex_edges,
        })
    }

    fn corners(&self, face: usize) -> [P3; 3] {
        self.faces[face].map(|point| self.points[point])
    }

    fn plan(&self, face: usize) -> [P2; 3] {
        self.corners(face).map(|[x, y, _]| [x, y])
    }

    /// The plan area of the top, rounded outward.
    fn area(&self) -> (f64, f64) {
        let sum: f64 = (0..self.faces.len())
            .map(|face| plan_area(self.plan(face)))
            .sum();
        widen(sum)
    }

    fn plane(&self, face: usize, minimum_fall: f64) -> Plane {
        let [p0, p1, p2] = self.corners(face);
        let (d1, d2) = (minus(p1, p0), minus(p2, p0));
        let doubled = d1[0] * d2[1] - d1[1] * d2[0];
        let a = (d1[2] * d2[1] - d2[2] * d1[1]) / doubled;
        let b = (d1[0] * d2[2] - d2[0] * d1[2]) / doubled;
        let fall = a.hypot(b);
        let steep = if fall - minimum_fall > TURN {
            Some(true)
        } else if minimum_fall - fall > TURN {
            Some(false)
        } else {
            None
        };
        let direction = (fall > TURN).then(|| [-a / fall, -b / fall]);
        Plane {
            fall,
            steep,
            direction,
            area: doubled / 2.0,
        }
    }

    /// The unit outward normal in plan of `face`'s edge `local`.
    fn outward(&self, face: usize, local: usize) -> P2 {
        let plan = self.plan(face);
        let (a, b) = (plan[local], plan[(local + 1) % 3]);
        let along = [b[0] - a[0], b[1] - a[1]];
        let length = along[0].hypot(along[1]);
        [along[1] / length, -along[0] / length]
    }

    /// How water on `face` meets its edge `local`; `None` where it has no
    /// direction.
    fn crossing(&self, face: usize, local: usize) -> Option<Crossing> {
        let direction = self.planes[face].direction?;
        let share = dot2(direction, self.outward(face, local));
        Some(if share > ALONG {
            Crossing::Out
        } else if share < -ALONG {
            Crossing::In
        } else {
            Crossing::Along
        })
    }

    /// The node of `face`.
    fn face_node(face: usize) -> usize {
        face
    }

    fn edge_node(&self, edge: usize) -> usize {
        self.faces.len() + edge
    }

    fn vertex_node(&self, point: usize) -> usize {
        self.faces.len() + self.edges.len() + point
    }

    /// Where water leaving `face` over its edge `local` may go.
    fn across(&self, face: usize, local: usize) -> Vec<usize> {
        let edge = self.face_edges[face][local];
        let info = &self.edges[edge];
        if info.border != Border::Inner {
            return vec![self.edge_node(edge)];
        }
        let (other, at) = info
            .faces
            .iter()
            .copied()
            .find(|(other, _)| *other != face)
            .unwrap_or((face, local));
        let mut ways = Vec::new();
        if self.planes[other].steep != Some(true) {
            ways.push(Self::face_node(other));
        }
        match self.crossing(other, at) {
            Some(Crossing::In) => ways.push(Self::face_node(other)),
            Some(Crossing::Out) => ways.push(self.edge_node(edge)),
            None | Some(Crossing::Along) => {
                ways.push(Self::face_node(other));
                ways.push(self.edge_node(edge));
            }
        }
        ways.sort_unstable();
        ways.dedup();
        ways
    }

    /// Every node with the ways water may go on from it.
    fn nodes(&self, hulls: &[Hull]) -> Vec<Node> {
        let mut nodes = Vec::with_capacity(self.faces.len() + self.edges.len() + self.points.len());
        for face in 0..self.faces.len() {
            let plan = self.plan(face);
            let mut node = Node::captured(hulls, &plan);
            let plane = self.planes[face];
            if !node.must_capture() {
                if plane.steep == Some(false) {
                    node.rest = node.unless_caught();
                } else {
                    if plane.steep.is_none() {
                        node.rest = Chance::May;
                    }
                    for local in 0..3 {
                        if matches!(self.crossing(face, local), None | Some(Crossing::Out)) {
                            node.ways.extend(self.across(face, local));
                        }
                    }
                }
            }
            nodes.push(node.settled());
        }
        for edge in &self.edges {
            let [a, b] = edge.ends.map(|end| self.points[end]);
            let mut node = Node::captured(hulls, &[[a[0], a[1]], [b[0], b[1]]]);
            if !node.must_capture() {
                match edge.border {
                    Border::Edge => {
                        node.rest = node.unless_caught();
                    }
                    Border::Unknown => {
                        node.capture = Chance::May;
                        node.rest = Chance::May;
                    }
                    Border::Inner => {
                        let rise = b[2] - a[2];
                        if rise < -LINEAR {
                            node.ways.push(self.vertex_node(edge.ends[1]));
                        } else if rise > LINEAR {
                            node.ways.push(self.vertex_node(edge.ends[0]));
                        } else {
                            node.rest = Chance::May;
                            node.ways.push(self.vertex_node(edge.ends[0]));
                            node.ways.push(self.vertex_node(edge.ends[1]));
                        }
                        for &(face, local) in &edge.faces {
                            if self.planes[face].steep != Some(true)
                                || self.crossing(face, local) != Some(Crossing::Out)
                            {
                                node.ways.push(Self::face_node(face));
                            }
                        }
                    }
                }
            }
            nodes.push(node.settled());
        }
        for (point, place) in self.points.iter().enumerate() {
            let mut node = Node::captured(hulls, &[[place[0], place[1]]]);
            if !node.must_capture() {
                for &(face, corner) in &self.vertex_faces[point] {
                    let plane = self.planes[face];
                    let into = match (plane.steep, plane.direction) {
                        (Some(true), Some(direction)) => {
                            let plan = self.plan(face);
                            let apex = plan[corner];
                            let unit = |toward: P2| {
                                let along = [toward[0] - apex[0], toward[1] - apex[1]];
                                let length = along[0].hypot(along[1]);
                                [along[0] / length, along[1] / length]
                            };
                            cross2(unit(plan[(corner + 1) % 3]), direction) >= -TURN
                                && cross2(direction, unit(plan[(corner + 2) % 3])) >= -TURN
                        }
                        _ => true,
                    };
                    if into {
                        node.ways.push(Self::face_node(face));
                    }
                }
                for &edge in &self.vertex_edges[point] {
                    let ends = self.edges[edge].ends;
                    let other = if ends[0] == point { ends[1] } else { ends[0] };
                    if self.points[other][2] < place[2] + LINEAR {
                        node.ways.push(self.edge_node(edge));
                    }
                }
                if node.ways.is_empty() {
                    node.rest = node.unless_caught();
                }
            }
            nodes.push(node.settled());
        }
        nodes
    }

    fn analyse(mut self, hulls: &[Hull], minimum_fall: f64) -> Analysis {
        self.planes = (0..self.faces.len())
            .map(|face| self.plane(face, minimum_fall))
            .collect();
        let nodes = self.nodes(hulls);
        let (drains, stays) = fates(&nodes);
        let area = self.area();
        let (mut open, mut kept) = (0.0, 0.0);
        for face in 0..self.faces.len() {
            let plane_area = self.planes[face].area;
            if !drains[face] {
                open += plane_area;
            }
            if stays[face] {
                kept += plane_area;
            } else {
                kept += self.held(face, &nodes, &stays, hulls);
            }
        }
        let upper = widen(open).1.min(area.1);
        let lower = widen(kept).0.min(upper);
        let path = Paths {
            tin: &self,
            nodes: &nodes,
            hulls,
            stays: &stays,
        }
        .longest(&drains);
        let decided = (0..self.faces.len()).all(|face| drains[face] || stays[face]);
        Analysis {
            area,
            undrained: (lower, upper),
            path,
            decided,
        }
    }
}

impl Tin {
    /// The plan area of `face` that surely holds its water, where the face
    /// would hold all of it but for the outlets meeting it: the face less
    /// where an outlet may take water, directly (a face too flat to shed
    /// it) or from upstream (the outlet swept back against the flow).
    fn held(&self, face: usize, nodes: &[Node], stays: &[bool], hulls: &[Hull]) -> f64 {
        let node = &nodes[face];
        let plane = self.planes[face];
        if stays[face] || node.must_capture() || !node.can_capture() {
            return 0.0;
        }
        let flat = plane.steep == Some(false);
        let onward = !node.ways.is_empty() && node.ways.iter().all(|way| stays[*way]);
        if !(flat || onward && plane.direction.is_some()) {
            return 0.0;
        }
        let plan = self.plan(face);
        let reach = 2.0 * diameter(self.corners(face)) + 1.0;
        let taken: f64 = hulls
            .iter()
            .map(|hull| match (flat, plane.direction) {
                (false, Some(direction)) => {
                    let back = [-direction[0] * reach, -direction[1] * reach];
                    let swept: Vec<P2> = hull
                        .points
                        .iter()
                        .flat_map(|point| [*point, [point[0] + back[0], point[1] + back[1]]])
                        .collect();
                    Hull::of(&swept, hull.margin).map_or(plane.area, |swept| swept.cut(plan))
                }
                _ => hull.cut(plan),
            })
            .sum();
        (plane.area - taken).max(0.0)
    }
}

/// `value` rounded outward.
fn widen(value: f64) -> (f64, f64) {
    if value == 0.0 {
        return (0.0, 0.0);
    }
    (
        (value * (1.0 - ROUNDING)).next_down().max(0.0),
        (value * (1.0 + ROUNDING)).next_up(),
    )
}

/// A place water may be, and where it may go on.
#[derive(Clone, Debug, Default)]
struct Node {
    ways: Vec<usize>,
    capture: Chance,
    rest: Chance,
}

/// Whether something befalls water at a node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Chance {
    #[default]
    Never,
    May,
    Must,
}

impl Node {
    /// A node over `points` in plan (a point, a segment or a triangle),
    /// captured where an outlet surely holds it all and possibly where one
    /// may meet it.
    fn captured(hulls: &[Hull], points: &[P2]) -> Self {
        Self {
            capture: if hulls.iter().any(|hull| hull.holds(points)) {
                Chance::Must
            } else if hulls.iter().any(|hull| hull.may_meet(points)) {
                Chance::May
            } else {
                Chance::Never
            },
            ..Self::default()
        }
    }

    /// Resting surely, or where an outlet may take the water, possibly.
    fn unless_caught(&self) -> Chance {
        if self.can_capture() {
            Chance::May
        } else {
            Chance::Must
        }
    }

    fn must_capture(&self) -> bool {
        self.capture == Chance::Must
    }

    fn can_capture(&self) -> bool {
        self.capture != Chance::Never
    }

    fn can_rest(&self) -> bool {
        self.rest != Chance::Never
    }

    fn must_rest(&self) -> bool {
        self.rest == Chance::Must
    }

    fn settled(mut self) -> Self {
        self.ways.sort_unstable();
        self.ways.dedup();
        self
    }
}

/// Which nodes surely drain, and which surely do not: the least
/// fixpoints, each node decided once every way on from it is.
fn fates(nodes: &[Node]) -> (Vec<bool>, Vec<bool>) {
    let mut from: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (at, node) in nodes.iter().enumerate() {
        for &way in &node.ways {
            from[way].push(at);
        }
    }
    let settle = |base: &dyn Fn(&Node) -> Option<bool>| {
        let mut decided = vec![false; nodes.len()];
        let mut open: Vec<usize> = nodes.iter().map(|node| node.ways.len()).collect();
        let mut queue = Vec::new();
        for (at, node) in nodes.iter().enumerate() {
            if base(node) == Some(true) {
                decided[at] = true;
                queue.push(at);
            }
        }
        while let Some(done) = queue.pop() {
            for &before in &from[done] {
                if decided[before] {
                    continue;
                }
                open[before] -= 1;
                if open[before] == 0 && base(&nodes[before]).is_none() {
                    decided[before] = true;
                    queue.push(before);
                }
            }
        }
        decided
    };
    // Surely drains: captured, or never resting with every way on draining.
    let drains = settle(&|node| {
        if node.must_capture() {
            Some(true)
        } else if node.can_rest() || node.ways.is_empty() {
            Some(false)
        } else {
            None
        }
    });
    // Surely does not: never captured, and resting surely or every way on
    // staying.
    let stays = settle(&|node| {
        if node.can_capture() {
            Some(false)
        } else if node.must_rest() {
            Some(true)
        } else if node.ways.is_empty() {
            Some(false)
        } else {
            None
        }
    });
    (drains, stays)
}

/// The longest path, bounded over the network.
struct Paths<'a> {
    tin: &'a Tin,
    nodes: &'a [Node],
    hulls: &'a [Hull],
    stays: &'a [bool],
}

impl Paths<'_> {
    fn longest(&self, drains: &[bool]) -> DrainagePath {
        let faces = self.tin.faces.len();
        if (0..faces).all(|face| self.stays[face]) {
            return DrainagePath::NoOutlet;
        }
        let Some(upper) = self.upper_runs() else {
            return DrainagePath::Unbounded;
        };
        let mut most: Option<f64> = None;
        for (face, run) in upper.iter().enumerate().take(faces) {
            if !self.stays[face] {
                most = larger(most, *run);
            }
        }
        let Some(most) = most else {
            return DrainagePath::NoOutlet;
        };
        let mut least: f64 = 0.0;
        for face in (0..faces).filter(|face| drains[*face]) {
            least = least.max(self.lower_run(face));
        }
        let upper = widen(most).1 + LINEAR;
        let lower = widen(least).0.min(upper);
        DrainagePath::Bounded {
            lower_metres: lower,
            upper_metres: upper,
        }
    }

    /// The plan distance from `point` to the nearest outlet.
    fn to_outlet(&self, point: P2) -> f64 {
        self.hulls
            .iter()
            .map(|hull| hull.distance(point))
            .fold(f64::INFINITY, f64::min)
    }

    /// The longest run from every node water from a face may reach, each
    /// node's after every way on from it; a cycle among them leaves the
    /// runs unbounded.
    fn upper_runs(&self) -> Option<Vec<Option<f64>>> {
        let count = self.nodes.len();
        let mut runs: Vec<Option<f64>> = vec![None; count];
        let mut done = vec![false; count];
        let mut on_stack = vec![false; count];
        for start in 0..self.tin.faces.len() {
            let mut stack = vec![(start, false)];
            while let Some((node, expanded)) = stack.pop() {
                if done[node] {
                    continue;
                }
                if expanded {
                    on_stack[node] = false;
                    runs[node] = self.upper_run(node, &runs);
                    done[node] = true;
                    continue;
                }
                if self.stays[node] {
                    done[node] = true;
                    continue;
                }
                on_stack[node] = true;
                stack.push((node, true));
                for &way in &self.nodes[node].ways {
                    if on_stack[way] {
                        return None;
                    }
                    if !done[way] {
                        stack.push((way, false));
                    }
                }
            }
        }
        Some(runs)
    }

    /// The longest run from `node`, every way on from it known.
    fn upper_run(&self, node: usize, runs: &[Option<f64>]) -> Option<f64> {
        let tin = self.tin;
        let info = &self.nodes[node];
        if info.must_capture() {
            return Some(0.0);
        }
        let after = |way: usize| runs[way];
        let base = info.can_capture().then_some(0.0);
        let faces = tin.faces.len();
        let longest_way = info.ways.iter().map(|way| after(*way)).fold(None, larger);
        let run = if node < faces {
            self.face_run(node, runs).or_else(|| {
                // No decided direction: across the face, then on.
                longest_way.map(|rest| diameter(tin.corners(node)) + rest)
            })
        } else if node < faces + tin.edges.len() {
            let [from, to] = tin.edges[node - faces].ends.map(|end| tin.points[end]);
            longest_way.map(|rest| norm(minus(to, from)) + rest)
        } else {
            longest_way
        };
        larger(base, run)
    }

    /// The longest run from a face with a decided direction: the greatest
    /// over its corners and back-traced points of the run to its exit plus
    /// the rest from there.
    fn face_run(&self, face: usize, runs: &[Option<f64>]) -> Option<f64> {
        let tin = self.tin;
        let plane = tin.planes[face];
        let direction = plane.direction?;
        if plane.steep != Some(true) {
            return None;
        }
        let stretch = plane.fall.hypot(1.0);
        let mut best: Option<f64> = None;
        for start in starts(tin.plan(face), direction) {
            for (run, end, local) in exits(tin.plan(face), direction, start) {
                let mut rest: Option<f64> = None;
                for way in tin.across(face, local) {
                    rest = larger(rest, self.rest_upper(way, end, runs));
                }
                if let Some(rest) = rest {
                    best = larger(best, Some(run * stretch + rest));
                }
            }
        }
        best
    }

    /// The longest run onward from `end` once water enters `way` there:
    /// down a valley to its lower end and on, or the longest from `way`.
    /// Water caught or resting on the way only runs less.
    fn rest_upper(&self, way: usize, end: P2, runs: &[Option<f64>]) -> Option<f64> {
        if let Some(valley) = self.valley(way, end) {
            return runs[valley.low].map(|rest| valley.along + rest);
        }
        runs[way]
    }

    /// Where water entering the edge node `way` at `end` runs, when its only
    /// way on is the vertex at its lower end.
    fn valley(&self, way: usize, end: P2) -> Option<Valley> {
        let tin = self.tin;
        let faces = tin.faces.len();
        if way < faces || way >= faces + tin.edges.len() {
            return None;
        }
        let node = &self.nodes[way];
        let [low] = node.ways[..] else {
            return None;
        };
        let edge = &tin.edges[way - faces];
        let at = edge
            .ends
            .iter()
            .position(|point| tin.vertex_node(*point) == low)?;
        let [a, b] = edge.ends.map(|point| tin.points[point]);
        let (from, to) = if at == 0 { (b, a) } else { (a, b) };
        let plan_length = (to[0] - from[0]).hypot(to[1] - from[1]);
        let share = if plan_length > 0.0 {
            ((end[0] - to[0]).hypot(end[1] - to[1]) / plan_length).min(1.0)
        } else {
            0.0
        };
        let length = norm(minus(to, from));
        // The first point from `end` towards `to` an outlet may hold.
        let caught = self
            .hulls
            .iter()
            .filter_map(|hull| hull.entry(end, [to[0], to[1]]))
            .reduce(f64::min)
            .map(|entry| entry * length * share);
        Some(Valley {
            low,
            along: length * share,
            caught,
            rests: node.can_rest(),
            to: [to[0], to[1]],
        })
    }

    /// A lower bound on the longest run from a face surely draining.
    fn lower_run(&self, face: usize) -> f64 {
        let tin = self.tin;
        let plane = tin.planes[face];
        let plan = tin.plan(face);
        let mut least = plan
            .iter()
            .map(|corner| self.to_outlet(*corner))
            .fold(0.0, f64::max);
        let Some(direction) = plane.direction else {
            return least;
        };
        if plane.steep != Some(true) {
            return least;
        }
        let stretch = plane.fall.hypot(1.0);
        for start in starts(plan, direction) {
            let mut shortest = f64::INFINITY;
            for (run, end, local) in exits(plan, direction, start) {
                let ways = tin.across(face, local);
                let rest = match ways[..] {
                    [way] => match self.valley(way, end) {
                        Some(Valley {
                            caught: Some(caught),
                            ..
                        }) => caught,
                        Some(Valley {
                            along,
                            caught: None,
                            rests: false,
                            to,
                            ..
                        }) => along + self.to_outlet(to),
                        _ => self.to_outlet(end),
                    },
                    _ => self.to_outlet(end),
                };
                // An outlet over the face may take the water before it
                // leaves.
                let caught = self
                    .hulls
                    .iter()
                    .filter_map(|hull| hull.entry(start, end))
                    .reduce(f64::min);
                shortest = shortest.min(match caught {
                    Some(share) => run * share * stretch,
                    None => run * stretch + rest,
                });
            }
            if shortest.is_finite() {
                least = least.max(shortest);
            }
            least = least.max(self.to_outlet(start));
        }
        least
    }
}

/// Water entering a valley: the vertex node at its lower end, the length
/// along the valley to it, where an outlet may first take it on the way,
/// whether it may rest on the valley, and the lower end in plan.
struct Valley {
    low: usize,
    along: f64,
    caught: Option<f64>,
    rests: bool,
    to: P2,
}

/// The points of a face where the run to its exit and on is greatest: its
/// corners, and where the flow line through a corner leaves it backwards.
fn starts(plan: [P2; 3], direction: P2) -> Vec<P2> {
    let mut starts = plan.to_vec();
    let back = [-direction[0], -direction[1]];
    for corner in plan {
        if let Some((run, end, _)) = exits(plan, back, corner)
            .into_iter()
            .find(|(run, ..)| *run > LINEAR)
        {
            let _ = run;
            starts.push(end);
        }
    }
    starts
}

/// Where a run from `start` along `direction` leaves the face `plan`
/// (anticlockwise): its length, end and the edge it crosses, each edge it
/// may cross first within rounding.
fn exits(plan: [P2; 3], direction: P2, start: P2) -> Vec<(f64, P2, usize)> {
    let mut found: Vec<(f64, P2, usize)> = Vec::new();
    for local in 0..3 {
        let (a, b) = (plan[local], plan[(local + 1) % 3]);
        let along = [b[0] - a[0], b[1] - a[1]];
        let length = along[0].hypot(along[1]);
        if length == 0.0 {
            continue;
        }
        let normal = [along[1] / length, -along[0] / length];
        let share = dot2(direction, normal);
        if share <= ALONG {
            continue;
        }
        let gap = dot2(normal, [a[0] - start[0], a[1] - start[1]]).max(0.0);
        let run = gap / share;
        let end = [start[0] + direction[0] * run, start[1] + direction[1] * run];
        found.push((run, end, local));
    }
    let Some(first) = found.iter().map(|(run, ..)| *run).reduce(f64::min) else {
        return found;
    };
    found.retain(|(run, ..)| *run <= first + LINEAR);
    found
}

/// The longest side of a face, in space.
fn diameter([a, b, c]: [P3; 3]) -> f64 {
    norm(minus(b, a))
        .max(norm(minus(c, b)))
        .max(norm(minus(a, c)))
}

/// Whether two segments lie on one line and overlap by more than
/// [`LINEAR`].
fn overlap([start, end]: [P3; 2], other: [P3; 2]) -> bool {
    let along = minus(end, start);
    let length = norm(along);
    if length <= LINEAR {
        return false;
    }
    let unit = along.map(|value| value / length);
    if other
        .iter()
        .any(|point| norm(cross(minus(*point, start), unit)) > LINEAR)
    {
        return false;
    }
    let [from, to] = other.map(|point| dot(minus(point, start), unit));
    from.max(to).min(length) - from.min(to).max(0.0) > LINEAR
}

fn plan_area([a, b, c]: [P2; 3]) -> f64 {
    cross2([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]) / 2.0
}

fn minus(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: P3) -> f64 {
    dot(a, a).sqrt()
}

fn dot2(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross2(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// The larger of two runs, none counting less than any.
fn larger(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Hull {
        Hull::of(
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.5, 0.5]],
            0.0,
        )
        .unwrap()
    }

    #[test]
    fn a_footprint_holds_meets_and_cuts_what_lies_over_it() {
        let hull = square();
        assert_eq!(hull.points.len(), 4);
        assert!((hull.area() - 1.0).abs() < 1e-15);
        assert!(hull.holds(&[[0.5, 0.5], [0.2, 0.9]]));
        assert!(!hull.holds(&[[0.5, 0.5], [1.0, 0.5]]));
        assert!(hull.may_meet(&[[1.0, 0.5], [2.0, 0.5], [2.0, 1.0]]));
        assert!(!hull.may_meet(&[[1.1, 0.5], [2.0, 0.5], [2.0, 1.0]]));
        // Half a triangle over it, half beside it.
        let cut = hull.cut([[0.5, 0.0], [1.5, 0.0], [1.5, 1.0]]);
        assert!((cut - 0.125).abs() < 1e-8, "{cut}");
        assert!((hull.distance([2.0, 0.5]) - 1.0).abs() < 1e-8);
        assert!(hull.distance([0.5, 0.5]).abs() < f64::EPSILON);
        // A segment from (-1, 0.5) to (3, 0.5) enters a quarter of the way.
        let entry = hull.entry([-1.0, 0.5], [3.0, 0.5]).unwrap();
        assert!((entry - 0.25).abs() < 1e-8, "{entry}");
        assert_eq!(hull.entry([-1.0, 2.0], [3.0, 2.0]), None);
        // No area, no footprint.
        assert!(Hull::of(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], 0.0).is_none());
    }

    #[test]
    fn a_run_leaves_a_face_over_the_edge_it_meets_first() {
        let plan = [[0.0, 0.0], [10.0, 0.0], [5.0, 5.0]];
        // From the base's middle straight up to the apex: both slanted
        // edges at once.
        let found = exits(plan, [0.0, 1.0], [5.0, 0.0]);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|(run, ..)| (run - 5.0).abs() < 1e-12));
        // From a corner on an edge water leaves over, at once.
        let found = exits(plan, [0.0, 1.0], [0.0, 0.0]);
        assert_eq!(found.len(), 1);
        assert!(found[0].0.abs() < f64::EPSILON);
        assert_eq!(found[0].2, 2);
        // The corners, and where the flow through the apex comes in.
        let starts = starts(plan, [0.0, 1.0]);
        assert!(
            starts
                .iter()
                .any(|point| (point[0] - 5.0).abs() < 1e-12 && point[1].abs() < 1e-12)
        );
    }
}
