//! Handrails along a flight or a ramp's run.
//!
//! A rail is measured in the frame of the stretch it runs along: positions
//! along its walking direction and across it, and heights above its pitch
//! line (the polyline through a flight's nosings, the line of a run's
//! surface along its steepest ascent).
//!
//! - A rail runs **along** the stretch when its body overlaps the pitch line
//!   along the direction, lies within the request's reach of the walking
//!   surface's sides across it and within its reach above the pitch line,
//!   and its top is not below the pitch line's lowest point.
//! - A rail must run **parallel** to the stretch: the upward-facing faces of
//!   its body must fill one rectangle along the direction in plan, as for a
//!   tread's width. A rail bending in plan, one with posts or brackets
//!   standing out of that rectangle, or one crossing the stretch at an angle
//!   is refused, never measured in part.
//! - The **top** of its body over a position along the direction is the
//!   highest point of its slice there. Between the positions of its corners
//!   (and of the pitch line's) that is the upper envelope of the lines its
//!   edges trace, so its least height above the pitch line lies at an end or
//!   where two of those lines cross, and its greatest at an end. Both are
//!   computed in floating point and widened by a numerical margin; a rail
//!   given as a tessellation of curved faces is widened further by its chord
//!   deviation, so a bound it straddles stays undecided.
//! - The **rise** over an extension is the difference between the highest
//!   and the lowest top over the stretch beyond the end, computed the same
//!   way; lines that do not rise or fall are evaluated without rounding, so a
//!   level extension of an exact rail rises by exactly zero.
//!
//! A **turning flight** is measured part by part: its parts are its runs of
//! consecutive treads sharing one frame square to their nosings (flight.rs
//! gives treads whose nosings are parallel within their uncertainty one
//! frame), and a rail near the flight must fill a rectangle along exactly one
//! part's direction, longer along it than across, within the reach of that
//! part's sides; otherwise the request refuses. Its pitch line runs along
//! the side it lies on: the ends of the nosings on that side lying on the
//! part's side line, bottom to top, each at its tread's elevation (ends at
//! one position, as winders meeting at a corner, merge into one point
//! spanning their elevations). Where the side turns away before or after
//! them, the pitch line's height over the rest of the part is known only to
//! lie between the last point and the next nosing's elevation, so a rail
//! there is measured against both; never beyond the flight. A rail reaching
//! over the middle of a turning flight, whose pitch line differs from side
//! to side, refuses.

use axiolid_core::Point3;
use axioval_engine::{
    ElevationInterval, HandrailEvidence, HandrailRequest, MeasuredInterval, MetricDirection,
    RailMeasurement, RailSide, StretchPart, Tread, TreadFlight, WalkingLine, WalkingStretch,
    WalkingSurfaceError, WalkingSurfaceService,
};
use axioval_ir::{Evidence, ObjectId};

use super::{
    AxiolidWalkingSurfaceService, PlanFrame, edge_counts, interval, plan_cross, rectangle,
};
use crate::flight::TreadFrame;
use crate::geometry::{Triangle, mesh_extent};

/// Relative numerical margin on a rail's heights and rises: a bound on the
/// rounding of interpolating its edges and the pitch line, scaled by the
/// coordinates' magnitude and the steepest line involved.
const RAIL_MARGIN: f64 = 64.0 * f64::EPSILON;

/// How far, relative to the coordinates' magnitude, a nosing's end may lie
/// off a part's side line and still be on it: the rounding of placements.
const SIDE_LINE: f64 = 1e-9;

/// The pitch line of a stretch (or one part of a turning flight, along one
/// side) and the walking surface's sides.
pub(super) struct Pitch {
    pub(super) direction: MetricDirection,
    /// Points of the line, `(position along, elevation)`, ascending along.
    points: Vec<(ElevationInterval, ElevationInterval)>,
    pub(super) sides: (ElevationInterval, ElevationInterval),
    /// The elevation of the nosing before the first point and after the
    /// last where the line goes on around a turn: over the rest of the part
    /// its height lies between that and the nearest point's.
    before: Option<ElevationInterval>,
    after: Option<ElevationInterval>,
    /// How far along the direction the flight reaches: a rail beside a turn
    /// is measured no farther.
    window: (f64, f64),
    /// Whether the line starts and ends where the stretch does, so a rail's
    /// rise beyond them is measured.
    ends: (bool, bool),
}

fn middle(value: ElevationInterval) -> f64 {
    f64::midpoint(value.lower_metres(), value.upper_metres())
}

fn half_width(value: ElevationInterval) -> f64 {
    (value.upper_metres() - value.lower_metres()) / 2.0
}

impl Pitch {
    /// A straight stretch's pitch line: its points only, the stretch's ends.
    pub(super) fn straight(
        direction: MetricDirection,
        points: Vec<(ElevationInterval, ElevationInterval)>,
        sides: (ElevationInterval, ElevationInterval),
    ) -> Self {
        Self {
            direction,
            points,
            sides,
            before: None,
            after: None,
            window: (f64::NEG_INFINITY, f64::INFINITY),
            ends: (true, true),
        }
    }

