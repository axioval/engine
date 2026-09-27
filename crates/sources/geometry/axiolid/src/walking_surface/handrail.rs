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

use axiolid_core::Point3;
use axioval_engine::{
    ElevationInterval, HandrailEvidence, HandrailRequest, MeasuredInterval, MetricDirection,
    RailMeasurement, Tread, TreadFlightRequest, WalkingStretch, WalkingSurfaceError,
    WalkingSurfaceService,
};
use axioval_ir::{Evidence, ObjectId};

use super::{
    AxiolidWalkingSurfaceService, PlanFrame, edge_counts, interval, plan_cross, rectangle,
    straight_direction,
};
use crate::geometry::{Triangle, mesh_extent};

/// Relative numerical margin on a rail's heights and rises: a bound on the
/// rounding of interpolating its edges and the pitch line, scaled by the
/// coordinates' magnitude and the steepest line involved.
const RAIL_MARGIN: f64 = 64.0 * f64::EPSILON;

/// The pitch line of a stretch and the walking surface's sides.
struct Pitch {
    direction: MetricDirection,
    /// Points of the line, `(position along, elevation)`, ascending along.
    points: Vec<(ElevationInterval, ElevationInterval)>,
    sides: (ElevationInterval, ElevationInterval),
}

fn middle(value: ElevationInterval) -> f64 {
    f64::midpoint(value.lower_metres(), value.upper_metres())
}

fn half_width(value: ElevationInterval) -> f64 {
    (value.upper_metres() - value.lower_metres()) / 2.0
}

