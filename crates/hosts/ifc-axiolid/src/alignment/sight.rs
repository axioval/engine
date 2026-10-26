//! Available sight distances along IFC alignments: the sight-distance
//! method of the bridge's
//! [`AlignmentService`](axioval_engine::AlignmentService).
//!
//! The eye stands in the section at the request's station, the target in
//! the section at plan distance `d` further along ([`super::section`]'s
//! frame: horizontal left normal, `+Z` up). Targets are looked at from
//! `d = 0` on in steps of at most the request's `step`; a step `[a, b]` is
//! **clear** when two bounds prove every target in it in view, and a
//! single target is **blocked** when the sight line to it is proven to
//! meet an obstruction. A step not proven clear is halved: a blocked
//! target ends the search there, and halving between the last clear
//! target and the first blocked one narrows the interval to
//! [`TOLERANCE`]. A step that stays neither clear nor blocked when halved
//! to the tolerance ends the proof of clear targets there, and the first
//! blocked target sampled beyond it, or the range, is the upper bound.
//!
//! # The gradient line
//!
//! In the developed profile (plan distance against height) the sight line
//! to the target at `m` lies above the gradient line `z` by
//!
//! ```text
//! f(s) = (1 - t) h_e + t h_t + (1 - t) z(s0) + t z(s0 + m) - z(s),  t = (s - s0) / m
//! ```
//!
//! for `s` from the eye's `s0` to `s0 + m`. At a fixed share `t`, moving
//! the target by `d - m` changes it by at most `t |d - m| Δg`, where `Δg`
//! is the spread of the gradient over the whole stretch looked over. So
//! every target of a step with midpoint `m` and half-width `h` clears the
//! gradient line when `f(s) - t h Δg > 0` everywhere: that function's
//! slope over a piece lies within the line's slope less the piece's
//! certified gradient interval (the law's own bound, as the parameters
//! read it), so its least value over the piece is bounded where the two
//! slope lines from its ends meet. A piece whose bound does not prove it
//! positive is halved, at most [`PROFILE_DEPTH`] times; a sampled value
//! below zero by more than the evaluation's error proves the target at
//! `m` blocked by the gradient line.
//!
//! # Bodies
//!
//! Every target of the step lies within `h (B1 + |u| B2 / speed)` of the
//! one at its midpoint (the sweep's bound, [`super::section`]), plus the
//! frame's error, so every sight line of the step lies within that reach
//! of the one to the midpoint target. The adapter's
//! [`SightObstacles::near`] clears that reach of the obstacles' meshes, by
//! their deviation; [`SightObstacles::blocking`] proves a single target
//! hidden by exact obstacles with the kernel's `line_of_sight`. An
//! unmeasured obstacle refuses the measurement. The target is handed over
//! with the frame's error as its accuracy; the eye is taken as evaluated.
//!
//! Terrain is a body like any other here: it is read only where it is
//! selected among the obstacles.

use axiolid_curve::ElevationLaw;
use axiolid_evaluate::elevation_height;
use axioval_axiolid::SightObstacles;
use axioval_engine::{
    AlignmentError, AlignmentInterval, SightDistance, SightDistanceRequest, SightObstruction,
};
use axioval_ir::ObjectId;

use super::{Centreline, IfcAlignmentService, evidence, gradient, slack};

/// How close, in metres, the last clear target and the first blocked one
/// are brought together.
pub(super) const TOLERANCE: f64 = 1e-3;

/// The narrowest step, in metres, tried before a stretch neither proven
/// clear nor blocked ends the proof of clear targets.
pub(super) const NARROWEST: f64 = 1e-6;

/// Steps one measurement may test before it stops proving.
pub(super) const SIGHT_BUDGET: usize = 20_000;

/// How many times a piece of the gradient line may be halved.
pub(super) const PROFILE_DEPTH: u32 = 24;

/// What a test of the gradient line found.
#[derive(Debug, PartialEq)]
enum Gap {
    /// The sight lines stay above it.
    Clear,
    /// A sampled point of it lies above the sight line.
    Below,
    /// Neither could be proven.
    Open,
}

/// What a search along the alignment proved.
struct Search {
    /// Every target up to here is in view.
    clear: f64,
    /// The first target proven blocked and what blocks it.
    blocked: Option<(f64, Vec<SightObstruction>)>,
    /// Why the proof of clear targets ended before the blocked one.
    stalled: Option<String>,
}