    /// The line through the middles of its points, dropping any that does
    /// not lie further along than the one before.
    pub(super) fn nominal(&self) -> Vec<(f64, f64)> {
        let mut line: Vec<(f64, f64)> = Vec::new();
        for (along, elevation) in &self.points {
            let point = (middle(*along), middle(*elevation));
            if line.last().is_none_or(|last| point.0 > last.0) {
                line.push(point);
            }
        }
        line
    }

    /// The steepest segment of the nominal line.
    pub(super) fn slope(line: &[(f64, f64)]) -> f64 {
        line.windows(2)
            .map(|pair| ((pair[1].1 - pair[0].1) / (pair[1].0 - pair[0].0)).abs())
            .fold(0.0, f64::max)
    }

    /// How far the nominal line may lie from the true one in height: each
    /// point off by half its intervals' widths, and the elevations beyond
    /// by half theirs.
    pub(super) fn error(&self, slope: f64) -> f64 {
        self.points
            .iter()
            .map(|(along, elevation)| half_width(*along) * slope + half_width(*elevation))
            .chain(
                self.before
                    .iter()
                    .chain(&self.after)
                    .map(|z| half_width(*z)),
            )
            .fold(0.0, f64::max)
    }

    /// The widest position interval along, which moves an extension's ends.
    pub(super) fn along_error(&self) -> f64 {
        self.points
            .iter()
            .map(|(along, _)| half_width(*along))
            .fold(0.0, f64::max)
    }
}

/// The nominal pitch line's elevation at `along`, held level beyond its
/// ends.
pub(super) fn pitch_at(line: &[(f64, f64)], along: f64) -> f64 {
    let Some(first) = line.first() else {
        return 0.0;
    };
    if along <= first.0 {
        return first.1;
    }
    for pair in line.windows(2) {
        let ((a, za), (b, zb)) = (pair[0], pair[1]);
        if along <= b {
            return za + (along - a) * (zb - za) / (b - a);
        }
    }
    line.last().map_or(first.1, |last| last.1)
}

/// A rail's edges in the stretch's frame: `[along, elevation]` at each end,
/// the lower position first.
struct Edges(Vec<[f64; 4]>);

impl Edges {
    fn new(faces: &[Triangle], frame: &PlanFrame) -> Self {
        let place = |point: &Point3| (frame.map(point.x, point.y)[0], point.z);
        let edges = edge_counts(faces)
            .into_values()
            .map(|(_, a, b)| {
                let (a, b) = (place(&a), place(&b));
                if a.0 <= b.0 {
                    [a.0, a.1, b.0, b.1]
                } else {
                    [b.0, b.1, a.0, a.1]
                }
            })
            .collect();
        Self(edges)
    }

    fn along(&self) -> (f64, f64) {
        self.0
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), edge| {
                (low.min(edge[0]), high.max(edge[2]))
            })
    }

    /// The top of the body's slice at `along`: the highest point of every
    /// edge reaching it. `None` where no edge does.
    fn top(&self, along: f64) -> Option<f64> {
        self.0
            .iter()
            .filter(|edge| edge[0] <= along && along <= edge[2])
            .map(|edge| {
                #[allow(clippy::float_cmp)]
                if edge[0] == edge[2] {
                    edge[1].max(edge[3])
                } else {
                    line(edge, along)
                }
            })
            .reduce(f64::max)
    }

    /// The least and greatest of `top(s) - under(s)` for `s` in `[low,
    /// high]`, with `under` linear between the `knots`, and the steepest
    /// line the top follows there. `None` where the body leaves a gap.
    fn range(
        &self,
        (low, high): (f64, f64),
        knots: &[f64],
        under: impl Fn(f64) -> f64,
    ) -> Option<(f64, f64, f64)> {
        let mut cuts: Vec<f64> = self
            .0
            .iter()
            .flat_map(|edge| [edge[0], edge[2]])
            .chain(knots.iter().copied())
            .filter(|cut| *cut > low && *cut < high)
            .chain([low, high])
            .collect();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let (mut least, mut most, mut steepest) = (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64);
        for cut in &cuts {
            let value = self.top(*cut)? - under(*cut);
            least = least.min(value);
            most = most.max(value);
        }
        for pair in cuts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            // Every edge spanning the piece traces one line over it.
            let lines: Vec<(f64, f64)> = self
                .0
                .iter()
                .filter(|edge| edge[0] <= a && edge[2] >= b && edge[0] < edge[2])
                .map(|edge| (slope(edge), line(edge, a)))
                .collect();
            if lines.is_empty() {
                return None;
            }
            let (under_a, under_b) = (under(a), under(b));
            let under_slope = (under_b - under_a) / (b - a);
            let envelope = |at: f64| {
                lines
                    .iter()
                    .map(|(k, start)| start + k * (at - a))
                    .fold(f64::NEG_INFINITY, f64::max)
                    - (under_a + under_slope * (at - a))
            };
            let mut candidates = vec![a, b];
            for (index, (k, start)) in lines.iter().enumerate() {
                steepest = steepest.max(k.abs());
                for (other_k, other_start) in &lines[index + 1..] {
                    #[allow(clippy::float_cmp)]
                    if k == other_k {
                        continue;
                    }
                    let at = a + (other_start - start) / (k - other_k);
                    if at > a && at < b {
                        candidates.push(at);
                    }
                }
            }
            for at in candidates {
                let value = envelope(at);
                least = least.min(value);
                most = most.max(value);
            }
        }
        Some((least, most, steepest))
    }
}

