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

use std::cell::{Cell, RefCell};
use std::fmt;

use axiolid_core::Point2;
use axiolid_overlay::{Polygon, Ring};
use axioval_engine::memory::{self, Admission, OverBudget as OverMemory};

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

/// The refusal's reason where an error holds only a static text and a
/// disc morphology outside any search is past [`DISC_PAIRS`] or the
/// check's memory budget (#432).
pub(crate) const OVER_DISC_BUDGET: &str = "the disc morphology was not built: its edges meet in \
     more pairs than the disc morphology budget or its estimated memory is more than the \
     check's memory budget";

/// The refusal's reason where an error holds only a static text and the
/// overlay's memory estimate alone is more than the check's memory budget
/// (#407).
pub(crate) const OVER_MEMORY: &str =
    "the plan overlay was not built: its estimated memory is more than the check's memory budget";

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
/// Admitted, its memory estimate ([`overlay_bytes`]).
pub(crate) fn admit(operands: &[&[Polygon]]) -> Result<u64, OverBudget> {
    admit_boxes(&edge_boxes(operands))
}

/// [`admit`] for a soup of rings the overlay unites in one arrangement
/// (`axiolid_overlay::union_soup`): every ring's edges count (#432).
pub(crate) fn admit_rings(rings: &[Ring]) -> Result<u64, OverBudget> {
    admit_boxes(&ring_boxes(rings.iter()))
}

fn admit_boxes(boxes: &[EdgeBox]) -> Result<u64, OverBudget> {
    let pairs = meeting_pairs(boxes);
    if pairs > OVERLAY_PAIRS {
        return Err(OverBudget {
            edges: boxes.len(),
            pairs,
        });
    }
    Ok(overlay_bytes(boxes.len(), pairs))
}

/// One edge's closed box: `[x_min, x_max, y_min, y_max]`.
type EdgeBox = [f64; 4];

fn edge_boxes(operands: &[&[Polygon]]) -> Vec<EdgeBox> {
    ring_boxes(
        operands
            .iter()
            .flat_map(|polygons| polygons.iter())
            .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes)),
    )
}

fn ring_boxes<'r>(rings: impl Iterator<Item = &'r Ring>) -> Vec<EdgeBox> {
    let mut boxes = Vec::new();
    for ring in rings {
        let points = &ring.points;
        for (index, a) in points.iter().enumerate() {
            let b = points[(index + 1) % points.len()];
            boxes.push([a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y)]);
        }
    }
    boxes
}

/// What one arrangement is handed: its edges and the pairs of them whose
/// boxes meet, and the kind of operation that hands them over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cost {
    /// The edges handed over.
    pub(crate) edges: usize,
    /// The pairs of them whose boxes meet.
    pub(crate) pairs: u64,
    /// What builds the arrangement, which sets its budget.
    pub(crate) kind: Kind,
}

/// The operations a search hands the overlay, each with its own budget
/// per step: an arrangement keeps a different amount per counted pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A boolean overlay of two regions, within [`OVERLAY_PAIRS`].
    Overlay,
    /// A Minkowski sum or erosion by a polygon of a few corners (a
    /// rectangle, an octagon), within [`POLYGON_PAIRS`].
    Polygon,
    /// A disc morphology (a 64-gon), within [`DISC_PAIRS`].
    Disc,
}

impl Cost {
    /// The memory the step is estimated to take while it builds: the
    /// pairs at what its kind keeps per pair, and the edges handed over
    /// (#407). Admitted against the check's memory budget before the
    /// step is built ([`Work::charge`]).
    pub(crate) fn bytes(self) -> u64 {
        let per_pair = match self.kind {
            Kind::Overlay => OVERLAY_PAIR_BYTES,
            Kind::Polygon => POLYGON_PAIR_BYTES,
            Kind::Disc => DISC_PAIR_BYTES,
        };
        let edges = u64::try_from(self.edges).unwrap_or(u64::MAX);
        self.pairs
            .saturating_mul(per_pair)
            .saturating_add(edges.saturating_mul(EDGE_BYTES))
    }

