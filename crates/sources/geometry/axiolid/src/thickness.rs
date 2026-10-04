//! How thick a closed body is along a direction: the length of each line
//! along it between the faces looking forward and those looking back.
//!
//! ADR 0004: this module measures thickness; what thickness a member needs
//! is a rule's decision.
//!
//! A face looks along the direction when its outward normal lies within
//! 60° of it (forward) or of its opposite (back); steeper faces are the
//! body's sides and are not crossed. Along a line, the body's thickness is
//! the sum of the forward faces' positions less the back faces'. Over the
//! plane square to the direction this is linear between the corners of the
//! two faces' projected triangles and the crossings of their edges, so its
//! least and greatest values are found there: at every projected vertex and
//! every crossing where a line meets as many forward as back faces.
//!
//! The thicknesses are widened by a part in a billion of the body's size
//! for the rounding of the projections, and by twice a tessellation's chord
//! deviation; the measurement is never exact.

use axioval_engine::{MetricDirection, Thickness, VerticalExtentError};
use axioval_ir::{Evidence, ObjectId};

use crate::geometry::Triangle;

/// A face whose outward normal is within this cosine of the direction
/// looks along it.
const LOOKS_ALONG: f64 = 0.5;

/// At most this many pairs of projected edges are crossed.
const MAX_EDGE_PAIRS: usize = 4_000_000;

/// A triangle projected onto the plane square to the direction, each
/// corner with its position along the direction.
struct Projected {
    corners: [[f64; 2]; 3],
    along: [f64; 3],
}

impl Projected {
    /// The position along the direction at plan point `q`, if `q` lies in
    /// the triangle (within `slack` of its edges).
    fn at(&self, q: [f64; 2], slack: f64) -> Option<f64> {
        let [a, b, c] = self.corners;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() <= f64::EPSILON * slack * slack {
            return None;
        }
        let weight = |p: [f64; 2], r: [f64; 2]| {
            ((r[0] - p[0]) * (q[1] - p[1]) - (r[1] - p[1]) * (q[0] - p[0])) / area
        };
        let (wa, wb, wc) = (weight(b, c), weight(c, a), weight(a, b));
        let tolerance = -slack.max(f64::EPSILON);
        (wa >= tolerance && wb >= tolerance && wc >= tolerance)
            .then(|| wa * self.along[0] + wb * self.along[1] + wc * self.along[2])
    }

    fn edges(&self) -> [([f64; 2], [f64; 2]); 3] {
        let [a, b, c] = self.corners;
        [(a, b), (b, c), (c, a)]
    }
}

/// Where two segments cross, if they do.
fn crossing(
    (start, end): ([f64; 2], [f64; 2]),
    (other_start, other_end): ([f64; 2], [f64; 2]),
) -> Option<[f64; 2]> {
    let along = [end[0] - start[0], end[1] - start[1]];
    let other = [other_end[0] - other_start[0], other_end[1] - other_start[1]];
    let denominator = along[0] * other[1] - along[1] * other[0];
    if denominator == 0.0 {
        return None;
    }
    let gap = [other_start[0] - start[0], other_start[1] - start[1]];
    let on_first = (gap[0] * other[1] - gap[1] * other[0]) / denominator;
    let on_second = (gap[0] * along[1] - gap[1] * along[0]) / denominator;
    ((0.0..=1.0).contains(&on_first) && (0.0..=1.0).contains(&on_second)).then(|| {
        [
            start[0] + on_first * along[0],
            start[1] + on_first * along[1],
        ]
    })
}