/// One measurement's fixed parts.
struct Look<'a> {
    centreline: &'a Centreline,
    law: &'a ElevationLaw,
    /// The eye's plan distance.
    start: f64,
    /// The gradient line's height there.
    ground: f64,
    eye: [f64; 3],
    eye_height: f64,
    target: [f64; 2],
    /// The farther of the eye and the target from the centreline in the
    /// section, which scales the frame's error.
    section: f64,
    obstacles: SightObstacles<'a>,
}

/// The least value over a piece of width `width` of a function whose ends
/// are at least `start` and `end` and whose slope lies in `[low, high]`:
/// where the two slope lines from the ends meet.
fn least(start: f64, end: f64, width: f64, (low, high): (f64, f64)) -> f64 {
    if width <= 0.0 {
        return start.min(end);
    }
    if low >= 0.0 {
        return start;
    }
    if high <= 0.0 {
        return end;
    }
    let meet = ((end - high * width - start) / (low - high)).clamp(0.0, width);
    let value = (start + low * meet).max(end - high * (width - meet));
    value - 4.0 * f64::EPSILON * (start.abs() + end.abs() + value.abs())
}

impl Look<'_> {
    /// The gradient line's height at plan distance `at`, and its error.
    fn height(&self, at: f64) -> Result<(f64, f64), String> {
        let height = elevation_height(self.law, at).map_err(|error| {
            format!("the gradient line cannot be evaluated at {at} m ({error})")
        })?;
        Ok((height, slack(self.centreline.length.max(height.abs()))))
    }

    /// Every gradient over the plan distances `[a, b]`.
    fn grades(&self, a: f64, b: f64) -> Result<(f64, f64), String> {
        gradient(self.law, (a, b))
    }

    /// The point of the section at plan distance `at`, `(lateral, up)`.
    fn place(&self, at: f64, section: [f64; 2]) -> Result<[f64; 3], String> {
        Ok(self.centreline.frame_at(at)?.place(section))
    }

    /// How the sight lines to the targets within `shift / Δg` of the one at
    /// `middle` ahead stand to the gradient line: `shift` is `h Δg`.
    fn gap(&self, middle: f64, shift: f64) -> Result<Gap, String> {
        let end = self.start + middle;
        let (far, far_error) = self.height(end)?;
        let rise = far + self.target_height() - (self.ground + self.eye_height);
        let slope = (rise - shift) / middle;
        let error = 4.0 * far_error;
        let value = |at: f64, height: f64| {
            self.ground + self.eye_height + slope * (at - self.start) - height
        };
        let mut open = false;
        let pieces = (middle / self.centreline.length.max(1.0) * 64.0)
            .ceil()
            .max(4.0);
        let width = middle / pieces;
        let mut stack: Vec<(f64, f64, f64, f64, u32)> = Vec::new();
        let mut from = self.start;
        let mut from_value = self.eye_height;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for index in 1..=(pieces as usize) {
            #[allow(clippy::cast_precision_loss)]
            let to = if index == pieces as usize {
                end
            } else {
                self.start + width * index as f64
            };
            let to_value = if index == pieces as usize {
                self.target_height() - shift
            } else {
                value(to, self.height(to)?.0)
            };
            stack.push((from, from_value, to, to_value, 0));
            from = to;
            from_value = to_value;
        }
        while let Some((a, fa, b, fb, depth)) = stack.pop() {
            if fa.min(fb) < -error {
                return Ok(Gap::Below);
            }
            let (low, high) = self.grades(a, b)?;
            let bound = least(fa - error, fb - error, b - a, (slope - high, slope - low));
            if bound > 0.0 {
                continue;
            }
            // An end within the evaluation's error of the sight line can
            // never be proven below it by halving.
            if depth >= PROFILE_DEPTH || fa.min(fb) <= error {
                open = true;
                continue;
            }
            let mid = f64::midpoint(a, b);
            let mid_value = value(mid, self.height(mid)?.0);
            stack.push((a, fa, mid, mid_value, depth + 1));
            stack.push((mid, mid_value, b, fb, depth + 1));
        }
        Ok(if open { Gap::Open } else { Gap::Clear })
    }

    /// The targets proven in view and the first proven blocked, looking
    /// at most `limit` metres ahead in steps of at most `step`.
    fn search(&self, limit: f64, step: f64) -> Result<Search, String> {
        let (mut clear, mut width) = (0.0_f64, step.min(limit));
        let mut blocked: Option<(f64, Vec<SightObstruction>)> = None;
        let mut stalled: Option<String> = None;
        let mut spent = 0;
        let budget = |spent: usize, clear: f64| {
            (spent > SIGHT_BUDGET).then(|| {
                format!("the search stopped after {SIGHT_BUDGET} steps at {clear} m ahead")
            })
        };
        'search: loop {
            // Forward by steps until a target is proven blocked, then by
            // halving between the last clear target and it.
            let end = match &blocked {
                Some((at, _)) if at - clear <= TOLERANCE => break,
                Some((at, _)) => 0.5 * (clear + at),
                None if limit - clear <= TOLERANCE => break,
                None => (clear + width).min(limit),
            };
            spent += 1;
            if let Some(reason) = budget(spent, clear) {
                stalled = Some(reason);
                break;
            }
            let Err(reason) = self.clear(clear, end)? else {
                clear = end;
                width = (2.0 * width).min(step);
                continue;
            };
            let found = self.blocked(end)?;
            if !found.is_empty() {
                blocked = Some((end, found));
                width = 0.5 * (end - clear);
                continue;
            }
            // Neither: narrower steps from the last clear target.
            let mut narrow = 0.5 * (end - clear);
            loop {
                spent += 1;
                if let Some(reason) = budget(spent, clear) {
                    stalled = Some(reason);
                    break 'search;
                }
                if narrow < NARROWEST {
                    stalled = Some(reason);
                    break 'search;
                }
                if self.clear(clear, clear + narrow)?.is_ok() {
                    clear += narrow;
                    break;
                }
                narrow *= 0.5;
            }
            width = narrow;
        }
        // Past a stretch neither proven clear nor blocked, the first target
        // sampled beyond it that is proven blocked bounds the distance.
        if stalled.is_some() && blocked.is_none() {
            let mut at = clear + step;
            while at <= limit && spent <= SIGHT_BUDGET {
                spent += 1;
                let found = self.blocked(at)?;
                if !found.is_empty() {
                    blocked = Some((at, found));
                    break;
                }
                at += step;
            }
        }
        // Bring the first blocked target down towards the last clear one by
        // halving: a target not proven blocked only moves the search, never
        // the bound.
        if let Some((mut at, mut found)) = blocked.take() {
            let mut below = clear;
            while at - below > TOLERANCE && spent <= SIGHT_BUDGET {
                spent += 1;
                let middle = f64::midpoint(below, at);
                let blocking = self.blocked(middle)?;
                if blocking.is_empty() {
                    below = middle;
                } else {
                    (at, found) = (middle, blocking);
                }
            }
            blocked = Some((at, found));
        }
        Ok(Search {
            clear,
            blocked,
            stalled,
        })
    }

    fn target_height(&self) -> f64 {
        self.target[1]
    }

    /// Whether every target from `a` to `b` ahead is proven in view; the
    /// reason where not.
    fn clear(&self, a: f64, b: f64) -> Result<Result<(), String>, String> {
        let (middle, half) = (f64::midpoint(a, b), 0.5 * (b - a));
        let (low, high) = self.grades(self.start, self.start + b)?;
        let shift = half * (high - low);
        if middle <= 0.0 || self.gap(middle, shift)? != Gap::Clear {
            return Ok(Err(format!(
                "the sight lines to the targets from {a} to {b} m ahead come near the \
                 gradient line"
            )));
        }
        let Some(reach) = self.reach(a, b)? else {
            return Ok(Err(format!(
                "between {a} and {b} m ahead the turn of the section cannot be bounded"
            )));
        };
        let target = self.place(self.start + middle, self.target)?;
        let near = self.obstacles.near(self.eye, target, reach);
        if near.is_empty() {
            return Ok(Ok(()));
        }
        Ok(Err(format!(
            "the sight lines to the targets from {a} to {b} m ahead come within {reach} m of {}",
            named(&near)
        )))
    }

    /// How far every target from `a` to `b` ahead lies at most from the one
    /// at its middle, with the frame's error; `None` where the turn of the
    /// section over the step is unbounded.
    fn reach(&self, a: f64, b: f64) -> Result<Option<f64>, String> {
        let (from, to) = (self.start + a, self.start + b);
        let half = 0.5 * (to - from);
        let frame = self.centreline.frame_at(f64::midpoint(from, to))?;
        let bounds = self.centreline.derivative_bounds(from, to)?;
        let speed = frame.speed - bounds.1 * half;
        if speed < 0.5 {
            return Ok(None);
        }
        let moved = half * (bounds.0 + self.target[0].abs() * bounds.1 / speed);
        let error = self.centreline.frame_error(self.section, bounds);
        Ok(Some((moved + error) * (1.0 + 1e-9)))
    }

    /// What is proven to block the sight line to the target `d` ahead.
    fn blocked(&self, d: f64) -> Result<Vec<SightObstruction>, String> {
        let mut found = Vec::new();
        if d <= 0.0 {
            return Ok(found);
        }
        if self.gap(d, 0.0)? == Gap::Below {
            found.push(SightObstruction::GradientLine);
        }
        let at = self.start + d;
        let target = self.place(at, self.target)?;
        let bounds = self.centreline.derivative_bounds(at, at)?;
        let error = self.centreline.frame_error(self.section, bounds);
        if let Some(bodies) = self
            .obstacles
            .blocking(self.eye, target, error)
            .map_err(|error| error.to_string())?
        {
            found.extend(bodies.into_iter().map(SightObstruction::Body));
        }
        Ok(found)
    }
}

