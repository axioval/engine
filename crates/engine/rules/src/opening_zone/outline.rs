//! Free section outlines: a polygon with voids, as the body set states an
//! arbitrary profile, and where a rectangle lies against it.

use super::face::{ROUNDING, Span};

/// A point in a profile's X-Y plane.
pub(crate) type Point = [f64; 2];

/// A simple polygon with voids, in its profile's coordinates. Every edge is
/// straight, so every measure taken of it is exact.
#[derive(Debug)]
pub(crate) struct Polygon {
    /// The outer ring first, then each void.
    rings: Vec<Vec<Point>>,
}

impl Polygon {
    /// Checks the rings form one region: each of at least three finite
    /// vertices enclosing an area, no two edges meeting except neighbours
    /// at their common vertex, each void inside the outer ring and outside
    /// every other void.
    pub(crate) fn new(outer: Vec<Point>, voids: Vec<Vec<Point>>) -> Result<Self, String> {
        let mut rings = vec![outer];
        rings.extend(voids);
        for (index, ring) in rings.iter().enumerate() {
            let name = ring_name(index);
            if ring.len() < 3 {
                return Err(format!("the {name} has {} vertices", ring.len()));
            }
            if ring.iter().flatten().any(|value| !value.is_finite()) {
                return Err(format!("the {name} has a coordinate that is not finite"));
            }
            if signed_area(ring).abs() <= ROUNDING * ROUNDING {
                return Err(format!("the {name} encloses no area"));
            }
        }
        let polygon = Self { rings };
        let edges: Vec<(usize, usize, Point, Point)> = polygon.indexed_edges().collect();
        for (i, a) in edges.iter().enumerate() {
            for b in &edges[i + 1..] {
                let neighbours = a.0 == b.0 && {
                    let n = polygon.rings[a.0].len();
                    (a.1 + 1) % n == b.1 || (b.1 + 1) % n == a.1
                };
                if !neighbours && segments_meet(a.2, a.3, b.2, b.3) {
                    return Err(if a.0 == b.0 {
                        format!("the {} crosses or touches itself", ring_name(a.0))
                    } else {
                        format!(
                            "the {} and the {} cross or touch",
                            ring_name(a.0),
                            ring_name(b.0)
                        )
                    });
                }
            }
        }
        for (index, ring) in polygon.rings.iter().enumerate().skip(1) {
            let vertex = ring[0];
            if !inside_ring(&polygon.rings[0], vertex) {
                return Err(format!("the {} lies outside the outline", ring_name(index)));
            }
            if polygon.rings[1..]
                .iter()
                .enumerate()
                .any(|(other, void)| other + 1 != index && inside_ring(void, vertex))
            {
                return Err(format!("the {} lies inside another void", ring_name(index)));
            }
        }
        Ok(polygon)
    }