    /// Nothing handed over.
    pub(crate) const NONE: Self = Self {
        edges: 0,
        pairs: 0,
        kind: Kind::Overlay,
    };

    /// The most pairs one step of this kind is handed.
    pub(crate) fn budget(self) -> u64 {
        match self.kind {
            Kind::Overlay => OVERLAY_PAIRS,
            Kind::Polygon => POLYGON_PAIRS,
            Kind::Disc => DISC_PAIRS,
        }
    }

    /// The step's share of a search's budget ([`SEARCH_STEPS`]), in
    /// millionths of a step at its kind's budget, rounded up.
    pub(crate) fn share(self) -> u64 {
        let share = (u128::from(self.pairs) * u128::from(STEP)).div_ceil(u128::from(self.budget()));
        u64::try_from(share).unwrap_or(u64::MAX)
    }

    fn named(self) -> &'static str {
        match self.kind {
            Kind::Overlay => "overlay",
            Kind::Polygon => "polygon morphology",
            Kind::Disc => "disc morphology",
        }
    }
}

/// What one overlay keeps per pair of meeting edges, about: a grown round
/// bar meeting in 2.4 million pairs took 0.4 GB, one in 10 million 1.9 GB
/// ([`OVERLAY_PAIRS`]).
const OVERLAY_PAIR_BYTES: u64 = 200;

/// What one Minkowski sum or erosion by a polygon keeps per pair, at
/// most: dense tessellated tables took about a gigabyte a million
/// ([`POLYGON_PAIRS`]), but the steps of a private sample's rectangle
/// search took up to 2.7 GB at 1.6 million pairs, about 1.8 KB a pair
/// (#407). The estimate stays above what was measured.
const POLYGON_PAIR_BYTES: u64 = 2048;

/// What one disc morphology keeps per counted pair, about: 0.1 GB at 1.0
/// million, 0.9 GB at 16 million ([`DISC_PAIRS`]).
const DISC_PAIR_BYTES: u64 = 100;

/// What an arrangement keeps per edge handed over, besides its pairs.
const EDGE_BYTES: u64 = 256;

/// The memory an overlay of `operands` is estimated to take
/// ([`Cost::bytes`]), counted as [`admit`] counts its pairs.
pub(crate) fn overlay_bytes(edges: usize, pairs: u64) -> u64 {
    Cost {
        edges,
        pairs,
        kind: Kind::Overlay,
    }
    .bytes()
}

/// The cost of an overlay of `operands` ([`admit`]'s count).
pub(crate) fn overlay_cost(operands: &[&[Polygon]]) -> Cost {
    let boxes = edge_boxes(operands);
    Cost {
        edges: boxes.len(),
        pairs: meeting_pairs(&boxes),
        kind: Kind::Overlay,
    }
}

/// The cost of a Minkowski sum of `polygons` with the convex polygon
/// `shape` (counter-clockwise, about the origin), or with `erosion` of the
/// erosion by it (#434).
///
/// The overlay builds both in one arrangement (axiolid-overlay 0.3.17,
/// `minkowski.rs`): the region's own rings, and for every boundary edge
/// from `a` to `b` the hull of the shape moved to `a` and to `b`, whose
/// edges are the shape's own, each moved to the end it faces, and two
/// edges as long as `b − a`. An erosion sums the complement within a
/// frame with the reflected shape, so it hands over the frame's edges,
/// the region's rings twice and the same hulls. Every hull of a shape a
/// metre and more across meets every hull within that reach, so along a
/// tessellated body of short edges the pairs grow with the square of its
/// corners. The count is of the hulls before the overlay merges
/// collinear corners and trims hulls along chords it proves inside the
/// set, so it bounds what the arrangement is handed from above: closely
/// for a rectangle along dense tessellated outlines, about ten times over
/// for a disc, whose hulls those chords trim the most.
pub(crate) fn minkowski_cost(polygons: &[Polygon], shape: &[Point2], erosion: bool) -> Cost {
    shaped_cost(polygons, shape, erosion, Kind::Polygon)
}