/// Up to five obstacles by name.
fn named(objects: &[&ObjectId]) -> String {
    let mut listed: Vec<String> = objects.iter().take(5).map(ToString::to_string).collect();
    if objects.len() > 5 {
        listed.push(format!("{} more", objects.len() - 5));
    }
    listed.join(", ")
}

impl IfcAlignmentService {
    pub(super) fn sight_distance(
        &self,
        request: &SightDistanceRequest,
    ) -> Result<SightDistance, AlignmentError> {
        let alignment = request.alignment();
        let centreline = self.centreline(alignment)?;
        let start = centreline.distance_of(request.station())?;
        let obstacles = SightObstacles::new(&self.geometry, request.obstacles())
            .map_err(|error| AlignmentError::Unavailable(error.to_string()))?;
        let unavailable = AlignmentError::Unavailable;
        let law = &centreline.elevated().elevation;
        let ground = elevation_height(law, start).map_err(|error| {
            unavailable(format!(
                "the gradient line cannot be evaluated at {start} m ({error})"
            ))
        })?;
        let eye = centreline
            .frame_at(start)
            .map_err(unavailable)?
            .place(request.eye());
        let look = Look {
            centreline,
            law,
            start,
            ground,
            eye,
            eye_height: request.eye()[1],
            target: request.target(),
            section: request.eye()[0]
                .hypot(request.eye()[1])
                .max(request.target()[0].hypot(request.target()[1])),
            obstacles,
        };
        let range = request.range();
        let limit = (centreline.length - start).min(range);
        let step = request.step();
        let Search {
            clear,
            blocked,
            stalled,
        } = look.search(limit, step).map_err(unavailable)?;
        let (upper, blocked_by) = blocked.unwrap_or((range, Vec::new()));
        let lower = clear.min(upper);
        let distance = AlignmentInterval::try_new(lower, upper)?;
        let mut locator = format!(
            "sight along {} from station {} ({start} m along it), eye at ({}, {}) and target \
             at ({}, {}) in the section, against the gradient line and {} obstacles \
             (terrain only as a selected body): clear to {lower} m ahead",
            centreline.locator,
            request.station(),
            request.eye()[0],
            request.eye()[1],
            request.target()[0],
            request.target()[1],
            request.obstacles().len(),
        );
        if let Some(reason) = &stalled {
            locator.push_str("; undecided beyond: ");
            locator.push_str(reason);
        }
        if limit < range && blocked_by.is_empty() {
            locator.push_str("; the alignment ends ");
            locator.push_str(&limit.to_string());
            locator.push_str(" m ahead, short of the range");
        }
        let exact = distance.is_exact();
        SightDistance::try_new(
            request.clone(),
            distance,
            blocked_by,
            evidence(&alignment.source, locator, exact),
        )
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use std::collections::BTreeMap;

    use axiolid_core::{Frame2, Point2, Point3, Vec2};
    use axiolid_curve::{CurvatureLaw, Curve2, Curve3, Elevated3, Intrinsic2};
    use axiolid_evaluate::bound::continuity_breaks3;
    use axiolid_mesh::TriMesh;
    use axioval_axiolid::AxiolidGeometry;
    use axioval_engine::AlignmentService;
    use axioval_ir::SourceId;

    use super::*;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    /// A centreline from the origin heading `+X` with plan curvature
    /// `curvature`, `length` long, whose gradient line is the polynomial
    /// `elevation` of plan distance.
    fn centreline(curvature: f64, length: f64, elevation: Vec<f64>) -> Centreline {
        let plan = Curve2::Intrinsic(Intrinsic2::new(
            Frame2 {
                origin: Point2::new(0.0, 0.0),
                x: Vec2::new(1.0, 0.0),
                y: Vec2::new(0.0, 1.0),
            },
            CurvatureLaw::Constant { curvature },
            length,
        ));
        let curve = Curve3::Elevated(Elevated3::new(
            plan,
            ElevationLaw::Polynomial {
                coefficients: elevation,
            },
        ));
        let breaks = continuity_breaks3(&curve, 2)
            .into_iter()
            .filter(|at| *at > 0.0 && *at < length)
            .collect();
        Centreline {
            curve,
            length,
            breaks,
            frame: None,
            stationing: Ok(None),
            cant: Ok(None),
            locator: "test".into(),
        }
    }

    /// A closed box from eight corners, bit 0 the first edge, bit 1 the
    /// second and bit 2 the third.
    fn hexahedron(corner: impl Fn(usize) -> [f64; 3]) -> TriMesh {
        let positions = (0..8)
            .map(|i| {
                let [x, y, z] = corner(i);
                Point3::new(x, y, z)
            })
            .collect();
        let quads: [[u32; 4]; 6] = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let indices = quads
            .iter()
            .flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d])
            .collect();
        TriMesh::new(positions, indices)
    }

    fn service(centreline: Centreline, bodies: Vec<(&str, TriMesh)>) -> IfcAlignmentService {
        let mut geometry = AxiolidGeometry::new();
        for (local, mesh) in bodies {
            geometry = geometry.with_mesh(id(local), mesh);
        }
        IfcAlignmentService {
            points: BTreeMap::new(),
            alignments: BTreeMap::from([(id("axis"), Ok(centreline))]),
            layouts: BTreeMap::new(),
            geometry,
        }
    }

    fn look(
        service: &IfcAlignmentService,
        station: f64,
        eye: [f64; 2],
        target: [f64; 2],
        range: f64,
        obstacles: &[&str],
    ) -> Result<SightDistance, AlignmentError> {
        let request = SightDistanceRequest::try_new(
            id("axis"),
            station,
            eye,
            target,
            (range, 10.0),
            obstacles.iter().map(|local| id(local)).collect(),
        )
        .unwrap();
        service.measure_sight_distance(&request)
    }

    /// On a parabolic crest of radius 3000 m, an eye 1 m and a target
    /// 0.15 m above the gradient line see each other over
    /// `sqrt(2 R) (sqrt(h_e) + sqrt(h_t))`, 107.46 m, while both stand on
    /// the crest.
    #[test]
    fn on_a_crest_the_sight_distance_is_the_closed_form() {
        let radius = 3000.0;
        let crest = centreline(0.0, 400.0, vec![50.0, 0.06, -0.5 / radius]);
        let service = service(crest, Vec::new());
        let expected = (2.0 * radius).sqrt() * (1.0_f64.sqrt() + 0.15_f64.sqrt());
        for station in [100.0, 180.0] {
            let sight = look(&service, station, [0.0, 1.0], [0.0, 0.15], 300.0, &[]).unwrap();
            let (lower, upper) = (sight.distance().lower(), sight.distance().upper());
            assert!(
                lower <= expected && expected <= upper && upper - lower <= 2.0 * TOLERANCE,
                "[{lower}, {upper}] must hold {expected}"
            );
            assert_eq!(sight.blocked_by(), [SightObstruction::GradientLine]);
            assert!(!sight.evidence().exact);
        }
        // Offsets do not change the developed profile.
        let sight = look(&service, 100.0, [1.75, 1.0], [-1.75, 0.15], 300.0, &[]).unwrap();
        assert!(sight.distance().lower() <= expected && expected <= sight.distance().upper());
    }

    /// On a level arc of radius 100 m turning left, a 6 m high wall on the
    /// inside, its face 4 m inside the centreline at 0.4 rad round and
    /// 20 m long, cuts the view from station 0 where the chord to the
    /// target passes its nearest corner: at twice the corner's angle from
    /// the tangent, times the radius. Without it, the view reaches the
    /// range; the wall's far side does not count.
    #[test]
    fn a_wall_inside_a_curve_shortens_the_sight_distance_to_its_corner() {
        let radius = 100.0;
        let arc = centreline(1.0 / radius, 300.0, vec![0.0]);
        let angle: f64 = 0.4;
        let normal = [angle.sin(), -angle.cos()];
        let tangent = [angle.cos(), angle.sin()];
        let corner = |i: usize| {
            let across = if i & 1 == 0 { 96.0 } else { 95.0 };
            let along = if i & 2 == 0 { -10.0 } else { 10.0 };
            let up = if i & 4 == 0 { -1.0 } else { 5.0 };
            [
                across * normal[0] + along * tangent[0],
                radius + across * normal[1] + along * tangent[1],
                up,
            ]
        };
        let expected = (0..8)
            .map(|i| {
                let [x, y, _] = corner(i);
                y.atan2(x)
            })
            .fold(f64::INFINITY, f64::min)
            * 2.0
            * radius;
        let wall = hexahedron(corner);
        let service = service(arc, vec![("wall", wall)]);
        let sight = look(&service, 0.0, [0.0, 1.0], [0.0, 1.0], 150.0, &["wall"]).unwrap();
        let (lower, upper) = (sight.distance().lower(), sight.distance().upper());
        assert!(
            lower <= expected && expected <= upper && upper - lower <= 0.05,
            "[{lower}, {upper}] must hold {expected}"
        );
        assert_eq!(sight.blocked_by(), [SightObstruction::Body(id("wall"))]);
        let clear = look(&service, 0.0, [0.0, 1.0], [0.0, 1.0], 150.0, &[]).unwrap();
        assert_eq!(
            (clear.distance().lower(), clear.distance().upper()),
            (150.0, 150.0)
        );
        assert!(clear.evidence().exact && clear.blocked_by().is_empty());
        // Looking from beyond it, the wall is behind the eye.
        let beyond = look(&service, 80.0, [0.0, 1.0], [0.0, 1.0], 100.0, &["wall"]).unwrap();
        assert_eq!(beyond.distance().lower(), 100.0);
    }

    /// The range past the alignment's end leaves the distance open up to
    /// the range; an unmeasured obstacle or an eye off the alignment
    /// refuses.
    #[test]
    fn an_alignment_ending_short_of_the_range_leaves_it_open() {
        let level = centreline(0.0, 100.0, vec![10.0]);
        let service = service(level, Vec::new());
        let sight = look(&service, 50.0, [0.0, 1.0], [0.0, 1.0], 80.0, &[]).unwrap();
        assert_eq!(
            (sight.distance().lower(), sight.distance().upper()),
            (50.0, 80.0)
        );
        assert!(
            sight.evidence().locator.contains("ends 50 m ahead"),
            "{}",
            sight.evidence().locator
        );
        assert!(matches!(
            look(&service, 150.0, [0.0, 1.0], [0.0, 1.0], 80.0, &[]),
            Err(AlignmentError::OffRange(_))
        ));
        let mut unmeasured = service;
        unmeasured.geometry = unmeasured.geometry.with_unmeasured(id("post"), "failed");
        let Err(AlignmentError::Unavailable(reason)) =
            look(&unmeasured, 0.0, [0.0, 1.0], [0.0, 1.0], 80.0, &["post"])
        else {
            panic!("an unmeasured obstacle must refuse");
        };
        assert!(reason.contains("not measured"), "{reason}");
    }
}