/// Two unit vectors square to `d` and to each other.
fn plane(d: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let helper = if d[0].abs() <= d[1].abs() && d[0].abs() <= d[2].abs() {
        [1.0, 0.0, 0.0]
    } else if d[1].abs() <= d[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let unit = |v: [f64; 3]| {
        let length = v.iter().map(|c| c * c).sum::<f64>().sqrt();
        v.map(|c| c / length)
    };
    let u = unit(cross(d, helper));
    (u, unit(cross(d, u)))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The positions along each line of the faces it meets, the same face met
/// twice (on a shared edge) counted once.
fn distinct(mut positions: Vec<f64>, tolerance: f64) -> Vec<f64> {
    positions.sort_by(f64::total_cmp);
    positions.dedup_by(|a, b| (*a - *b).abs() <= tolerance);
    positions
}

/// The thicknesses of the closed mesh `soup` along `direction`.
pub(crate) fn measure(
    object: &ObjectId,
    direction: MetricDirection,
    soup: &[Triangle],
    tessellation: Option<f64>,
    closed: bool,
) -> Result<Thickness, VerticalExtentError> {
    let unavailable = |reason: &str| VerticalExtentError::Unavailable(format!("{object} {reason}"));
    if !closed {
        return Err(unavailable(
            "is not closed, so it has no inside to be thick",
        ));
    }
    let outward = crate::face_normals::signed_volume(soup).signum();
    let d = direction.components();
    let (u, v) = plane(d);
    let mut forward = Vec::new();
    let mut back = Vec::new();
    let mut size = 1.0_f64;
    for triangle in soup {
        let points = triangle.map(|point| point.to_array());
        for point in &points {
            size = size.max(point.iter().fold(0.0_f64, |m, c| m.max(c.abs())));
        }
        let (e1, e2) = (
            [0, 1, 2].map(|i| points[1][i] - points[0][i]),
            [0, 1, 2].map(|i| points[2][i] - points[0][i]),
        );
        let normal = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let length = dot(normal, normal).sqrt();
        if length == 0.0 || !length.is_finite() {
            continue;
        }
        let facing = outward * dot(normal, d) / length;
        let projected = Projected {
            corners: points.map(|p| [dot(p, u), dot(p, v)]),
            along: points.map(|p| dot(p, d)),
        };
        if facing > LOOKS_ALONG {
            forward.push(projected);
        } else if facing < -LOOKS_ALONG {
            back.push(projected);
        }
    }
    if forward.is_empty() || back.is_empty() {
        return Err(unavailable(
            "has no faces looking along the direction and back",
        ));
    }
    if forward.len().saturating_mul(back.len()).saturating_mul(9) > MAX_EDGE_PAIRS {
        return Err(unavailable("is too detailed to measure its thickness"));
    }
    let slack = 1e-9 * size;
    let mut candidates: Vec<[f64; 2]> = forward
        .iter()
        .chain(&back)
        .flat_map(|triangle| triangle.corners)
        .collect();
    for front in &forward {
        for behind in &back {
            for edge in front.edges() {
                for other in behind.edges() {
                    candidates.extend(crossing(edge, other));
                }
            }
        }
    }
    let mut least = f64::INFINITY;
    let mut most = f64::NEG_INFINITY;
    for q in candidates {
        let exits = distinct(
            forward.iter().filter_map(|t| t.at(q, 1e-12)).collect(),
            slack,
        );
        let entries = distinct(back.iter().filter_map(|t| t.at(q, 1e-12)).collect(), slack);
        if exits.is_empty() || exits.len() != entries.len() {
            continue;
        }
        let thickness = exits.iter().sum::<f64>() - entries.iter().sum::<f64>();
        least = least.min(thickness);
        most = most.max(thickness);
    }
    if !least.is_finite() {
        return Err(unavailable(
            "has no line meeting as many faces looking forward as back",
        ));
    }
    let margin = slack + 2.0 * tessellation.unwrap_or(0.0);
    let evidence = Evidence {
        source: object.source.clone(),
        locator: format!("thickness:{object}:[{},{},{}]", d[0], d[1], d[2]),
        exact: false,
    };
    Thickness::try_new(
        object.clone(),
        direction,
        (least - margin).max(0.0),
        (most + margin).max(margin),
        evidence,
    )
}
