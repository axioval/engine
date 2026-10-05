//! Sections across IFC alignments and clearance envelopes swept along
//! them: the section and envelope methods of the bridge's
//! [`AlignmentService`](axioval::engine::AlignmentService).
//!
//! # The section frame
//!
//! The section at plan distance `d` is the vertical plane through the
//! centreline point `C(d)` normal to the plan tangent there: `lateral` is
//! the horizontal left normal `L(d)`, `up` is `+Z`. A section coordinate
//! `(u, v)` is the point `P(d; u, v) = C(d) + u L(d) + v Z`. A station is
//! carried to its plan distance through the alignment's station equations
//! (`Stationing::distance_at`); a station no range or several ranges carry
//! is refused, one before the start or beyond the end is off the range.
//! The alignment's centreline is read as positions are (an alignment
//! placed off the identity is refused, openbimrs/ifc#357).
//!
//! # A section as an interval region
//!
//! A body's mesh is cut by the computed plane: every triangle crossing it
//! gives one oriented segment, its interior to the left, from a vertex
//! classification shared by every triangle (a vertex on the plane counts
//! as in front) and a cut point computed once per edge from its endpoints
//! in a canonical order, so the segments close into cycles exactly. The
//! band is every triangle clipped to the slab within `radius` of the plane
//! and projected onto it; `radius` is the mesh's certified deviation plus
//! the frame's numerical error ([`frame_error`]: the evaluator's stated
//! accuracy and rounding, scaled by the body's reach from the centreline
//! and the curve's derivative bounds). A point at distance `radius` or
//! more from every mesh triangle cannot change sides between the mesh and
//! the body, and between the computed and the exact plane, so the region
//! of [`axioval::engine::BodySection`] holds the exact section. Only a
//! closed mesh (every directed edge matched by its reverse) is cut; a body
//! measured through its parts is refused, since overlapping parts would
//! count their overlap twice.
//!
//! # A clearance envelope swept along a range
//!
//! For a step `[a, b]` with midpoint `m` and half-width `h`, every point
//! `P(s; u, v)` with `s` in the step lies within
//!
//! ```text
//! D = h (B1 + |u|max B2 / speed)
//! ```
//!
//! of `P(m; u, v)`, since `|dP/ds| <= |C'| + |u| |L'|` and `|L'|` is at most
//! the plan curvature `|c''| / |c'|`; `B1` and `B2` are the certified
//! derivative bounds of the centreline over the step (split at its
//! continuity breaks) and `speed` the least plan speed there. A body can
//! reach into the envelope swept over the step only if some point of its
//! mesh lies within `r = D + deviation + frame error` of the envelope
//! placed at `m`. Each triangle is clipped to the slab `|t| <= r` around the
//! mid plane and projected onto it, and tested against the envelope grown
//! by `r`: a test in 3D of the triangles between the samples, never of
//! cuts at the samples alone. When no triangle comes that near, the
//! envelope placed at `m` lies wholly inside or wholly outside the body,
//! and its first vertex's winding number in the mesh says which: inside is
//! a sure intrusion, outside proves the body clear of the step.
//!
//! A body near the envelope in a step is decided by sections: a witness
//! point surely inside both the body's section region and the envelope at
//! a station of the range is a sure intrusion. Otherwise the step is
//! halved, at most [`DEPTH`] times and within [`SWEEP_BUDGET`] intervals per
//! body; a step still undecided leaves the body possible, with the
//! stretch and the reason, never clear. So a body crossing the envelope
//! between two samples, however thin, is found or left possible.

use std::collections::BTreeMap;

use axiolid_evaluate::bound::curve_derivative_bounds3;
use axiolid_evaluate::{elevated_derivative, elevated_point};
use axioval::axiolid::AxiolidGeometry;
use axioval::engine::{
    AlignmentError, AlignmentInterval, BodySection, EnvelopeRequest, EnvelopeSweep, Intrusion,
    Section, SectionPolygon, SectionRequest,
};
use axioval::ir::ObjectId;

use super::{Centreline, IfcAlignmentService, RESOLUTION, evidence, slack};

/// How many times a step may be halved before a body near the envelope in
/// it is left possible.
pub(super) const DEPTH: u32 = 10;

/// Intervals one body's sweep may test.
pub(super) const SWEEP_BUDGET: usize = 20_000;

/// Sampled steps a range may hold.
const MAX_STEPS: f64 = 100_000.0;

/// A section frame: origin on the centreline, horizontal unit tangent and
/// left normal; up is `+Z`.
struct Frame {
    origin: [f64; 3],
    tangent: [f64; 3],
    lateral: [f64; 3],
    /// The plan speed `|c'|` there.
    speed: f64,
}

impl Frame {
    /// `(t, u, v)`: along the tangent, lateral and up, from the origin.
    fn coordinates(&self, point: [f64; 3]) -> [f64; 3] {
        let relative: [f64; 3] = std::array::from_fn(|i| point[i] - self.origin[i]);
        let dot = |axis: [f64; 3]| (0..3).map(|i| axis[i] * relative[i]).sum::<f64>();
        [dot(self.tangent), dot(self.lateral), relative[2]]
    }

    /// The point at section coordinates `(u, v)`.
    fn place(&self, [u, v]: [f64; 2]) -> [f64; 3] {
        [
            self.origin[0] + u * self.lateral[0],
            self.origin[1] + u * self.lateral[1],
            self.origin[2] + v,
        ]
    }
}