/// The cost of a disc morphology of `polygons` by a disc of `radius`
/// ([`minkowski_cost`] with the overlay's 64-gon), eroding with `erosion`.
pub(crate) fn disc_cost(polygons: &[Polygon], radius: f64, erosion: bool) -> Cost {
    shaped_cost(polygons, &disc(radius), erosion, Kind::Disc)
}

/// The cost of a disc morphology in the overlay's fast mode, whose
/// notches are filled by chords where that adds at most `budget` (#407):
/// [`disc_cost`] of the rings with every corner dropped that lies within
/// `budget` of the chord across it ([`thinned`]). A wall of teeth
/// shallower than the budget then counts as the few corners the fill
/// leaves of it, as the overlay builds it in time about its corners,
/// while every corner farther out still counts.
pub(crate) fn fast_disc_cost(
    polygons: &[Polygon],
    radius: f64,
    erosion: bool,
    budget: f64,
) -> Cost {
    let thinned: Vec<Polygon> = polygons
        .iter()
        .filter_map(|polygon| {
            Some(Polygon {
                outer: thinned(&polygon.outer.points, budget)?,
                holes: polygon
                    .holes
                    .iter()
                    .filter_map(|hole| thinned(&hole.points, budget))
                    .collect(),
            })
        })
        .collect();
    disc_cost(&thinned, radius, erosion)
}

/// `points`, a closed ring, with the corners dropped that lie within
/// `within` of the chord the corners kept span across them
/// (Douglas-Peucker from the ring's first corner and the corner farthest
/// from it); `None` where fewer than three are kept.
fn thinned(points: &[Point2], within: f64) -> Option<axiolid_overlay::Ring> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let distance = |a: Point2, b: Point2, p: Point2| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length = dx.hypot(dy);
        if length == 0.0 {
            return (p.x - a.x).hypot(p.y - a.y);
        }
        ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / length
    };
    let far = (1..n)
        .max_by(|&i, &j| {
            let d = |k: usize| (points[k].x - points[0].x).hypot(points[k].y - points[0].y);
            d(i).total_cmp(&d(j))
        })
        .unwrap_or(1);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    // Spans of the ring, as indices from `start` to `end` going round.
    let mut spans = vec![(0, far), (far, n)];
    while let Some((start, end)) = spans.pop() {
        let (a, b) = (points[start], points[end % n]);
        let mut most = (0.0_f64, 0);
        for (index, point) in points.iter().enumerate().take(end).skip(start + 1) {
            let d = distance(a, b, *point);
            if d > most.0 {
                most = (d, index);
            }
        }
        if most.0 > within {
            keep[most.1] = true;
            spans.push((start, most.1));
            spans.push((most.1, end));
        }
    }
    let kept: Vec<Point2> = points
        .iter()
        .zip(&keep)
        .filter(|(_, keep)| **keep)
        .map(|(point, _)| *point)
        .collect();
    (kept.len() >= 3).then_some(axiolid_overlay::Ring { points: kept })
}

fn shaped_cost(polygons: &[Polygon], shape: &[Point2], erosion: bool, kind: Kind) -> Cost {
    let rings: Vec<&[Point2]> = polygons
        .iter()
        .flat_map(|polygon| std::iter::once(&polygon.outer).chain(&polygon.holes))
        .map(|ring| ring.points.as_slice())
        .filter(|points| !points.is_empty())
        .collect();
    if rings.is_empty() || shape.len() < 3 {
        return Cost::NONE;
    }
    let shape: Vec<Point2> = if erosion {
        shape.iter().map(|p| Point2::new(-p.x, -p.y)).collect()
    } else {
        shape.to_vec()
    };
    let mut boxes = Vec::new();
    let own = |boxes: &mut Vec<EdgeBox>, points: &[Point2]| {
        for (index, a) in points.iter().enumerate() {
            let b = points[(index + 1) % points.len()];
            boxes.push([a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y)]);
        }
    };
    let mut swept: Vec<Vec<Point2>> = rings.iter().map(|ring| ring.to_vec()).collect();
    for ring in &rings {
        own(&mut boxes, ring);
    }
    if erosion {
        for ring in &rings {
            own(&mut boxes, ring);
        }
        let (mut low, mut high) = (rings[0][0], rings[0][0]);
        for point in rings.iter().copied().flatten() {
            low = Point2::new(low.x.min(point.x), low.y.min(point.y));
            high = Point2::new(high.x.max(point.x), high.y.max(point.y));
        }
        let reach = shape
            .iter()
            .fold(0.0_f64, |most, p| most.max(p.x.abs()).max(p.y.abs()));
        let pad = 2.0 * reach + (high.x - low.x).max(high.y - low.y) + 1.0;
        let frame = vec![
            Point2::new(low.x - pad, low.y - pad),
            Point2::new(high.x + pad, low.y - pad),
            Point2::new(high.x + pad, high.y + pad),
            Point2::new(low.x - pad, high.y + pad),
        ];
        own(&mut boxes, &frame);
        swept.push(frame);
    }
    for ring in &swept {
        for (index, &a) in ring.iter().enumerate() {
            let b = ring[(index + 1) % ring.len()];
            sweep_boxes(&mut boxes, &shape, a, b);
        }
    }
    Cost {
        edges: boxes.len(),
        pairs: meeting_pairs(&boxes),
        kind,
    }
}