impl Pitch {
    /// The line through the middles of its points, dropping any that does
    /// not lie further along than the one before.
    fn nominal(&self) -> Vec<(f64, f64)> {
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
    fn slope(line: &[(f64, f64)]) -> f64 {
        line.windows(2)
            .map(|pair| ((pair[1].1 - pair[0].1) / (pair[1].0 - pair[0].0)).abs())
            .fold(0.0, f64::max)
    }

    /// How far the nominal line may lie from the true one in height: each
    /// point off by half its intervals' widths.
    fn error(&self, slope: f64) -> f64 {
        self.points
            .iter()
            .map(|(along, elevation)| half_width(*along) * slope + half_width(*elevation))
            .fold(0.0, f64::max)
    }

    /// The widest position interval along, which moves an extension's ends.
    fn along_error(&self) -> f64 {
        self.points
            .iter()
            .map(|(along, _)| half_width(*along))
            .fold(0.0, f64::max)
    }
}

/// The nominal pitch line's elevation at `along`, held level beyond its
/// ends.
fn pitch_at(line: &[(f64, f64)], along: f64) -> f64 {
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

fn measured(value: f64, margin: f64) -> Result<MeasuredInterval, WalkingSurfaceError> {
    MeasuredInterval::try_new(value - margin, value + margin)
}

impl AxiolidWalkingSurfaceService {
    fn pitch(
        &self,
        subject: &ObjectId,
        stretch: WalkingStretch,
    ) -> Result<Pitch, WalkingSurfaceError> {
        let unmeasured = || {
            WalkingSurfaceError::Unsupported(format!(
                "the walking surface of {subject} fills no rectangle, so its sides are not \
                 measured"
            ))
        };
        match stretch {
            WalkingStretch::Flight => {
                let flight =
                    self.measure_tread_flight(&TreadFlightRequest::new(subject.clone()))?;
                let direction = straight_direction(&flight, "handrails")?;
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
                let sides = sides.ok_or_else(unmeasured)?;
                let fold =
                    |pick: fn(&(ElevationInterval, ElevationInterval)) -> ElevationInterval,
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
                let left = fold(|side| side.0, f64::min).ok_or_else(unmeasured)?;
                let right = fold(|side| side.1, f64::max).ok_or_else(unmeasured)?;
                Ok(Pitch {
                    direction,
                    points,
                    sides: (interval(left)?, interval(right)?),
                })
            }
            WalkingStretch::Run(index) => {
                let ramp = self.measure_sloped_runs(subject)?;
                let run = ramp.runs().get(index).ok_or_else(|| {
                    WalkingSurfaceError::Unsupported(format!(
                        "{subject} has no run {} of {}",
                        index + 1,
                        ramp.runs().len()
                    ))
                })?;
                Ok(Pitch {
                    direction: run.direction(),
                    points: vec![(run.start(), run.bottom()), (run.end(), run.top())],
                    sides: run.sides().ok_or_else(unmeasured)?,
                })
            }
        }
    }

    pub(super) fn handrails(
        &self,
        request: &HandrailRequest,
    ) -> Result<HandrailEvidence, WalkingSurfaceError> {
        let subject = request.subject();
        let pitch = self.pitch(subject, request.stretch())?;
        let (Some(first), Some(last)) = (pitch.points.first(), pitch.points.last()) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let ends = (first.0, last.0);
        let elevations = pitch.points.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(low, high), (_, elevation)| {
                (
                    low.min(elevation.lower_metres()),
                    high.max(elevation.upper_metres()),
                )
            },
        );
        let line = pitch.nominal();
        let stretch = Stretch {
            request,
            frame: PlanFrame::new(pitch.direction),
            strip: (
                (ends.0.lower_metres(), ends.1.upper_metres()),
                (
                    pitch.sides.0.lower_metres() - request.reach(),
                    pitch.sides.1.upper_metres() + request.reach(),
                ),
            ),
            elevations,
            slope: Pitch::slope(&line),
            knots: line.iter().map(|point| point.0).collect(),
            line,
            pitch: &pitch,
        };
        let mut rails = Vec::new();
        for rail in request.rails() {
            if let Some(found) = self.rail_along(rail, &stretch)? {
                rails.push((rail.clone(), found));
            }
        }
        let evidence = Evidence {
            source: subject.source.clone(),
            locator: format!("handrails:{subject}"),
            exact: false,
        };
        HandrailEvidence::try_new(
            request.clone(),
            pitch.direction,
            ends,
            pitch.sides,
            rails,
            evidence,
        )
    }

    /// One requested rail, `None` where it has no body or does not run
    /// along the stretch.
    fn rail_along(
        &self,
        rail: &ObjectId,
        stretch: &Stretch<'_>,
    ) -> Result<Option<RailMeasurement>, WalkingSurfaceError> {
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
        let invalid = || {
            WalkingSurfaceError::Unavailable(format!("rail {rail} has an invalid chord deviation"))
        };
        let Some((min, max)) = self.geometry.enclosing_extent(rail) else {
            if mesh_extent(mesh).is_none() {
                return Ok(None);
            }
            return Err(invalid());
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
        RailMeasurement::try_new(
            (widened(start, deviation)?, widened(end, deviation)?),
            (widened(left, deviation)?, widened(right, deviation)?),
            measured(found.lowest, margin)?,
            measured(found.highest, margin)?,
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
        let (line, frame) = (&stretch.line, &stretch.frame);
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
        let (Some(first), Some(last)) = (first, last) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let (from, to) = edges.along();
        let over = (from.max(first.0), to.min(last.0));
        if over.0 > over.1 {
            return Ok(None);
        }
        let Some((lowest, highest, steepest)) =
            edges.range(over, &stretch.knots, |along| pitch_at(line, along))
        else {
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
            bottom_rise: beyond((first.0 - extension, first.0)),
            top_rise: beyond((last.0, last.0 + extension)),
        }))
    }
}

/// What every rail along one stretch is measured against.
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

/// A rail's measurement before its margins.
struct RailFound {
    start: ElevationInterval,
    end: ElevationInterval,
    across: (ElevationInterval, ElevationInterval),
    lowest: f64,
    highest: f64,
    steepest: f64,
    scale: f64,
    bottom_rise: Option<(f64, f64)>,
    top_rise: Option<(f64, f64)>,
}