impl Centreline {
    /// The section frame at plan distance `at`.
    fn frame_at(&self, at: f64) -> Result<Frame, String> {
        let unevaluated = |error| format!("the centreline cannot be evaluated at {at} m ({error})");
        let point = elevated_point(self.elevated(), at).map_err(unevaluated)?;
        let derivative = elevated_derivative(self.elevated(), at).map_err(unevaluated)?;
        let speed = derivative.x.hypot(derivative.y);
        if !(speed.is_finite() && speed > 0.5) {
            return Err(format!(
                "the centreline is not measured by its plan length at {at} m"
            ));
        }
        let (tx, ty) = (derivative.x / speed, derivative.y / speed);
        Ok(Frame {
            origin: [point.x, point.y, point.z],
            tangent: [tx, ty, 0.0],
            lateral: [-ty, tx, 0.0],
            speed,
        })
    }

    /// Bounds `(B1, B2)` on the centreline's first two derivatives over
    /// `[a, b]`, split at its continuity breaks.
    fn derivative_bounds(&self, a: f64, b: f64) -> Result<(f64, f64), String> {
        let mut cuts = vec![a];
        cuts.extend(self.breaks.iter().copied().filter(|at| *at > a && *at < b));
        cuts.push(b);
        let (mut first, mut second) = (0.0_f64, 0.0_f64);
        for pair in cuts.windows(2) {
            let bounds =
                curve_derivative_bounds3(&self.curve, pair[0], pair[1]).ok_or_else(|| {
                    format!(
                        "the centreline has no certified derivative bound over [{}, {}] m",
                        pair[0], pair[1]
                    )
                })?;
            first = first.max(bounds.first);
            second = second.max(bounds.second);
        }
        Ok((first, second))
    }

    /// The plan distance a station labels.
    fn distance_of(&self, station: f64) -> Result<f64, AlignmentError> {
        let stationing = self
            .stationing
            .as_ref()
            .map_err(|reason| AlignmentError::Unavailable(reason.clone()))?;
        let distance = match stationing {
            None => station,
            Some(stationing) => match stationing.distance_at(station) {
                Ok(distance) => distance,
                Err(ifc_alignment::AlignmentError::AmbiguousStation { distances, .. }) => {
                    return Err(AlignmentError::Ambiguous(format!(
                        "station {station} labels {} distances along {}",
                        distances.len(),
                        self.locator
                    )));
                }
                Err(error) => {
                    return Err(AlignmentError::OffRange(format!(
                        "no stretch of {} carries station {station} ({error})",
                        self.locator
                    )));
                }
            },
        };
        if !distance.is_finite() || distance < -RESOLUTION || distance > self.length + RESOLUTION {
            return Err(AlignmentError::OffRange(format!(
                "station {station} lies {distance} m along {}, outside [0, {}] m",
                self.locator, self.length
            )));
        }
        Ok(distance.clamp(0.0, self.length))
    }

    /// The numerical error of a section coordinate at a point `reach` from
    /// the centreline: the evaluator's accuracy and rounding on the origin
    /// and the axes, scaled by the curve's derivative bounds.
    fn frame_error(&self, reach: f64, (first, second): (f64, f64)) -> f64 {
        slack(self.length.max(reach)) * (4.0 + 2.0 * reach) * (1.0 + first + second)
    }
}

/// A body's mesh in the centreline's frame.
struct Solid {
    triangles: Vec<[[f64; 3]; 3]>,
    deviation: f64,
    low: [f64; 3],
    high: [f64; 3],
}

impl Solid {
    /// The corners of the body's box, for a frame's coordinates.
    fn corners(&self) -> impl Iterator<Item = [f64; 3]> + '_ {
        (0..8).map(|corner| {
            std::array::from_fn(|axis| {
                if corner & (1 << axis) == 0 {
                    self.low[axis]
                } else {
                    self.high[axis]
                }
            })
        })
    }

    /// The box of the body in a frame's coordinates and its reach from the
    /// frame's origin.
    fn framed_box(&self, frame: &Frame) -> ([f64; 3], [f64; 3], f64) {
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        let mut reach: f64 = 0.0;
        for corner in self.corners() {
            let framed = frame.coordinates(corner);
            for axis in 0..3 {
                low[axis] = low[axis].min(framed[axis]);
                high[axis] = high[axis].max(framed[axis]);
            }
            reach = reach.max(framed.iter().map(|value| value * value).sum::<f64>().sqrt());
        }
        (low, high, reach)
    }

    /// The body's winding number around `point`, rounded: each triangle's
    /// solid angle (Van Oosterom and Strackee) summed over the full sphere.
    fn winding(&self, point: [f64; 3]) -> i64 {
        let mut total = 0.0;
        for triangle in &self.triangles {
            let [a, b, c] = triangle
                .map(|vertex| -> [f64; 3] { std::array::from_fn(|i| vertex[i] - point[i]) });
            let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let dot = |p: [f64; 3], q: [f64; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
            let cross = [
                b[1] * c[2] - b[2] * c[1],
                b[2] * c[0] - b[0] * c[2],
                b[0] * c[1] - b[1] * c[0],
            ];
            let (la, lb, lc) = (length(a), length(b), length(c));
            let numerator = dot(a, cross);
            let denominator = la * lb * lc + dot(a, b) * lc + dot(a, c) * lb + dot(b, c) * la;
            total += 2.0 * numerator.atan2(denominator);
        }
        #[allow(clippy::cast_possible_truncation)]
        let winding = (total / (4.0 * std::f64::consts::PI)).round() as i64;
        winding
    }
}