/// The boxes of the edges of the hull of `shape` moved to `from` and to
/// `to`: each edge of the shape moved to the end its outward normal faces
/// (`to` where it faces along `to − from`), and the two edges joining the
/// copies where that changes.
fn sweep_boxes(boxes: &mut Vec<EdgeBox>, shape: &[Point2], from: Point2, to: Point2) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let corners = shape.len();
    let next = |index: usize| (index + 1) % corners;
    let faces = |index: usize| {
        let (start, end) = (shape[index], shape[next(index)]);
        // The outward normal of a counter-clockwise edge, against to − from.
        (end.y - start.y) * dx - (end.x - start.x) * dy > 0.0
    };
    let moved = |point: Point2, by: Point2| Point2::new(point.x + by.x, point.y + by.y);
    let segment = |start: Point2, end: Point2| {
        [
            start.x.min(end.x),
            start.x.max(end.x),
            start.y.min(end.y),
            start.y.max(end.y),
        ]
    };
    for index in 0..corners {
        let by = if faces(index) { to } else { from };
        let corner = shape[next(index)];
        boxes.push(segment(moved(shape[index], by), moved(corner, by)));
        if faces(index) != faces(next(index)) {
            boxes.push(segment(moved(corner, from), moved(corner, to)));
        }
    }
}

/// The regular polygon of [`DISC_SIDES`] sides around a disc of
/// `radius`, about the origin: the shape the overlay's disc morphology
/// sums with or erodes by, up to its margin, for [`minkowski_cost`].
fn disc(radius: f64) -> Vec<Point2> {
    let sides = f64::from(DISC_SIDES);
    let outer = radius / (std::f64::consts::PI / sides).cos();
    (0..DISC_SIDES)
        .map(|i| {
            let angle = std::f64::consts::TAU * f64::from(i) / sides;
            Point2::new(outer * angle.cos(), outer * angle.sin())
        })
        .collect()
}

/// The sides of the overlay's disc polygon (axiolid-overlay 0.3.17).
const DISC_SIDES: u32 = 64;

/// The most pairs of edges whose boxes meet that one Minkowski sum or
/// erosion by a polygon of a few corners is handed ([`minkowski_cost`]).
/// Its arrangement keeps far more per pair than an overlay's: dense
/// tessellated tables eroded by a 1.8 m rectangle at 0.56 million pairs
/// took 1.0 s and 0.4 GB, at 2.0 million 3.9 s and 1.9 GB, at 4.3
/// million 8.6 s and 4.2 GB, at 9.4 million 20 s and 9.7 GB (release):
/// about a gigabyte and two seconds a million.
pub(crate) const POLYGON_PAIRS: u64 = 2_000_000;

/// The most pairs one disc morphology is handed ([`disc_cost`]). The
/// count is about ten times what the arrangement builds (chords trim the
/// 64-gon's hulls along dense outlines): the same tables dilated by a
/// 0.6 m disc at 1.0 million pairs took 0.4 s and 0.1 GB, at 4.3
/// million 1.7 s and 0.4 GB, at 16 million 3.7 s and 0.9 GB.
pub(crate) const DISC_PAIRS: u64 = 16_000_000;

