//! How much work one plan overlay may be handed (#419).
//!
//! The overlay subdivides the plane by every edge of both operands at once:
//! each pair of edges whose boxes meet is asked where they cross, and every
//! crossing becomes a vertex the subdivision keeps. Long, slender shadows
//! lying over one another (a finely tessellated beam's faces, each grown by
//! a disc into a fan of long triangles) meet about every other edge, so
//! the pairs grow with the square of the edges and a few thousand
//! triangles asked for gigabytes, which aborted the host. The pairs are
//! counted exactly before the overlay is called, in `O(n log n)` for `n`
//! edges, and an overlay past [`OVERLAY_PAIRS`] is refused by name.

use std::fmt;

use axiolid_overlay::Polygon;

/// The most pairs of edges whose boxes meet that one plan overlay is
/// handed. Every crossing the subdivision keeps comes from such a pair (two
/// segments cross at most twice), so the pairs bound both its time and its
/// memory: a grown round bar of 64 facets meets about 2.4 million pairs and
/// is measured in about 2 s and 0.4 GB, one of 128 facets about 10 million
/// in 10 s and 1.9 GB, and one of 256 facets aborted at 4.5 GB.
pub(crate) const OVERLAY_PAIRS: u64 = 4_000_000;

/// The refusal's reason where an error holds only a static text; it names
/// [`OVERLAY_PAIRS`], which a test keeps in step.
pub(crate) const OVER_BUDGET: &str = "the plan overlay was not built: its edges meet in more than 4000000 pairs, \
     past the overlay budget";

/// An overlay refused before it was built because its operands' edges meet
/// in more pairs than [`OVERLAY_PAIRS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OverBudget {
    /// The edges of both operands.
    pub(crate) edges: usize,
    /// The pairs of them whose boxes meet.
    pub(crate) pairs: u64,
}

impl fmt::Display for OverBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the plan overlay was not built: its {} edges meet in {} pairs, past the \
             overlay budget of {OVERLAY_PAIRS} pairs",
            self.edges, self.pairs
        )
    }
}

/// Refuses an overlay of `operands` whose edges' boxes meet in more pairs
/// than [`OVERLAY_PAIRS`]; every ring of every polygon counts, holes too.
pub(crate) fn admit(operands: &[&[Polygon]]) -> Result<(), OverBudget> {
    let boxes = edge_boxes(operands);
    let pairs = meeting_pairs(&boxes);
    if pairs > OVERLAY_PAIRS {
        return Err(OverBudget {
            edges: boxes.len(),
            pairs,
        });
    }
    Ok(())
}

/// One edge's closed box: `[x_min, x_max, y_min, y_max]`.
type EdgeBox = [f64; 4];

fn edge_boxes(operands: &[&[Polygon]]) -> Vec<EdgeBox> {
    let mut boxes = Vec::new();
    for polygon in operands.iter().flat_map(|polygons| polygons.iter()) {
        for ring in std::iter::once(&polygon.outer).chain(&polygon.holes) {
            let points = &ring.points;
            for (index, a) in points.iter().enumerate() {
                let b = points[(index + 1) % points.len()];
                boxes.push([a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y)]);
            }
        }
    }
    boxes
}