fn slope(edge: &[f64; 4]) -> f64 {
    (edge[3] - edge[1]) / (edge[2] - edge[0])
}

/// An edge's elevation at `along`, exactly its ends' where they agree.
fn line(edge: &[f64; 4], along: f64) -> f64 {
    #[allow(clippy::float_cmp)]
    if edge[1] == edge[3] {
        return edge[1];
    }
    edge[1] + (along - edge[0]) * slope(edge)
}

/// `(lower, upper)` widened by `margin`.
fn widened(
    value: ElevationInterval,
    margin: f64,
) -> Result<ElevationInterval, WalkingSurfaceError> {
    interval((value.lower_metres() - margin, value.upper_metres() + margin))
}

/// The positions of the walking surface's outermost sides over `sides`.
fn outermost(
    sides: &[(ElevationInterval, ElevationInterval)],
) -> Option<(ElevationInterval, ElevationInterval)> {
    let fold = |pick: fn(&(ElevationInterval, ElevationInterval)) -> ElevationInterval,
                outer: fn(f64, f64) -> f64| {
        sides.iter().map(pick).fold(None, |most, side| {
            Some(match most {
                None => (side.lower_metres(), side.upper_metres()),
                Some((low, high)) => (
                    outer(low, side.lower_metres()),
                    outer(high, side.upper_metres()),
                ),
            })
        })
    };
    let left = fold(|side| side.0, f64::min)?;
    let right = fold(|side| side.1, f64::max)?;
    Some((interval(left).ok()?, interval(right).ok()?))
}

/// One straight part of a turning flight: its treads share one frame.
struct Part {
    direction: MetricDirection,
    /// Its first and last tread, by index.
    treads: (usize, usize),
    sides: (ElevationInterval, ElevationInterval),
}

/// A turning flight's parts, bottom to top: its runs of consecutive treads
/// in one frame.
fn parts(frames: &[Option<TreadFrame>]) -> Result<Vec<Part>, WalkingSurfaceError> {
    let mut parts: Vec<(MetricDirection, usize, usize)> = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let Some(frame) = frame else { continue };
        match parts.last_mut() {
            Some((direction, _, last)) if *last + 1 == index && *direction == frame.direction => {
                *last = index;
            }
            _ => parts.push((frame.direction, index, index)),
        }
    }
    parts
        .into_iter()
        .map(|(direction, first, last)| {
            let sides: Vec<_> = frames[first..=last]
                .iter()
                .flatten()
                .map(|frame| frame.sides)
                .collect();
            Ok(Part {
                direction,
                treads: (first, last),
                sides: outermost(&sides).ok_or(WalkingSurfaceError::InvalidMeasurement)?,
            })
        })
        .collect()
}

/// A requested rail's body and the box enclosing it, `None` for a rail
/// without one.
type Located = Option<([f64; 3], [f64; 3])>;

/// The pitch line of a straight flight: through its nosings, and the upper
/// floor's edge where it ends in a riser.
pub(super) fn flight_pitch(
    subject: &ObjectId,
    flight: &TreadFlight,
) -> Result<Pitch, WalkingSurfaceError> {
    let WalkingLine::Straight(direction) = flight.walking_line() else {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    };
    let treads = flight.treads();
    let mut points: Vec<_> = treads
        .iter()
        .map(|tread| (tread.front(), tread.elevation()))
        .collect();
    if flight.ends_in_riser()
        && let Some(last) = treads.last()
    {
        // The upper floor's edge, where the last riser arrives.
        points.push((last.back(), flight.top()));
    }
    let sides: Option<Vec<_>> = treads.iter().map(Tread::sides).collect();
    let sides = sides
        .and_then(|sides| outermost(&sides))
        .ok_or_else(|| unmeasured(subject))?;
    Ok(Pitch::straight(*direction, points, sides))
}

fn unmeasured(subject: &ObjectId) -> WalkingSurfaceError {
    WalkingSurfaceError::Unsupported(format!(
        "the walking surface of {subject} fills no rectangle, so its sides are not measured"
    ))
}