/// One step at its kind's budget, in the units a search's share is
/// counted in ([`Cost::share`]).
const STEP: u64 = 1_000_000;

/// How many steps at their kinds' budgets the overlays and Minkowski
/// operations of one placement search may spend together (#434). Each
/// such step takes some four seconds and up to two gigabytes; a search
/// spends a share of a step on each of its steps. An angle search tries
/// up to 512 intervals, each admitted step by step, so without this a
/// search whose every step passes may still run for hours. Its memory is
/// bounded by the steps alone, since each step's arrangement is dropped
/// before the next is built. The heaviest search of a private 157-space
/// house, decided after 83 s, spent 42 steps.
pub(crate) const SEARCH_STEPS: u64 = 100;

/// The work one search has handed the overlay so far, against each
/// kind's budget for every step ([`Cost::budget`]) and [`SEARCH_STEPS`]
/// for all of them.
///
/// Every step is charged whether or not a scene already holds its result,
/// and a region a scene keeps is charged once per search: what a search
/// may spend is a function of its request alone, never of the searches
/// before it.
#[derive(Debug, Default)]
pub(crate) struct Work {
    spent: Cell<u64>,
    charged: RefCell<Vec<u32>>,
    /// A smaller budget than [`SEARCH_STEPS`], in millionths of a step,
    /// for a test placing a search at its threshold.
    #[cfg(test)]
    limit: Option<u64>,
}

impl Work {
    /// A search's work within `limit` millionths of a step rather than
    /// [`SEARCH_STEPS`].
    #[cfg(test)]
    pub(crate) fn within(limit: u64) -> Self {
        Self {
            limit: Some(limit),
            ..Self::default()
        }
    }

    /// The most a search may spend, in millionths of a step.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn limit(&self) -> u64 {
        #[cfg(test)]
        if let Some(limit) = self.limit {
            return limit;
        }
        SEARCH_STEPS * STEP
    }

    /// Charges a step of `cost` that is about to be built, or refuses it
    /// past any budget, and admits it against the check's memory budget
    /// ([`axioval_engine::memory`]): the step waits there while the steps
    /// other threads build would pass it together. The step is built
    /// while the returned [`Step`] is held.
    pub(crate) fn charge(&self, what: &'static str, cost: Cost) -> Result<Step, Refusal> {
        self.charge_kept(what, cost)?;
        // The estimate alone within the budget was checked above, so the
        // admission only waits, never refuses.
        let admission =
            memory::admit(cost.bytes()).map_err(|over| Refusal::Memory { what, cost, over })?;
        Ok(Step {
            _admission: admission,
        })
    }

    /// Charges a step of `cost` whose result is kept from an earlier
    /// build, so nothing is built: refused exactly as building it would
    /// be, the memory budget included, but never waiting for memory.
    pub(crate) fn charge_kept(&self, what: &'static str, cost: Cost) -> Result<(), Refusal> {
        if cost.pairs > cost.budget() {
            return Err(Refusal::Step { what, cost });
        }
        memory::check(cost.bytes()).map_err(|over| Refusal::Memory { what, cost, over })?;
        let spent = self.spent.get().saturating_add(cost.share());
        if spent > self.limit() {
            return Err(Refusal::Search { what, cost });
        }
        self.spent.set(spent);
        Ok(())
    }

    /// Charges the step that builds a region a scene keeps, named by
    /// `key`, once per search: later asks are free and admitted at once.
    /// Where the scene holds the region already (`kept`), the step is
    /// charged as if built ([`Self::charge_kept`]) and never waits.
    pub(crate) fn charge_once(
        &self,
        key: u32,
        what: &'static str,
        cost: impl FnOnce() -> Cost,
        kept: bool,
    ) -> Result<Step, Refusal> {
        if self.charged.borrow().contains(&key) {
            return Ok(Step::NONE);
        }
        let step = if kept {
            self.charge_kept(what, cost())?;
            Step::NONE
        } else {
            self.charge(what, cost())?
        };
        self.charged.borrow_mut().push(key);
        Ok(step)
    }

    /// The share of [`SEARCH_STEPS`] spent so far, in millionths of a step.
    #[cfg(test)]
    pub(crate) fn spent(&self) -> u64 {
        self.spent.get()
    }
}