/// `object`'s body as a closed mesh in `centreline`'s frame; `None` when it
/// has no body.
fn solid(
    geometry: &AxiolidGeometry,
    centreline: &Centreline,
    object: &ObjectId,
) -> Result<Option<Solid>, String> {
    if geometry.has_no_body(object) {
        return Ok(None);
    }
    if geometry.is_unmeasured(object) {
        let reason = geometry
            .unmeasured()
            .find(|(id, _)| *id == object)
            .map_or("unmeasured", |(_, reason)| reason);
        return Err(format!("{object} is unmeasured: {reason}"));
    }
    let mesh = geometry
        .mesh(object)
        .ok_or_else(|| format!("{object} has no mesh registered"))?;
    let deviation = geometry
        .fidelity(object)
        .map_err(|error| format!("{object} has no usable deviation ({error})"))?
        .deviation_metres();
    let key = |index: u32| -> Option<[u64; 3]> {
        let point = mesh.positions.get(index as usize)?;
        Some([point.x.to_bits(), point.y.to_bits(), point.z.to_bits()])
    };
    let mut edges: BTreeMap<([u64; 3], [u64; 3]), i64> = BTreeMap::new();
    let mut triangles = Vec::with_capacity(mesh.triangle_count());
    for [a, b, c] in mesh.triangles() {
        let keys = [key(a), key(b), key(c)];
        let [Some(ka), Some(kb), Some(kc)] = keys else {
            return Err(format!(
                "{object}'s mesh addresses a vertex it does not hold"
            ));
        };
        for (from, to) in [(ka, kb), (kb, kc), (kc, ka)] {
            if from == to {
                continue;
            }
            // A directed edge and its reverse cancel.
            let (pair, sign) = if from < to {
                ((from, to), 1)
            } else {
                ((to, from), -1)
            };
            *edges.entry(pair).or_default() += sign;
        }
        let vertex = |index: u32| {
            let point = mesh.positions[index as usize];
            centreline.to_local([point.x, point.y, point.z])
        };
        triangles.push([vertex(a), vertex(b), vertex(c)]);
    }
    if edges.values().any(|count| *count != 0) {
        return Err(format!(
            "{object}'s mesh is no closed solid (an edge without its reverse), so it cannot be \
             cut"
        ));
    }
    if triangles.is_empty() {
        return Err(format!("{object}'s mesh holds no triangle"));
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for vertex in triangles.iter().flatten() {
        for axis in 0..3 {
            low[axis] = low[axis].min(vertex[axis]);
            high[axis] = high[axis].max(vertex[axis]);
        }
    }
    if !low.iter().chain(&high).all(|value| value.is_finite()) {
        return Err(format!(
            "{object}'s mesh has a coordinate that is not finite"
        ));
    }
    Ok(Some(Solid {
        triangles,
        deviation,
        low,
        high,
    }))
}

/// The point where the edge `[a, b]` (in frame coordinates, `t` of
/// opposite classification) meets the plane, computed from its endpoints
/// in a canonical order so both triangles sharing it agree to the bit.
fn edge_cut(a: [f64; 3], b: [f64; 3]) -> [f64; 2] {
    let ordered = |p: &[f64; 3], q: &[f64; 3]| {
        p.iter()
            .zip(q)
            .map(|(x, y)| x.total_cmp(y))
            .find(|order| order.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    let (a, b) = if ordered(&a, &b).is_le() {
        (a, b)
    } else {
        (b, a)
    };
    let ratio = a[0] / (a[0] - b[0]);
    [a[1] + ratio * (b[1] - a[1]), a[2] + ratio * (b[2] - a[2])]
}

/// The triangle (frame coordinates) clipped to `|t| <= half` and projected
/// onto the plane; empty when it misses the slab.
fn clipped(triangle: &[[f64; 3]; 3], half: f64) -> Vec<[f64; 2]> {
    let mut polygon: Vec<[f64; 3]> = triangle.to_vec();
    for (sign, limit) in [(1.0, half), (-1.0, half)] {
        let inside = |p: &[f64; 3]| sign * p[0] <= limit;
        let mut out = Vec::with_capacity(polygon.len() + 2);
        for i in 0..polygon.len() {
            let (current, next) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            if inside(&current) {
                out.push(current);
            }
            if inside(&current) != inside(&next) {
                let ratio = (sign * limit - current[0]) / (next[0] - current[0]);
                out.push(std::array::from_fn(|axis| {
                    current[axis] + ratio * (next[axis] - current[axis])
                }));
            }
        }
        polygon = out;
        if polygon.is_empty() {
            break;
        }
    }
    polygon.iter().map(|p| [p[1], p[2]]).collect()
}

/// A section's oriented cut segments and its band's convex polygons.
type CutAndBand = (Vec<[[f64; 2]; 2]>, Vec<Vec<[f64; 2]>>);

/// The cut and band of `solid` in `frame`, the band within `radius`.
fn cut_and_band(solid: &Solid, frame: &Frame, radius: f64) -> CutAndBand {
    let mut cut = Vec::new();
    let mut band = Vec::new();
    for triangle in &solid.triangles {
        let framed = triangle.map(|vertex| frame.coordinates(vertex));
        let low = framed.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
        let high = framed
            .iter()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        if low > radius || high < -radius {
            continue;
        }
        let front = framed.map(|p| p[0] >= 0.0);
        if front.iter().any(|f| *f) && front.iter().any(|f| !*f) {
            // The vertex on its own side, then the next two in order.
            let odd = (0..3)
                .find(|&i| front[i] != front[(i + 1) % 3] && front[i] != front[(i + 2) % 3])
                .unwrap_or(0);
            let (v0, v1, v2) = (framed[odd], framed[(odd + 1) % 3], framed[(odd + 2) % 3]);
            let (p01, p20) = (edge_cut(v0, v1), edge_cut(v2, v0));
            cut.push(if front[odd] { [p01, p20] } else { [p20, p01] });
        }
        let polygon = clipped(&framed, radius);
        if !polygon.is_empty() {
            band.push(polygon);
        }
    }
    (cut, band)
}

/// Distance from `point` to the closed segment `[a, b]`.
fn segment_distance(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let along = if length > 0.0 {
        (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point[0] - a[0] - along * dx).hypot(point[1] - a[1] - along * dy)
}

/// Whether `point` lies inside the polygon `vertices` (winding number).
fn inside(vertices: &[[f64; 2]], point: [f64; 2]) -> bool {
    let count = vertices.len();
    if count < 3 {
        return false;
    }
    let mut winding = 0;
    for i in 0..count {
        let (a, b) = (vertices[i], vertices[(i + 1) % count]);
        let side = (b[0] - a[0]) * (point[1] - a[1]) - (point[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= point[1] && b[1] > point[1] && side > 0.0 {
            winding += 1;
        } else if b[1] <= point[1] && a[1] > point[1] && side < 0.0 {
            winding -= 1;
        }
    }
    winding != 0
}

/// Whether the polygons `p` (convex, possibly degenerate) and `q` (simple)
/// come within `reach` of each other.
fn within(p: &[[f64; 2]], q: &[[f64; 2]], reach: f64) -> bool {
    if p.iter().any(|point| inside(q, *point)) || q.iter().any(|point| inside(p, *point)) {
        return true;
    }
    let edges = |polygon: &[[f64; 2]]| -> Vec<([f64; 2], [f64; 2])> {
        let count = polygon.len();
        match count {
            0 => Vec::new(),
            1 => vec![(polygon[0], polygon[0])],
            _ => (0..count)
                .map(|i| (polygon[i], polygon[(i + 1) % count]))
                .collect(),
        }
    };
    let (pe, qe) = (edges(p), edges(q));
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    for (a, b) in &pe {
        for (c, d) in &qe {
            let crossing = cross(*a, *b, *c) * cross(*a, *b, *d) < 0.0
                && cross(*c, *d, *a) * cross(*c, *d, *b) < 0.0;
            if crossing
                || segment_distance(*a, *c, *d) <= reach
                || segment_distance(*b, *c, *d) <= reach
                || segment_distance(*c, *a, *b) <= reach
                || segment_distance(*d, *a, *b) <= reach
            {
                return true;
            }
        }
    }
    false
}

/// What one step tells about a body.
enum Near {
    /// The body is proven out of the envelope swept over the step.
    Clear,
    /// A point of the body is proven inside the envelope at this distance.
    Inside(f64),
    /// The body may reach into it; the reason says why.
    Within(String),
}

/// A sweep of one envelope along one centreline.
struct Sweep<'c> {
    centreline: &'c Centreline,
    envelope: &'c SectionPolygon,
    /// The envelope's box in section coordinates.
    low: [f64; 2],
    high: [f64; 2],
}

impl Sweep<'_> {
    /// The envelope's reach from the centreline.
    fn reach(&self) -> f64 {
        let lateral = self.low[0].abs().max(self.high[0].abs());
        let up = self.low[1].abs().max(self.high[1].abs());
        lateral.hypot(up)
    }

    /// Whether `solid` can reach into the envelope swept over `[a, b]`.
    fn near(&self, solid: &Solid, (a, b): (f64, f64)) -> Result<Near, String> {
        let middle = 0.5 * (a + b);
        let half = 0.5 * (b - a);
        let frame = self.centreline.frame_at(middle)?;
        let bounds = self.centreline.derivative_bounds(a, b)?;
        let speed = frame.speed - bounds.1 * half;
        if speed < 0.5 {
            // The lateral axis's turn is unbounded over so long a step;
            // only a shorter one bounds it.
            return Ok(Near::Within(format!(
                "between {a} and {b} m along it the turn of the section cannot be bounded"
            )));
        }
        let moved = half * (bounds.0 + self.envelope.lateral_reach() * bounds.1 / speed);
        let (low, high, reach) = solid.framed_box(&frame);
        let error = self.centreline.frame_error(reach.max(self.reach()), bounds);
        let radius = (moved + solid.deviation + error) * (1.0 + 1e-9) + slack(reach) * 1e-3;
        let misses = low[0] > radius
            || high[0] < -radius
            || low[1] > self.high[0] + radius
            || high[1] < self.low[0] - radius
            || low[2] > self.high[1] + radius
            || high[2] < self.low[1] - radius;
        if misses {
            return Ok(Near::Clear);
        }
        let vertices = self.envelope.vertices();
        for triangle in &solid.triangles {
            let framed = triangle.map(|vertex| frame.coordinates(vertex));
            let outside = (0..3).any(|axis| {
                let (lowest, highest) = framed.iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(lowest, highest), p| (lowest.min(p[axis]), highest.max(p[axis])),
                );
                match axis {
                    0 => lowest > radius || highest < -radius,
                    _ => {
                        lowest > self.high[axis - 1] + radius
                            || highest < self.low[axis - 1] - radius
                    }
                }
            });
            if outside {
                continue;
            }
            let projected = clipped(&framed, radius);
            if !projected.is_empty() && within(&projected, vertices, radius) {
                return Ok(Near::Within(format!(
                    "between {a} and {b} m along it it comes within {radius} m of the \
                     envelope there"
                )));
            }
        }
        // No triangle that near: the placed envelope lies wholly inside the
        // body, or wholly outside it, farther than the radius.
        if solid.winding(frame.place(vertices[0])) != 0 {
            Ok(Near::Inside(middle))
        } else {
            Ok(Near::Clear)
        }
    }

    /// A distance in `[a, b]` where a point of `solid` is proven inside the
    /// envelope, if a section at `at` proves one.
    fn proven_at(&self, solid: &Solid, object: &ObjectId, at: f64) -> Result<bool, String> {
        let frame = self.centreline.frame_at(at)?;
        let bounds = self.centreline.derivative_bounds(at, at)?;
        let (_, _, reach) = solid.framed_box(&frame);
        let radius = solid.deviation + self.centreline.frame_error(reach.max(self.reach()), bounds);
        let (cut, band) = cut_and_band(solid, &frame, radius);
        if cut.is_empty() {
            return Ok(false);
        }
        let section = BodySection::try_new(object.clone(), cut, band, radius)
            .map_err(|error| error.to_string())?;
        let vertices = self.envelope.vertices();
        let scale = self
            .high
            .iter()
            .chain(&self.low)
            .fold(1.0_f64, |scale, value| scale.max(value.abs()));
        let margin = 1e-9 * scale;
        let mut candidates = section.witnesses();
        let count = vertices.len();
        #[allow(clippy::cast_precision_loss)]
        let centroid = [
            vertices.iter().map(|v| v[0]).sum::<f64>() / count as f64,
            vertices.iter().map(|v| v[1]).sum::<f64>() / count as f64,
        ];
        candidates.push(centroid);
        for vertex in vertices {
            for share in [0.01, 0.1, 0.5] {
                candidates.push([
                    vertex[0] + share * (centroid[0] - vertex[0]),
                    vertex[1] + share * (centroid[1] - vertex[1]),
                ]);
            }
        }
        // A coarse grid over the envelope's box.
        for i in 1..8 {
            for j in 1..8 {
                let (fi, fj) = (f64::from(i) / 8.0, f64::from(j) / 8.0);
                candidates.push([
                    self.low[0] + fi * (self.high[0] - self.low[0]),
                    self.low[1] + fj * (self.high[1] - self.low[1]),
                ]);
            }
        }
        Ok(candidates.into_iter().any(|point| {
            self.envelope.contains_beyond(point, margin) && section.surely_inside(point)
        }))
    }

    /// Whether `solid` reaches into the envelope swept over `[from, to]`
    /// plan distances, stepping by `step`.
    fn intrusion(
        &self,
        solid: &Solid,
        object: &ObjectId,
        (from, to): (f64, f64),
        step: f64,
    ) -> Result<Intrusion, String> {
        let mut steps = Vec::new();
        let mut start = from;
        loop {
            let end = (start + step).min(to);
            steps.push((start, end, 0_u32));
            if end >= to {
                break;
            }
            start = end;
        }
        steps.reverse();
        let mut spent = 0_usize;
        let mut undecided: Option<String> = None;
        let sure = |at: f64| {
            AlignmentInterval::try_new(at, at)
                .map(|distance| Intrusion::Sure { distance })
                .map_err(|error| error.to_string())
        };
        while let Some((a, b, depth)) = steps.pop() {
            spent += 1;
            if spent > SWEEP_BUDGET {
                undecided.get_or_insert_with(|| {
                    format!(
                        "after {SWEEP_BUDGET} stretches tested it is still undecided from {a} m \
                         along it on"
                    )
                });
                break;
            }
            let why = match self.near(solid, (a, b))? {
                Near::Clear => continue,
                Near::Inside(at) => return sure(at),
                Near::Within(why) => why,
            };
            let middle = 0.5 * (a + b);
            let mut stations = vec![middle];
            if depth == 0 {
                stations.extend([a, b]);
            }
            for at in stations {
                if self.proven_at(solid, object, at)? {
                    return sure(at);
                }
            }
            if depth < DEPTH && b - a > RESOLUTION {
                steps.push((middle, b, depth + 1));
                steps.push((a, middle, depth + 1));
            } else {
                undecided.get_or_insert_with(|| {
                    format!(
                        "{why}, and no section proves it inside; the step is too coarse to \
                         decide"
                    )
                });
            }
        }
        Ok(match undecided {
            Some(reason) => Intrusion::Possible(reason),
            None => Intrusion::Clear,
        })
    }
}

impl IfcAlignmentService {
    pub(super) fn section(&self, request: &SectionRequest) -> Result<Section, AlignmentError> {
        let alignment = request.alignment();
        let centreline = self.centreline(alignment)?;
        let geometry = &self.geometry;
        let distance = centreline.distance_of(request.station())?;
        let frame = centreline
            .frame_at(distance)
            .map_err(AlignmentError::Unavailable)?;
        let bounds = centreline
            .derivative_bounds(distance, distance)
            .map_err(AlignmentError::Unavailable)?;
        let mut bodies = Vec::new();
        let mut exact = true;
        for object in request.objects() {
            if geometry.parts_of(object).is_some() {
                return Err(AlignmentError::Unavailable(format!(
                    "{object} is measured through its parts, whose overlaps a cut cannot tell \
                     apart"
                )));
            }
            let body =
                match solid(geometry, centreline, object).map_err(AlignmentError::Unavailable)? {
                    None => BodySection::try_new(object.clone(), Vec::new(), Vec::new(), 0.0)?,
                    Some(solid) => {
                        let (_, _, reach) = solid.framed_box(&frame);
                        let radius = solid.deviation + centreline.frame_error(reach, bounds);
                        let (cut, band) = cut_and_band(&solid, &frame, radius);
                        BodySection::try_new(object.clone(), cut, band, radius)?
                    }
                };
            exact &= body.is_exact();
            bodies.push(body);
        }
        let locator = format!(
            "section of {} at station {} ({distance} m along it)",
            centreline.locator,
            request.station()
        );
        Section::try_new(
            request.clone(),
            bodies,
            evidence(&alignment.source, locator, exact),
        )
    }

    pub(super) fn envelope(
        &self,
        request: &EnvelopeRequest,
    ) -> Result<EnvelopeSweep, AlignmentError> {
        let alignment = request.alignment();
        let centreline = self.centreline(alignment)?;
        let geometry = &self.geometry;
        let (from, to) = request.range();
        let ends = (centreline.distance_of(from)?, centreline.distance_of(to)?);
        let range = (ends.0.min(ends.1), ends.0.max(ends.1));
        if (range.1 - range.0) / request.step() > MAX_STEPS {
            return Err(AlignmentError::Unavailable(format!(
                "the range of {} m holds more than {MAX_STEPS} steps of {} m",
                range.1 - range.0,
                request.step()
            )));
        }
        let vertices = request.envelope().vertices();
        let fold = |axis: usize| {
            vertices
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), vertex| {
                    (low.min(vertex[axis]), high.max(vertex[axis]))
                })
        };
        let (lateral, up) = (fold(0), fold(1));
        let sweep = Sweep {
            centreline,
            envelope: request.envelope(),
            low: [lateral.0, up.0],
            high: [lateral.1, up.1],
        };
        let mut intrusions = Vec::new();
        for object in request.bodies() {
            let unavailable =
                |reason: String| AlignmentError::Unavailable(format!("{object}: {reason}"));
            let intrusion = match solid(geometry, centreline, object).map_err(unavailable)? {
                None => Intrusion::Clear,
                Some(solid) => sweep
                    .intrusion(&solid, object, range, request.step())
                    .map_err(unavailable)?,
            };
            intrusions.push((object.clone(), intrusion));
        }
        let decided = intrusions
            .iter()
            .all(|(_, intrusion)| !matches!(intrusion, Intrusion::Possible(_)));
        let locator = format!(
            "clearance envelope swept along {} over [{}, {}] m in steps of {} m",
            centreline.locator,
            range.0,
            range.1,
            request.step()
        );
        EnvelopeSweep::try_new(
            request.clone(),
            intrusions,
            evidence(&alignment.source, locator, decided),
        )
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use super::*;
    use axiolid_core::Vec3;
    use axiolid_curve::CurvatureLaw;
    use axiolid_mesh::TriMesh;
    use axioval::engine::{AlignmentService, SectionAxis};
    use axioval::ir::SourceId;

    use super::super::tests::centreline;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    /// The closed box `[low, high]`, its faces turned outwards.
    fn cuboid(low: [f64; 3], high: [f64; 3]) -> TriMesh {
        let positions = (0..8)
            .map(|corner| {
                let pick = |axis: usize| {
                    if corner & (1 << axis) == 0 {
                        low[axis]
                    } else {
                        high[axis]
                    }
                };
                Vec3::new(pick(0), pick(1), pick(2))
            })
            .collect();
        let quads: [[u32; 4]; 6] = [
            [0, 4, 6, 2],
            [1, 3, 7, 5],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 2, 3, 1],
            [4, 5, 7, 6],
        ];
        let indices = quads
            .iter()
            .flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d])
            .collect();
        TriMesh::new(positions, indices)
    }

    /// A straight centreline 100 m along `+X`, level at 10 m.
    fn straight() -> Centreline {
        centreline(CurvatureLaw::Constant { curvature: 0.0 }, 100.0, 0.0)
    }

    /// A left-turning arc of radius 50 m from the origin heading `+X`,
    /// level at 10 m: its centre is at `(0, 50)`.
    fn curved() -> Centreline {
        centreline(CurvatureLaw::Constant { curvature: 0.02 }, 60.0, 0.0)
    }

    /// The plan point `lateral` left of the arc at `distance` along it.
    fn on_arc(distance: f64, lateral: f64) -> [f64; 2] {
        let angle = distance / 50.0;
        let radius = 50.0 - lateral;
        [radius * angle.sin(), 50.0 - radius * angle.cos()]
    }

    fn service(centreline: Centreline, bodies: &[(&str, TriMesh)]) -> IfcAlignmentService {
        let mut geometry = AxiolidGeometry::new();
        for (local, mesh) in bodies {
            geometry = geometry.with_mesh(id(local), mesh.clone());
        }
        IfcAlignmentService {
            points: BTreeMap::new(),
            alignments: BTreeMap::from([(id("axis"), Ok(centreline))]),
            geometry,
        }
    }

    /// A 4 m wide, 5 m high envelope above the gradient line.
    fn envelope() -> SectionPolygon {
        SectionPolygon::try_new(vec![[-2.0, 0.0], [2.0, 0.0], [2.0, 5.0], [-2.0, 5.0]]).unwrap()
    }

    fn sweep(
        service: &IfcAlignmentService,
        bodies: &[&str],
        range: (f64, f64),
        step: f64,
    ) -> Result<EnvelopeSweep, AlignmentError> {
        let request = EnvelopeRequest::try_new(
            id("axis"),
            envelope(),
            range,
            step,
            bodies.iter().map(|local| id(local)).collect(),
        )
        .unwrap();
        service.measure_envelope(&request)
    }

    fn verdict<'s>(sweep: &'s EnvelopeSweep, local: &str) -> &'s Intrusion {
        &sweep
            .intrusions()
            .iter()
            .find(|(body, _)| body == &id(local))
            .unwrap()
            .1
    }

    #[test]
    fn a_box_has_its_section_area_and_thickness_at_a_station() {
        // 2 m wide, 0.5 m high, from 4 m to 6 m along the axis.
        let service = service(
            straight(),
            &[("box", cuboid([4.0, -1.0, 10.0], [6.0, 1.0, 10.5]))],
        );
        let cut = |station: f64| {
            service
                .measure_section(
                    &SectionRequest::try_new(id("axis"), station, vec![id("box")]).unwrap(),
                )
                .unwrap()
        };
        let section = cut(5.0);
        let [body] = section.bodies() else {
            panic!("one body");
        };
        let area = body.area().unwrap();
        assert!(area.lower() <= 1.0 && area.upper() >= 1.0, "{area:?}");
        // The band around the cut, within the frame's numerical error.
        assert!(area.upper() - area.lower() < 1e-4, "{area:?}");
        let up = body.extent(SectionAxis::Up).unwrap().unwrap();
        assert!(up.lower() <= 0.5 && up.upper() >= 0.5 && up.upper() - up.lower() < 1e-4);
        let lateral = body.extent(SectionAxis::Lateral).unwrap().unwrap();
        assert!(lateral.lower() <= 2.0 && lateral.upper() >= 2.0);
        assert!(lateral.upper() - lateral.lower() < 1e-4, "{lateral:?}");
        assert!(body.surely_inside([0.0, 0.25]));
        assert!(!body.possibly_inside([0.0, 1.0]));
        assert!(!section.evidence().exact);
        // Beyond the box the plane misses it.
        let missed = cut(50.0);
        assert!(
            missed.bodies()[0]
                .extent(SectionAxis::Up)
                .unwrap()
                .is_none()
        );
        // Off the range, refused.
        let refused = service
            .measure_section(&SectionRequest::try_new(id("axis"), 120.0, vec![id("box")]).unwrap());
        assert!(
            matches!(refused, Err(AlignmentError::OffRange(ref reason)) if reason.contains("120")),
            "{refused:?}"
        );
    }

    #[test]
    fn a_box_cut_across_a_curve_has_its_height_and_holds_its_centre() {
        // A box around the point 2 m left of the arc at 20 m.
        let [x, y] = on_arc(20.0, 2.0);
        let service = service(
            curved(),
            &[(
                "box",
                cuboid([x - 0.2, y - 0.2, 10.0], [x + 0.2, y + 0.2, 11.0]),
            )],
        );
        let section = service
            .measure_section(&SectionRequest::try_new(id("axis"), 20.0, vec![id("box")]).unwrap())
            .unwrap();
        let body = &section.bodies()[0];
        let up = body.extent(SectionAxis::Up).unwrap().unwrap();
        assert!(
            up.lower() <= 1.0 && up.upper() >= 1.0 && up.upper() - up.lower() < 1e-4,
            "{up:?}"
        );
        assert!(body.surely_inside([2.0, 0.5]), "{body:?}");
        assert!(!body.possibly_inside([0.0, 0.5]), "{body:?}");
    }

    #[test]
    fn bodies_in_the_envelope_are_found_and_bodies_clear_of_it_pass() {
        let service = service(
            straight(),
            &[
                // Inside the envelope from 30 m to 32 m.
                ("inside", cuboid([30.0, -0.5, 11.0], [32.0, 0.5, 12.0])),
                // Beside it, 3 m to 4 m to the left.
                ("beside", cuboid([20.0, 3.0, 10.0], [60.0, 4.0, 12.0])),
                // Just over its top, from 5.2 m above the gradient line.
                ("above", cuboid([10.0, -3.0, 15.2], [20.0, 3.0, 16.0])),
            ],
        );
        let swept = sweep(&service, &["inside", "beside", "above"], (0.0, 100.0), 10.0).unwrap();
        assert!(
            matches!(verdict(&swept, "inside"), Intrusion::Sure { .. }),
            "{swept:?}"
        );
        assert_eq!(verdict(&swept, "beside"), &Intrusion::Clear);
        assert_eq!(verdict(&swept, "above"), &Intrusion::Clear);
        assert_eq!(swept.count(), (1, 1));
        assert!(swept.evidence().exact);
        // Outside the range, the inside body is clear.
        let swept = sweep(&service, &["inside"], (40.0, 100.0), 10.0).unwrap();
        assert_eq!(verdict(&swept, "inside"), &Intrusion::Clear);
    }

    #[test]
    fn bodies_along_a_curve_are_found_or_pass() {
        let [x, y] = on_arc(30.0, 0.0);
        let [fx, fy] = on_arc(30.0, 8.0);
        let service = service(
            curved(),
            &[
                (
                    "inside",
                    cuboid([x - 0.3, y - 0.3, 11.0], [x + 0.3, y + 0.3, 12.0]),
                ),
                (
                    "far",
                    cuboid([fx - 0.3, fy - 0.3, 11.0], [fx + 0.3, fy + 0.3, 12.0]),
                ),
            ],
        );
        let swept = sweep(&service, &["far", "inside"], (0.0, 60.0), 10.0).unwrap();
        assert!(
            matches!(verdict(&swept, "inside"), Intrusion::Sure { .. }),
            "{swept:?}"
        );
        assert_eq!(verdict(&swept, "far"), &Intrusion::Clear);
    }

    /// engine#253's Done-when: a body crossing the envelope strictly
    /// between two sampled stations, thinner than the step, is found or
    /// left possible, never passed.
    #[test]
    fn a_thin_body_crossing_between_samples_is_never_passed() {
        // Where the track is at a distance, and its heading there.
        type Place = fn(f64) -> ([f64; 2], f64);
        let cases: [(Centreline, Place); 2] = [
            (straight(), |distance| ([distance, 0.0], 0.0)),
            (curved(), |distance| {
                (on_arc(distance, 0.0), distance / 50.0)
            }),
        ];
        for (centreline, place) in cases {
            // Plates 2 cm and 1 mm thick square across the track at 41 m,
            // strictly between the samples at 40 m and 50 m (and every
            // section a halving reaches before 2^-10 of the step), from 1 m
            // below to 3 m above the gradient line and 3 m to either side.
            let plate = |thickness: f64| {
                let ([x, y], heading) = place(41.0);
                let (along, across) = (
                    [heading.cos(), heading.sin()],
                    [-heading.sin(), heading.cos()],
                );
                let positions = (0..8)
                    .map(|corner: u32| {
                        let thick = if corner & 1 == 0 { -0.5 } else { 0.5 } * thickness;
                        let wide = if corner & 2 == 0 { -3.0 } else { 3.0 };
                        let high = if corner & 4 == 0 { 9.0 } else { 13.0 };
                        Vec3::new(
                            x + thick * along[0] + wide * across[0],
                            y + thick * along[1] + wide * across[1],
                            high,
                        )
                    })
                    .collect();
                let mut mesh = cuboid([0.0; 3], [1.0; 3]);
                mesh.positions = positions;
                mesh
            };
            let service = service(
                centreline,
                &[("plate", plate(0.02)), ("foil", plate(0.001))],
            );
            for step in [10.0, 50.0] {
                let swept = sweep(&service, &["foil", "plate"], (0.0, 50.0), step).unwrap();
                for body in ["plate", "foil"] {
                    assert_ne!(
                        verdict(&swept, body),
                        &Intrusion::Clear,
                        "{body} at step {step}: {swept:?}"
                    );
                }
                // The foil is thinner than ten halvings resolve: left
                // possible, with the stretch named.
                let Intrusion::Possible(reason) = verdict(&swept, "foil") else {
                    panic!("the foil must be undecided: {swept:?}");
                };
                assert!(
                    reason.contains("too coarse") && reason.contains("41"),
                    "{reason}"
                );
                let (sure, possible) = swept.count();
                assert!(sure <= possible && possible == 2, "{swept:?}");
            }
            // Halving the step finds the 2 cm plate.
            let swept = sweep(&service, &["plate"], (0.0, 50.0), 10.0).unwrap();
            assert!(
                matches!(verdict(&swept, "plate"), Intrusion::Sure { .. }),
                "{swept:?}"
            );
        }
    }

    #[test]
    fn a_body_engulfing_the_envelope_is_found_without_a_near_face() {
        // A block holding the whole envelope, from 20 m to 80 m.
        let service = service(
            straight(),
            &[("block", cuboid([20.0, -10.0, 0.0], [80.0, 10.0, 30.0]))],
        );
        let swept = sweep(&service, &["block"], (40.0, 60.0), 20.0).unwrap();
        assert!(
            matches!(verdict(&swept, "block"), Intrusion::Sure { .. }),
            "{swept:?}"
        );
    }

    #[test]
    fn unreadable_bodies_and_alignments_are_refused_by_name() {
        let mut open = cuboid([30.0, -0.5, 11.0], [32.0, 0.5, 12.0]);
        open.indices.truncate(30);
        let mut service = service(straight(), &[("open", open)]);
        service.geometry = std::mem::take(&mut service.geometry)
            .with_unmeasured(id("lost"), "no body representation");
        let refused = sweep(&service, &["open"], (0.0, 50.0), 10.0);
        let Err(AlignmentError::Unavailable(reason)) = refused else {
            panic!("an open mesh must be refused: {refused:?}");
        };
        assert!(reason.contains("no closed solid"), "{reason}");
        let refused = sweep(&service, &["lost"], (0.0, 50.0), 10.0);
        let Err(AlignmentError::Unavailable(reason)) = refused else {
            panic!("an unmeasured body must be refused: {refused:?}");
        };
        assert!(reason.contains("unmeasured"), "{reason}");
        // An alignment placed off the identity is refused as positions
        // refuse it (openbimrs/ifc#357).
        let mut moved = ifc_geometry::Transform::identity();
        moved.origin = [10.0, 0.0, 0.0];
        service
            .alignments
            .insert(id("axis"), Err(super::super::rigid(&moved).unwrap_err()));
        let refused = sweep(&service, &["open"], (0.0, 50.0), 10.0);
        let Err(AlignmentError::Unavailable(reason)) = refused else {
            panic!("an alignment off the identity must be refused: {refused:?}");
        };
        assert!(reason.contains("openbimrs/ifc#357"), "{reason}");
        let refused = service
            .measure_section(&SectionRequest::try_new(id("axis"), 5.0, vec![id("open")]).unwrap());
        assert!(
            matches!(&refused, Err(AlignmentError::Unavailable(reason)) if reason.contains("openbimrs/ifc#357")),
            "{refused:?}"
        );
    }
}