impl AxiolidWalkingSurfaceService {
    /// The pitch line of a ramp's run: the line of its surface.
    pub(super) fn run_pitch(
        &self,
        subject: &ObjectId,
        index: usize,
    ) -> Result<Pitch, WalkingSurfaceError> {
        let ramp = self.measure_sloped_runs(subject)?;
        let run = ramp.runs().get(index).ok_or_else(|| {
            WalkingSurfaceError::Unsupported(format!(
                "{subject} has no run {} of {}",
                index + 1,
                ramp.runs().len()
            ))
        })?;
        Ok(Pitch::straight(
            run.direction(),
            vec![(run.start(), run.bottom()), (run.end(), run.top())],
            run.sides().ok_or_else(|| unmeasured(subject))?,
        ))
    }

    pub(super) fn handrails(
        &self,
        request: &HandrailRequest,
    ) -> Result<HandrailEvidence, WalkingSurfaceError> {
        let subject = request.subject();
        let pitch = match request.stretch() {
            WalkingStretch::Flight => {
                let (flight, frames) = self.framed_flight(subject)?;
                if flight.walking_line().is_turning() {
                    return self.turning_handrails(request, &flight, &frames);
                }
                flight_pitch(subject, &flight)?
            }
            WalkingStretch::Run(index) => self.run_pitch(subject, index)?,
        };
        let (Some(first), Some(last)) = (pitch.points.first(), pitch.points.last()) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let ends = (first.0, last.0);
        let stretch = Stretch::new(request, &pitch, None);
        let mut rails = Vec::new();
        for rail in request.rails() {
            if let Some(found) = self.rail_along(rail, &stretch)? {
                rails.push((rail.clone(), found));
            }
        }
        HandrailEvidence::try_new(
            request.clone(),
            pitch.direction,
            ends,
            pitch.sides,
            rails,
            evidence(subject),
        )
    }