/// The number of pairs of `boxes` that meet, touching included, counted by
/// a sweep along x: when a box opens, every open box meets it in x, and
/// it meets one in y unless that box ends below it or starts above it.
/// Two Fenwick trees over the open boxes' ends and starts in y answer
/// those counts. A non-finite coordinate is ordered by `total_cmp`; the
/// overlay refuses such a point itself.
fn meeting_pairs(boxes: &[EdgeBox]) -> u64 {
    // Opens sort before closes at the same x, so touching boxes meet.
    let mut events: Vec<(f64, bool, usize)> = Vec::with_capacity(2 * boxes.len());
    for (index, edge) in boxes.iter().enumerate() {
        events.push((edge[0], false, index));
        events.push((edge[1], true, index));
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut ys: Vec<f64> = boxes.iter().flat_map(|edge| [edge[2], edge[3]]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| a.total_cmp(b).is_eq());
    let rank = |y: f64| ys.partition_point(|v| v.total_cmp(&y).is_lt());
    // `tops` counts open boxes by their y_max, `bottoms` by their y_min.
    let (mut tops, mut bottoms) = (Fenwick::new(ys.len()), Fenwick::new(ys.len()));
    let (mut open, mut pairs) = (0_u64, 0_u64);
    for (_, closes, index) in events {
        let [_, _, low, high] = boxes[index];
        let (low, high) = (rank(low), rank(high));
        if closes {
            tops.add(high, -1);
            bottoms.add(low, -1);
            open -= 1;
            continue;
        }
        // Open boxes ending below this one's bottom, or starting above its top.
        let below = tops.prefix(low);
        let above = open - bottoms.prefix(high + 1);
        pairs += open - below - above;
        tops.add(high, 1);
        bottoms.add(low, 1);
        open += 1;
    }
    pairs
}

/// Counts by position, with prefix sums in `O(log n)`.
struct Fenwick(Vec<i64>);

impl Fenwick {
    fn new(size: usize) -> Self {
        Self(vec![0; size + 1])
    }

    fn add(&mut self, position: usize, delta: i64) {
        let mut index = position + 1;
        while index < self.0.len() {
            self.0[index] += delta;
            index += index & index.wrapping_neg();
        }
    }

    /// The count at positions below `end`.
    fn prefix(&self, end: usize) -> u64 {
        let (mut index, mut sum) = (end, 0_i64);
        while index > 0 {
            sum += self.0[index];
            index -= index & index.wrapping_neg();
        }
        u64::try_from(sum).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{EdgeBox, OVER_BUDGET, OVERLAY_PAIRS, OverBudget, admit, meeting_pairs};

    /// Every pair checked one by one.
    fn brute(boxes: &[EdgeBox]) -> u64 {
        let mut pairs = 0;
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                if a[0] <= b[1] && b[0] <= a[1] && a[2] <= b[3] && b[2] <= a[3] {
                    pairs += 1;
                }
            }
        }
        pairs
    }

    #[test]
    fn the_sweep_counts_the_pairs_a_scan_finds() {
        // A deterministic scatter of boxes, some touching, some repeated.
        let mut state = 0x2545_f491_u64;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            #[allow(clippy::cast_precision_loss)]
            let unit = (state >> 40) as f64 / (1_u64 << 24) as f64;
            (unit * 8.0).round() / 2.0
        };
        let mut boxes = Vec::new();
        for _ in 0..300 {
            let (x, y, w, h) = (next(), next(), next() / 4.0, next() / 4.0);
            boxes.push([x, x + w, y, y + h]);
        }
        boxes.push(boxes[0]);
        assert_eq!(meeting_pairs(&boxes), brute(&boxes));
        // Stacked rows meeting in x but not in y are no pairs.
        let rows: Vec<EdgeBox> = (0..50)
            .map(|row| [0.0, 10.0, f64::from(row), f64::from(row) + 0.5])
            .collect();
        assert_eq!(meeting_pairs(&rows), 0);
        assert_eq!(meeting_pairs(&[]), 0);
    }

    /// The static reason names the budget the counted one does.
    #[test]
    fn both_reasons_name_the_budget() {
        let over = OverBudget {
            edges: 9,
            pairs: OVERLAY_PAIRS + 1,
        };
        assert!(
            OVER_BUDGET.contains(&format!("more than {OVERLAY_PAIRS} pairs")),
            "{OVER_BUDGET}"
        );
        assert!(
            over.to_string()
                .ends_with(&format!("past the overlay budget of {OVERLAY_PAIRS} pairs")),
            "{over}"
        );
    }

    /// Only an overlay past the budget is refused; a union counts its
    /// operand once.
    #[test]
    fn an_overlay_within_the_budget_is_admitted() {
        use axiolid_core::Point2;
        use axiolid_overlay::{Polygon, Ring};
        let square = Polygon {
            outer: Ring {
                points: vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                    Point2::new(1.0, 1.0),
                    Point2::new(0.0, 1.0),
                ],
            },
            holes: Vec::new(),
        };
        assert_eq!(admit(&[&[square.clone()], &[square]]), Ok(()));
        // 3,000 long, thin triangles along one line: each meets every other.
        let slivers: Vec<Polygon> = (0..3000)
            .map(|index| {
                let y = f64::from(index) * 1e-4;
                Polygon {
                    outer: Ring {
                        points: vec![
                            Point2::new(0.0, y),
                            Point2::new(10.0, y + 0.5),
                            Point2::new(0.0, y + 1.0),
                        ],
                    },
                    holes: Vec::new(),
                }
            })
            .collect();
        let refused = admit(&[&slivers]).unwrap_err();
        assert_eq!(refused.edges, 9000);
        assert!(refused.pairs > OVERLAY_PAIRS, "{refused:?}");
    }
}