    /// Each edge with its ring, its index in the ring and its end points.
    fn indexed_edges(&self) -> impl Iterator<Item = (usize, usize, Point, Point)> + '_ {
        self.rings.iter().enumerate().flat_map(|(ring, vertices)| {
            (0..vertices.len()).map(move |index| {
                (
                    ring,
                    index,
                    vertices[index],
                    vertices[(index + 1) % vertices.len()],
                )
            })
        })
    }

    /// How far the outline reaches along `(p, q)`, least and most.
    pub(crate) fn range(&self, p: f64, q: f64) -> Span {
        self.rings[0]
            .iter()
            .map(|[x, y]| p * x + q * y)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
                (low.min(value), high.max(value))
            })
    }

    /// The area the outline encloses less its voids.
    pub(crate) fn area(&self) -> f64 {
        signed_area(&self.rings[0]).abs()
            - self.rings[1..]
                .iter()
                .map(|ring| signed_area(ring).abs())
                .sum::<f64>()
    }

    /// The outline's extent along the profile's X and Y.
    pub(crate) fn bounds(&self) -> [Span; 2] {
        [self.range(1.0, 0.0), self.range(0.0, 1.0)]
    }

    /// Where the region is an L: one ring of six vertices whose every edge
    /// runs along X or Y. `None` for any other shape.
    ///
    /// The L leaves out one corner of the box around it, the notch. Its
    /// leg running the whole height along Y is the web, and the part of
    /// the height beside the notch is the web zone, clear of the leg
    /// running along X (the flange, or a precast ledge), whatever the legs'
    /// thicknesses.
    pub(crate) fn l_shape(&self) -> Option<LShape> {
        let [ring] = self.rings.as_slice() else {
            return None;
        };
        if ring.len() != 6 {
            return None;
        }
        let near = |a: f64, b: f64| (a - b).abs() <= ROUNDING;
        if !ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .all(|(a, b)| near(a[0], b[0]) || near(a[1], b[1]))
        {
            return None;
        }
        let [(x0, x1), (y0, y1)] = self.bounds();
        let inner: Vec<Point> = ring
            .iter()
            .copied()
            .filter(|p| !(near(p[0], x0) || near(p[0], x1) || near(p[1], y0) || near(p[1], y1)))
            .collect();
        let [reflex] = inner.as_slice() else {
            return None;
        };
        let missing: Vec<Point> = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
            .into_iter()
            .filter(|corner| {
                !ring
                    .iter()
                    .any(|p| near(p[0], corner[0]) && near(p[1], corner[1]))
            })
            .collect();
        let [corner] = missing.as_slice() else {
            return None;
        };
        Some(LShape {
            web_zone: (reflex[1].min(corner[1]), reflex[1].max(corner[1])),
            web: if near(corner[0], x1) {
                (x0, reflex[0])
            } else {
                (reflex[0], x1)
            },
        })
    }

    /// Whether `point` lies inside the region: inside the outline and
    /// outside every void.
    fn contains(&self, point: Point) -> bool {
        self.rings
            .iter()
            .filter(|ring| inside_ring(ring, point))
            .count()
            % 2
            == 1
    }

    /// Where the rectangle `rect` (spans along the profile's X and Y) lies
    /// against the region, looking along `axis` (0 for X, 1 for Y): its
    /// clear distance to the boundary towards lower and towards higher
    /// coordinates across its whole width, or `None` when the boundary
    /// passes through its interior or it lies outside.
    ///
    /// A boundary edge meeting the rectangle's sides only, such as the face
    /// of a wall an opening passes through, does not enter it. Distances
    /// and contact are decided within [`ROUNDING`].
    pub(crate) fn clearance(&self, rect: [Span; 2], axis: usize) -> Option<Span> {
        let across = 1 - axis;
        let (along, band) = (rect[axis], rect[across]);
        let band = if band.1 - band.0 > 2.0 * ROUNDING {
            (band.0 + ROUNDING, band.1 - ROUNDING)
        } else {
            let middle = f64::midpoint(band.0, band.1);
            (middle, middle)
        };
        let (mut low, mut high) = (f64::INFINITY, f64::INFINITY);
        for (_, _, a, b) in self.indexed_edges() {
            let Some((first, last)) = clip(a, b, across, band) else {
                continue;
            };
            let (near, far) = (first[axis].min(last[axis]), first[axis].max(last[axis]));
            if far <= along.0 + ROUNDING {
                low = low.min(along.0 - far);
            } else if near >= along.1 - ROUNDING {
                high = high.min(near - along.1);
            } else {
                return None;
            }
        }
        let mut centre = [0.0; 2];
        centre[axis] = f64::midpoint(along.0, along.1);
        centre[across] = f64::midpoint(band.0, band.1);
        self.contains(centre)
            .then_some((low.max(0.0), high.max(0.0)))
    }
}

/// An L-shaped section in its profile's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LShape {
    /// The part of Y beside the notch, clear of the leg along X.
    pub(crate) web_zone: Span,
    /// The span along X of the leg running the whole height.
    pub(crate) web: Span,
}

fn ring_name(index: usize) -> String {
    if index == 0 {
        "outline".to_owned()
    } else {
        format!("void {index}")
    }
}