    /// The handrails along a turning flight, part by part.
    fn turning_handrails(
        &self,
        request: &HandrailRequest,
        flight: &TreadFlight,
        frames: &[Option<TreadFrame>],
    ) -> Result<HandrailEvidence, WalkingSurfaceError> {
        let subject = request.subject();
        let parts = parts(frames)?;
        let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let (Some(first), Some(last)) = (first, last) else {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "{subject} is a turning flight starting or ending on a winder, which fills no \
                 rectangle, so its handrails have no end to reach beyond"
            )));
        };
        let ends = (
            first.along.0,
            if flight.ends_in_riser() {
                last.along.1
            } else {
                last.along.0
            },
        );
        let body = self.body(subject, true)?;
        let mut rails = Vec::new();
        for rail in request.rails() {
            let Some((part, side)) = self.assign(rail, request, flight, &parts, &body.soup)? else {
                continue;
            };
            let pitch = side_pitch(subject, flight, frames, &parts, part, side, &body.soup)?;
            let stretch = Stretch::new(request, &pitch, Some(flight));
            if let Some(found) = self.rail_along(rail, &stretch)? {
                rails.push((rail.clone(), found.in_part(part)));
            }
        }
        let parts = parts
            .iter()
            .map(|part| StretchPart::try_new(part.direction, part.sides))
            .collect::<Result<Vec<_>, _>>()?;
        HandrailEvidence::try_in_parts(request.clone(), parts, ends, rails, evidence(subject))
    }

    /// The part of a turning flight a requested rail runs along and the
    /// side it runs on, `None` for a rail without a body or away from the
    /// flight. A rail near the flight must fill a rectangle along exactly
    /// one part's direction, longer along it than across, within the reach
    /// of that part's sides and on one side of its middle.
    fn assign(
        &self,
        rail: &ObjectId,
        request: &HandrailRequest,
        flight: &TreadFlight,
        parts: &[Part],
        soup: &[Triangle],
    ) -> Result<Option<(usize, RailSide)>, WalkingSurfaceError> {
        let subject = request.subject();
        let Some((min, max)) = self.locate(rail)? else {
            return Ok(None);
        };
        let (low, high) = (
            flight
                .treads()
                .first()
                .map_or(0.0, |tread| tread.elevation().lower_metres()),
            flight.top().upper_metres(),
        );
        if max[2] < low || min[2] > high + request.above() {
            return Ok(None);
        }
        let scale = min
            .iter()
            .chain(max.iter())
            .fold(1.0_f64, |scale, value| scale.max(value.abs()));
        let slack = 1e-9 * scale;
        let corners = [
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ];
        let mut candidates = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let frame = PlanFrame::new(part.direction);
            let mapped = corners.map(|[x, y]| frame.map(x, y));
            let reach = (
                part.sides.0.lower_metres() - request.reach(),
                part.sides.1.upper_metres() + request.reach(),
            );
            let window = extent(soup, &frame);
            let apart = [window, reach]
                .iter()
                .enumerate()
                .any(|(axis, (from, to))| {
                    mapped.iter().all(|corner| corner[axis] < from - slack)
                        || mapped.iter().all(|corner| corner[axis] > to + slack)
                });
            if !apart {
                candidates.push((index, frame));
            }
        }
        if candidates.is_empty() {
            // A rail beside the flight's walking surface but along none of
            // its parts (beside a winder zone reaching out past them) might
            // be a piece of its handrail: refused, never dropped.
            if beside(soup, (min, max), request.reach() + slack) {
                return Err(WalkingSurfaceError::Unsupported(format!(
                    "rail {rail} lies beside the turning flight {subject} but along none of its \
                     straight parts"
                )));
            }
            return Ok(None);
        }
        let body = self.body(rail, true)?;
        let upward: Vec<Triangle> = body
            .soup
            .iter()
            .filter(|triangle| {
                let (cross, bound) = plan_cross(triangle);
                cross > bound
            })
            .copied()
            .collect();
        let mut along = Vec::new();
        for (index, frame) in candidates {
            let Some([(start, end), (left, right)]) = rectangle(&upward, &frame, 0.0)? else {
                continue;
            };
            let length = end.lower_metres() - start.upper_metres();
            let width = right.upper_metres() - left.lower_metres();
            if length > width {
                along.push((index, (left, right)));
            }
        }
        let [(index, (left, right))] = along[..] else {
            let why = if along.is_empty() {
                "does not run straight along any straight part"
            } else {
                "runs along several straight parts at once"
            };
            return Err(WalkingSurfaceError::Unsupported(format!(
                "rail {rail} {why} of the turning flight {subject}"
            )));
        };
        let (side_left, side_right) = parts[index].sides;
        let middle_low = f64::midpoint(side_left.lower_metres(), side_right.lower_metres());
        let middle_high = f64::midpoint(side_left.upper_metres(), side_right.upper_metres());
        let side = if right.upper_metres() < middle_low.next_down() {
            RailSide::Right
        } else if left.lower_metres() > middle_high.next_up() {
            RailSide::Left
        } else {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "rail {rail} reaches over the middle of the turning flight {subject}, whose \
                 pitch line differs from side to side"
            )));
        };
        Ok(Some((index, side)))
    }

    /// The box enclosing a requested rail, `None` for one without a body.
    fn locate(&self, rail: &ObjectId) -> Result<Located, WalkingSurfaceError> {
        if self.geometry.has_no_body(rail) {
            return Ok(None);
        }
        if let Some((_, reason)) = self
            .geometry
            .unmeasured()
            .find(|(unmeasured, _)| *unmeasured == rail)
        {
            return Err(WalkingSurfaceError::Unavailable(format!(
                "rail {rail} has a body that was not measured: {reason}"
            )));
        }
        let mesh = self
            .geometry
            .mesh(rail)
            .ok_or_else(|| WalkingSurfaceError::UnknownObject(rail.clone()))?;
        match self.geometry.enclosing_extent(rail) {
            Some(extent) => Ok(Some(extent)),
            None if mesh_extent(mesh).is_none() => Ok(None),
            None => Err(WalkingSurfaceError::Unavailable(format!(
                "rail {rail} has an invalid chord deviation"
            ))),
        }
    }

    /// One requested rail, `None` where it has no body or does not run
    /// along the stretch.
    fn rail_along(
        &self,
        rail: &ObjectId,
        stretch: &Stretch<'_>,
    ) -> Result<Option<RailMeasurement>, WalkingSurfaceError> {
        let Some((min, max)) = self.locate(rail)? else {
            return Ok(None);
        };
        let invalid = || {
            WalkingSurfaceError::Unavailable(format!("rail {rail} has an invalid chord deviation"))
        };
        let strip = stretch.strip;
        if max[2] < stretch.elevations.0 || min[2] > stretch.elevations.1 + stretch.request.above()
        {
            return Ok(None);
        }
        let corners = [
            [min[0], min[1]],
            [max[0], min[1]],
            [max[0], max[1]],
            [min[0], max[1]],
        ]
        .map(|[x, y]| stretch.frame.map(x, y));
        let scale = min
            .iter()
            .chain(max.iter())
            .fold(1.0_f64, |scale, value| scale.max(value.abs()));
        let apart = (0..2).any(|axis| {
            let (from, to) = if axis == 0 { strip.0 } else { strip.1 };
            let slack = 1e-9 * scale;
            corners.iter().all(|corner| corner[axis] < from - slack)
                || corners.iter().all(|corner| corner[axis] > to + slack)
        });
        if apart {
            return Ok(None);
        }
        let deviation = self
            .geometry
            .fidelity(rail)
            .map_err(|_| invalid())?
            .deviation_metres();
        let Some(found) = self.rail(rail, stretch)? else {
            return Ok(None);
        };
        let (start, end) = (found.start, found.end);
        let (left, right) = found.across;
        let along = end.upper_metres() + deviation >= strip.0.0
            && start.lower_metres() - deviation <= strip.0.1;
        let beside = right.upper_metres() + deviation >= strip.1.0
            && left.lower_metres() - deviation <= strip.1.1;
        if !along || !beside {
            return Ok(None);
        }
        let pitch = stretch.pitch;
        let margin = RAIL_MARGIN * found.scale * (1.0 + found.steepest + stretch.slope)
            + pitch.error(stretch.slope)
            + deviation;
        let rise = |rise: Option<(f64, f64)>| -> Result<_, WalkingSurfaceError> {
            rise.map(|(value, steepest)| {
                // Level lines are evaluated exactly; a sloping one rounds,
                // and the extension's ends move with the pitch line's
                // positions.
                let rounding = if steepest > 0.0 {
                    2.0 * RAIL_MARGIN * found.scale * (1.0 + steepest)
                        + 2.0 * pitch.along_error() * steepest
                } else {
                    0.0
                };
                let spread = rounding + 2.0 * deviation;
                MeasuredInterval::try_new((value - spread).max(0.0), value + spread)
            })
            .transpose()
        };
        let measured = |(low, high): (f64, f64)| {
            MeasuredInterval::try_new(low - margin, high.max(low) + margin)
        };
        RailMeasurement::try_new(
            (widened(start, deviation)?, widened(end, deviation)?),
            (widened(left, deviation)?, widened(right, deviation)?),
            measured(found.lowest)?,
            measured(found.highest)?,
        )?
        .with_rises(rise(found.bottom_rise)?, rise(found.top_rise)?)
        .map(Some)
    }

    /// One rail measured along the pitch line, or `None` where it does not
    /// reach over it.
    fn rail(
        &self,
        rail: &ObjectId,
        stretch: &Stretch<'_>,
    ) -> Result<Option<RailFound>, WalkingSurfaceError> {
        let subject = stretch.request.subject();
        let (line, frame, pitch) = (&stretch.line, &stretch.frame, stretch.pitch);
        let body = self.body(rail, true)?;
        let upward: Vec<Triangle> = body
            .soup
            .iter()
            .filter(|triangle| {
                let (cross, bound) = plan_cross(triangle);
                cross > bound
            })
            .copied()
            .collect();
        let Some([(start, end), across]) = rectangle(&upward, frame, 0.0)? else {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "rail {rail} does not run straight along {subject}: its plan is no rectangle \
                 along the walking direction"
            )));
        };
        let edges = Edges::new(&body.soup, frame);
        let (first, last) = (line.first(), line.last());
        let (Some(&first), Some(&last)) = (first, last) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let (from, to) = edges.along();
        // Beyond the line's first and last point the rail is measured only
        // where the line goes on around a turn, and never past the flight.
        let low = match pitch.before {
            Some(_) => from.max(pitch.window.0),
            None => from.max(first.0),
        };
        let high = match pitch.after {
            Some(_) => to.min(pitch.window.1),
            None => to.min(last.0),
        };
        if low > high {
            return Ok(None);
        }
        let Some((lowest, highest, steepest)) = heights(&edges, stretch, (low, high)) else {
            return Err(WalkingSurfaceError::Unsupported(format!(
                "rail {rail} leaves a gap along {subject}"
            )));
        };
        let scale = body.soup.iter().flatten().fold(
            line.iter()
                .fold(1.0_f64, |scale, (a, z)| scale.max(a.abs()).max(z.abs())),
            |scale, point| {
                scale
                    .max(point.x.abs())
                    .max(point.y.abs())
                    .max(point.z.abs())
            },
        );
        let extension = stretch.request.extension();
        // A rail stopping short of the stretch by no more than the rounding
        // of the positions reaches it.
        let rounding = RAIL_MARGIN * scale;
        let beyond = |span: (f64, f64)| {
            if extension <= 0.0 || span.0 < from - rounding || span.1 > to + rounding {
                return None;
            }
            edges
                .range((span.0.max(from), span.1.min(to)), &[], |_| 0.0)
                .map(|(low, high, steepest)| (high - low, steepest))
        };
        Ok(Some(RailFound {
            start,
            end,
            across,
            lowest,
            highest,
            steepest,
            scale,
            bottom_rise: pitch
                .ends
                .0
                .then(|| beyond((first.0 - extension, first.0)))
                .flatten(),
            top_rise: pitch
                .ends
                .1
                .then(|| beyond((last.0, last.0 + extension)))
                .flatten(),
        }))
    }
}