/// Admits one step outside any search (#432): refused past its kind's
/// budget ([`Cost::budget`]) or where its memory estimate alone is more
/// than the check's memory budget, and otherwise admitted against that
/// budget while the returned [`Step`] is held. The refusal is a function
/// of the step's operands alone, never of what was kept before it.
pub(crate) fn admit_step(what: &'static str, cost: Cost) -> Result<Step, Refusal> {
    if cost.pairs > cost.budget() {
        return Err(Refusal::Step { what, cost });
    }
    let admission =
        memory::admit(cost.bytes()).map_err(|over| Refusal::Memory { what, cost, over })?;
    Ok(Step {
        _admission: admission,
    })
}

/// A step admitted against the check's memory budget while it builds
/// ([`Work::charge`]); dropping it lets the steps waiting for memory run.
#[must_use = "a step is admitted only while it is held"]
#[derive(Debug)]
pub(crate) struct Step {
    /// Released when the step is dropped.
    _admission: Option<Admission>,
}

impl Step {
    /// A step that takes nothing: its result is kept.
    pub(crate) const NONE: Self = Self { _admission: None };
}

/// A step a search refused before handing it to the overlay (#434).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The step alone is past its kind's budget ([`Cost::budget`]).
    Step { what: &'static str, cost: Cost },
    /// The step would take the search past [`SEARCH_STEPS`].
    Search { what: &'static str, cost: Cost },
    /// The step's memory estimate alone is more than the check's memory
    /// budget (#407).
    Memory {
        what: &'static str,
        cost: Cost,
        over: OverMemory,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step { what, cost } => write!(
                f,
                "the {what} was not built: its {} edges meet in {} pairs, past the {} \
                 budget of {} pairs",
                cost.edges,
                cost.pairs,
                cost.named(),
                cost.budget()
            ),
            Self::Search { what, cost } => write!(
                f,
                "the {what} was not built: its {} pairs of meeting edges would take the \
                 search past its budget of {SEARCH_STEPS} steps at their budgets",
                cost.pairs
            ),
            Self::Memory { what, cost, over } => write!(
                f,
                "the {what} was not built: at its {} pairs of meeting edges {over}",
                cost.pairs
            ),
        }
    }
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
    use super::{
        Cost, EdgeBox, Kind, OVER_BUDGET, OVERLAY_PAIRS, OverBudget, POLYGON_PAIRS, Refusal,
        SEARCH_STEPS, Work, admit, meeting_pairs, minkowski_cost,
    };
    use axiolid_core::Point2;
    use axiolid_overlay::{Polygon, Ring};

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
        assert!(admit(&[&[square.clone()], &[square]]).is_ok());
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

    /// A rectangle swept along each edge of a square hands over the
    /// hull's six edges per edge besides the square's own four; an
    /// erosion adds the frame, its hulls and the square's rings again.
    #[test]
    fn a_sum_hands_over_one_hull_per_edge() {
        let shape = [
            Point2::new(-0.5, -0.25),
            Point2::new(0.5, -0.25),
            Point2::new(0.5, 0.25),
            Point2::new(-0.5, 0.25),
        ];
        let region = [square(0.0, 0.0, 4.0)];
        let sum = minkowski_cost(&region, &shape, false);
        assert_eq!(sum.edges, 4 + 4 * 6);
        let erosion = minkowski_cost(&region, &shape, true);
        assert_eq!(erosion.edges, 2 * 4 + 4 + 8 * 6);
        assert!(erosion.pairs > sum.pairs);
        assert_eq!(minkowski_cost(&[], &shape, false).pairs, 0);
    }

    /// A step past its kind's budget is refused alone; steps within it
    /// are charged until the search's budget is spent, and the step that
    /// would pass it is refused by name. A kept region is charged once.
    #[test]
    fn a_search_is_refused_past_its_steps() {
        let step = |pairs| Cost {
            edges: 10,
            pairs,
            kind: Kind::Polygon,
        };
        let work = Work::default();
        let alone = work.charge("sum", step(POLYGON_PAIRS + 1)).unwrap_err();
        assert!(matches!(alone, Refusal::Step { .. }));
        assert!(
            alone.to_string().ends_with(&format!(
                "past the polygon morphology budget of {POLYGON_PAIRS} pairs"
            )),
            "{alone}"
        );
        assert_eq!(work.spent(), 0);
        for _ in 0..SEARCH_STEPS {
            let _step = work.charge("sum", step(POLYGON_PAIRS)).unwrap();
        }
        let last = work.charge("sum", step(1)).unwrap_err();
        assert!(matches!(last, Refusal::Search { .. }));
        assert!(
            last.to_string().ends_with(&format!(
                "past its budget of {SEARCH_STEPS} steps at their budgets"
            )),
            "{last}"
        );
        let kept = Work::default();
        for _ in 0..3 {
            let _step = kept
                .charge_once(7, "kept", || step(POLYGON_PAIRS / 2), false)
                .unwrap();
        }
        assert_eq!(kept.spent(), 500_000);
        // A region the scene holds already is charged alike, once, and
        // refused alike: as if built, but never admitted.
        let held = Work::default();
        for _ in 0..3 {
            let _step = held
                .charge_once(7, "kept", || step(POLYGON_PAIRS / 2), true)
                .unwrap();
        }
        assert_eq!(held.spent(), 500_000);
        assert!(matches!(
            Work::default().charge_once(8, "kept", || step(POLYGON_PAIRS + 1), true),
            Err(Refusal::Step { .. })
        ));
    }

    /// A step whose memory estimate alone is more than the check's
    /// budget is refused by name, built or kept alike, whatever else is
    /// admitted; one within it is admitted while it is held (#407).
    #[test]
    fn a_step_past_the_memory_budget_is_refused_by_name() {
        use axioval_engine::MemoryBudget;
        use std::sync::Arc;
        let step = |pairs| Cost {
            edges: 10,
            pairs,
            kind: Kind::Polygon,
        };
        // Without a budget, nothing is refused for memory.
        let _unbudgeted = Work::default().charge("sum", step(POLYGON_PAIRS)).unwrap();
        let budget = Arc::new(MemoryBudget::new(step(1000).bytes()));
        let _entered = budget.enter();
        let work = Work::default();
        let held = work.charge("sum", step(1000)).unwrap();
        assert_eq!(budget.in_use(), step(1000).bytes());
        let refused = work.charge("sum", step(2000)).unwrap_err();
        assert!(matches!(refused, Refusal::Memory { .. }), "{refused:?}");
        assert!(
            refused.to_string().starts_with(
                "the sum was not built: at its 2000 pairs of meeting edges its estimated"
            ),
            "{refused}"
        );
        assert!(
            refused
                .to_string()
                .ends_with("more than the check's memory budget of 2.0 MiB"),
            "{refused}"
        );
        drop(held);
        assert_eq!(work.charge_kept("sum", step(2000)), Err(refused));
        assert_eq!(budget.in_use(), 0);
        // A refused step spends nothing of the search's budget.
        assert_eq!(work.spent(), step(1000).share());
    }

    /// Each kind's estimate grows with its pairs at what it keeps per
    /// pair, and the largest step any kind admits is estimated within
    /// the default budget's floor, so a default budget refuses no step.
    #[test]
    fn the_largest_step_fits_the_default_budget() {
        for (kind, pairs) in [
            (Kind::Overlay, OVERLAY_PAIRS),
            (Kind::Polygon, POLYGON_PAIRS),
            (Kind::Disc, super::DISC_PAIRS),
        ] {
            let largest = Cost {
                edges: 1_000_000,
                pairs,
                kind,
            };
            assert!(
                largest.bytes() < axioval_engine::MemoryBudget::FLOOR,
                "{largest:?}"
            );
        }
        assert_eq!(super::overlay_bytes(4, 10), 10 * 200 + 4 * 256);
    }
}