/// The shoelace area, positive when the ring runs anticlockwise.
fn signed_area(ring: &[Point]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

/// Even-odd ray crossing: whether `point` lies inside `ring`.
fn inside_ring(ring: &[Point], point: Point) -> bool {
    let mut inside = false;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if point[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// The part of segment `a`-`b` whose coordinate `across` lies within
/// `band`, as its two end points.
fn clip(a: Point, b: Point, across: usize, band: Span) -> Option<(Point, Point)> {
    let (start, end) = (a[across], b[across]);
    if (start - end).abs() <= f64::EPSILON * start.abs().max(end.abs()).max(1.0) {
        return (band.0 <= start && start <= band.1).then_some((a, b));
    }
    let at = |value: f64| (value - start) / (end - start);
    let (s0, s1) = (at(band.0), at(band.1));
    let (low, high) = (s0.min(s1).max(0.0), s0.max(s1).min(1.0));
    if low > high {
        return None;
    }
    let point = |s: f64| [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])];
    Some((point(low), point(high)))
}

fn orientation(a: Point, b: Point, c: Point) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether the closed segments `a`-`b` and `c`-`d` share a point.
fn segments_meet(a: Point, b: Point, c: Point, d: Point) -> bool {
    let on = |p: Point, q: Point, r: Point| {
        r[0] >= p[0].min(q[0])
            && r[0] <= p[0].max(q[0])
            && r[1] >= p[1].min(q[1])
            && r[1] <= p[1].max(q[1])
    };
    let (d1, d2) = (orientation(c, d, a), orientation(c, d, b));
    let (d3, d4) = (orientation(a, b, c), orientation(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on(c, d, a))
        || (d2 == 0.0 && on(c, d, b))
        || (d3 == 0.0 && on(a, b, c))
        || (d4 == 0.0 && on(a, b, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5 m wall 0.2 m thick, mitred at its far end: 5 m long on y = 0,
    /// 5.2 m on y = 0.2.
    fn mitred() -> Polygon {
        Polygon::new(
            vec![[0.0, 0.0], [5.0, 0.0], [5.2, 0.2], [0.0, 0.2]],
            Vec::new(),
        )
        .unwrap()
    }

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn a_rectangle_through_a_mitred_wall_is_as_far_from_the_end_as_its_short_face() {
        let wall = mitred();
        let (low, high) = wall.clearance([(3.5, 4.5), (0.0, 0.2)], 0).unwrap();
        close(low, 3.5);
        close(high, 0.5);
        // Only half-way through, it meets the mitre further out.
        let (_, high) = wall.clearance([(3.5, 4.5), (0.1, 0.2)], 0).unwrap();
        close(high, 0.6);
        assert!(wall.clearance([(4.5, 5.1), (0.0, 0.2)], 0).is_none());
        assert!(wall.clearance([(5.3, 5.4), (0.0, 0.2)], 0).is_none());
        assert_eq!(wall.bounds(), [(0.0, 5.2), (0.0, 0.2)]);
        close(wall.area(), 1.02);
    }

    #[test]
    fn an_l_has_its_web_beside_the_notch_whatever_the_leg_thicknesses() {
        // A 0.3 m wide, 0.4 m deep L: web 0.12 m thick on the low X side,
        // ledge 0.08 m thick at the bottom, the notch at the top right.
        let l = Polygon::new(
            vec![
                [0.0, 0.0],
                [0.3, 0.0],
                [0.3, 0.08],
                [0.12, 0.08],
                [0.12, 0.4],
                [0.0, 0.4],
            ],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            l.l_shape(),
            Some(LShape {
                web_zone: (0.08, 0.4),
                web: (0.0, 0.12)
            })
        );
        // Turned over, the ledge on top: the web zone lies below it.
        let turned = Polygon::new(
            vec![
                [0.0, 0.0],
                [0.12, 0.0],
                [0.12, 0.32],
                [0.3, 0.32],
                [0.3, 0.4],
                [0.0, 0.4],
            ],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(turned.l_shape().unwrap().web_zone, (0.0, 0.32));
        assert!(mitred().l_shape().is_none());
    }

    #[test]
    fn a_void_inside_the_rectangle_is_the_boundary_crossing_it() {
        let slab = Polygon::new(
            vec![[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]],
            vec![vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]]],
        )
        .unwrap();
        close(slab.area(), 11.0);
        assert!(slab.clearance([(0.5, 2.5), (0.5, 2.5)], 0).is_none());
        assert!(slab.clearance([(1.2, 1.8), (1.2, 1.8)], 0).is_none());
        let (low, high) = slab.clearance([(2.5, 3.5), (0.5, 2.5)], 0).unwrap();
        close(low, 0.5);
        close(high, 0.5);
    }

    #[test]
    fn an_outline_crossing_itself_or_a_stray_void_is_refused() {
        assert!(
            Polygon::new(
                vec![[0.0, 0.0], [3.0, 0.0], [3.0, 2.0], [1.0, -1.0]],
                Vec::new()
            )
            .unwrap_err()
            .contains("itself")
        );
        assert!(
            Polygon::new(
                vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                vec![vec![[2.0, 2.0], [3.0, 2.0], [3.0, 3.0]]]
            )
            .unwrap_err()
            .contains("outside the outline")
        );
        assert!(
            Polygon::new(vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], Vec::new())
                .unwrap_err()
                .contains("no area")
        );
    }
}