/// What a rail's heights are measured above over one stretch of it: the
/// pitch line, or anything between two elevations beside a turn.
#[derive(Clone, Copy)]
enum Under {
    Line,
    Between(f64, f64),
}

/// The least and the greatest height of a rail's top over `[low, high]`
/// along the stretch, each as `(above the highest pitch line it may have,
/// above the lowest)`, and the steepest line its top follows; `None` where
/// the rail leaves a gap. Between the pitch line's first and last point the
/// line is known; before and after them, beside a turn, it lies anywhere
/// between the neighbouring elevations.
#[allow(clippy::type_complexity)]
fn heights(
    edges: &Edges,
    stretch: &Stretch<'_>,
    (low, high): (f64, f64),
) -> Option<((f64, f64), (f64, f64), f64)> {
    let (line, pitch) = (&stretch.line, stretch.pitch);
    let (&first, &last) = (line.first()?, line.last()?);
    let mut spans: Vec<((f64, f64), Under)> = Vec::new();
    if let Some(before) = pitch.before
        && low < first.0
    {
        spans.push((
            (low, high.min(first.0)),
            Under::Between(middle(before), first.1),
        ));
    }
    if low.max(first.0) <= high.min(last.0) {
        spans.push(((low.max(first.0), high.min(last.0)), Under::Line));
    }
    if let Some(after) = pitch.after
        && high > last.0
    {
        spans.push((
            (low.max(last.0), high),
            Under::Between(last.1, middle(after)),
        ));
    }
    let (mut lowest, mut highest) = (
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    );
    let mut steepest = 0.0_f64;
    for (span, under) in spans {
        let (below, above) = match under {
            Under::Line => {
                let found = edges.range(span, &stretch.knots, |along| pitch_at(line, along))?;
                (found, found)
            }
            Under::Between(lower, upper) => (
                edges.range(span, &[], |_| lower)?,
                edges.range(span, &[], |_| upper)?,
            ),
        };
        // Above the highest pitch line the rail stands least high; above
        // the lowest, most.
        lowest = (lowest.0.min(above.0), lowest.1.min(below.0));
        highest = (highest.0.max(above.1), highest.1.max(below.1));
        steepest = steepest.max(below.2).max(above.2);
    }
    lowest.0.is_finite().then_some((lowest, highest, steepest))
}

fn evidence(subject: &ObjectId) -> Evidence {
    Evidence {
        source: subject.source.clone(),
        locator: format!("handrails:{subject}"),
        exact: false,
    }
}

/// Whether the plan box from `min` to `max` comes within `grown` of the plan
/// box of `soup`.
fn beside(soup: &[Triangle], (min, max): ([f64; 3], [f64; 3]), grown: f64) -> bool {
    let (near, far) = soup.iter().flatten().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(near, far), point| {
            (
                [near[0].min(point.x), near[1].min(point.y)],
                [far[0].max(point.x), far[1].max(point.y)],
            )
        },
    );
    (0..2).all(|axis| max[axis] >= near[axis] - grown && min[axis] <= far[axis] + grown)
}

/// The least and greatest position of `soup` along `frame`'s direction,
/// widened by the rounding of the projection.
fn extent(soup: &[Triangle], frame: &PlanFrame) -> (f64, f64) {
    soup.iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
            let along = frame.map(point.x, point.y)[0];
            let rounding = frame.rounding(point.x, point.y);
            (low.min(along - rounding), high.max(along + rounding))
        })
}

/// The pitch line of part `index` of a turning flight along `side`: the
/// nosings' ends on that side lying on the part's side line, from the part's
/// treads on as far as they go, each at its tread's elevation.
#[allow(clippy::too_many_lines)]
fn side_pitch(
    subject: &ObjectId,
    flight: &TreadFlight,
    frames: &[Option<TreadFrame>],
    parts: &[Part],
    index: usize,
    side: RailSide,
    soup: &[Triangle],
) -> Result<Pitch, WalkingSurfaceError> {
    let part = &parts[index];
    let frame = PlanFrame::new(part.direction);
    let treads = flight.treads();
    let count = treads.len();
    let WalkingLine::Turning(vertices) = flight.walking_line() else {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    };
    let line = match side {
        RailSide::Left => part.sides.1,
        RailSide::Right => part.sides.0,
    };
    let scale = vertices
        .iter()
        .flatten()
        .fold(1.0_f64, |scale, value| scale.max(value.abs()));
    let unsupported = |why: &str| {
        WalkingSurfaceError::Unsupported(format!(
            "the {} side of the turning flight {subject} {why}",
            match side {
                RailSide::Left => "left",
                RailSide::Right => "right",
            }
        ))
    };
    // The end of tread `i`'s nosing on the side, its position along the
    // part and whether it lies on the side line.
    let end = |i: usize| -> Option<(ElevationInterval, bool)> {
        let nosing = treads.get(i)?.nosing()?;
        let (before, after) = (
            &vertices[i.saturating_sub(1)],
            &vertices[(i + 1).min(count - 1)],
        );
        let walking = [after[0] - before[0], after[1] - before[1]];
        let (from, to) = (nosing.from(), nosing.to());
        let turn = walking[0] * (to[1] - from[1]) - walking[1] * (to[0] - from[0]);
        // `to` lies left of `from` when the nosing turns left of the
        // walking direction.
        let point = match (side, turn > 0.0) {
            (RailSide::Left, true) | (RailSide::Right, false) => to,
            _ => from,
        };
        let [along, across] = frame.map(point[0], point[1]);
        let known = nosing.radius() + frame.rounding(point[0], point[1]);
        let slack = known + SIDE_LINE * scale;
        let on = across >= line.lower_metres() - slack && across <= line.upper_metres() + slack;
        Some((interval((along - known, along + known)).ok()?, on))
    };
    let on = |i: usize| end(i).is_some_and(|(_, on)| on);
    let (mut first, mut last) = part.treads;
    if !(first..=last).all(on) {
        return Err(unsupported(
            "has a nosing ending off its straight part's side",
        ));
    }
    while first > 0 && on(first - 1) {
        first -= 1;
    }
    while last + 1 < count && on(last + 1) {
        last += 1;
    }
    // Ends at one position (winders meeting at a corner) are one point
    // spanning their elevations.
    let mut points: Vec<(ElevationInterval, ElevationInterval)> = Vec::new();
    for (i, tread) in treads.iter().enumerate().take(last + 1).skip(first) {
        let Some((along, _)) = end(i) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let elevation = tread.elevation();
        match points.last_mut() {
            Some((previous, height)) if along.lower_metres() <= previous.upper_metres() => {
                *previous = interval((
                    previous.lower_metres().min(along.lower_metres()),
                    previous.upper_metres().max(along.upper_metres()),
                ))?;
                *height = interval((
                    height.lower_metres().min(elevation.lower_metres()),
                    height.upper_metres().max(elevation.upper_metres()),
                ))?;
            }
            Some((previous, _)) if along.lower_metres() < previous.lower_metres() => {
                return Err(unsupported("turns back along its straight part"));
            }
            _ => points.push((along, elevation)),
        }
    }
    let mut after = (last + 1 < count).then(|| treads[last + 1].elevation());
    if last + 1 == count && flight.ends_in_riser() {
        // The upper floor's edge, where the last riser arrives: the last
        // tread's back edge, square to the part only in the part's frame.
        match frames[last] {
            Some(frame) if frame.direction == part.direction => {
                points.push((frame.along.1, flight.top()));
            }
            _ => after = Some(flight.top()),
        }
    }
    Ok(Pitch {
        direction: part.direction,
        points,
        sides: part.sides,
        before: (first > 0).then(|| treads[first - 1].elevation()),
        after,
        window: extent(soup, &frame),
        ends: (
            index == 0 && first == 0,
            index + 1 == parts.len() && last + 1 == count,
        ),
    })
}

/// What every rail along one stretch (or one part and side of a turning
/// flight) is measured against.
struct Stretch<'a> {
    request: &'a HandrailRequest,
    frame: PlanFrame,
    /// Positions along and across the direction a rail must reach into.
    strip: ((f64, f64), (f64, f64)),
    /// The lowest and highest elevation of the pitch line.
    elevations: (f64, f64),
    /// The nominal pitch line, its steepest slope and its knots.
    line: Vec<(f64, f64)>,
    slope: f64,
    knots: Vec<f64>,
    pitch: &'a Pitch,
}

impl<'a> Stretch<'a> {
    /// The stretch of `pitch`; along a turning `flight`, a rail may lie
    /// anywhere along the flight and between its lowest tread and its top.
    fn new(request: &'a HandrailRequest, pitch: &'a Pitch, flight: Option<&TreadFlight>) -> Self {
        let (ends, elevations) = match flight {
            None => {
                let first = pitch
                    .points
                    .first()
                    .map_or(0.0, |(along, _)| along.lower_metres());
                let last = pitch
                    .points
                    .last()
                    .map_or(0.0, |(along, _)| along.upper_metres());
                let elevations = pitch.points.iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(low, high), (_, elevation)| {
                        (
                            low.min(elevation.lower_metres()),
                            high.max(elevation.upper_metres()),
                        )
                    },
                );
                ((first, last), elevations)
            }
            Some(flight) => (
                pitch.window,
                (
                    flight
                        .treads()
                        .first()
                        .map_or(0.0, |tread| tread.elevation().lower_metres()),
                    flight.top().upper_metres(),
                ),
            ),
        };
        let line = pitch.nominal();
        Self {
            request,
            frame: PlanFrame::new(pitch.direction),
            strip: (
                ends,
                (
                    pitch.sides.0.lower_metres() - request.reach(),
                    pitch.sides.1.upper_metres() + request.reach(),
                ),
            ),
            elevations,
            slope: Pitch::slope(&line),
            knots: line.iter().map(|point| point.0).collect(),
            line,
            pitch,
        }
    }
}

/// A rail's measurement before its margins.
struct RailFound {
    start: ElevationInterval,
    end: ElevationInterval,
    across: (ElevationInterval, ElevationInterval),
    /// The least height of its top, above the highest and the lowest the
    /// pitch line may be.
    lowest: (f64, f64),
    /// The greatest, likewise.
    highest: (f64, f64),
    steepest: f64,
    scale: f64,
    bottom_rise: Option<(f64, f64)>,
    top_rise: Option<(f64, f64)>,
}
