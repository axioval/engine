//! Stair flights and ramps: treads, risers, goings, sloped runs and the
//! headroom above a walking surface.
//!
//! ADR 0004: this seam measures. Whether a riser is too high, a flight too
//! irregular or a ramp too steep for its length is a rule's judgement over
//! these measurements.
//!
//! A service reports **positions**, never the derived lengths: tread
//! elevations, the positions of each tread's front and back edge along the
//! flight's walking line, a run's lowest and highest points and its extent
//! along its own ascending direction. Risers, goings, tread depths,
//! nosings, rises, run lengths, slopes, tread widths and winder angles are
//! computed here from those positions as intervals sure to hold the exact
//! value, so no adapter can report a riser that disagrees with its treads.
//! A position is an interval; the evidence is exact exactly when every
//! position is a point.
//!
//! Widths come the same way: a tread or a run may report the positions of
//! its two sides across its walking direction ([`across`]), stated only
//! where it fills the rectangle between its ends and those sides. A landing
//! ([`LandingEvidence`]) is the level surface a request's candidate carries
//! at one end of a flight or run, reported as the positions of its far side
//! and its sides along the direction leaving that end; its depth is taken
//! from the end's arrival line. The clearance below a subject
//! ([`ClearanceBelow`]) is its height above the floors of the spaces a
//! request names.
//!
//! A flight climbs along its **walking line** ([`WalkingLine`]): a straight
//! flight along one horizontal direction, a turning flight (winders, a
//! quarter turn) along a plan polyline with a vertex on every tread. The
//! request says where a turning flight's line runs
//! ([`WalkingLinePlacement`]): midway across the treads, or at a stated
//! distance from the side the flight turns towards. Goings are measured
//! along that line, nosing to nosing. A turning flight's tread walks along
//! its own direction, square to its nosing, and a winder fills no rectangle
//! along any, so it has no sides and the flight no width. Treads may also
//! carry their nosing edge ([`PlanSegment`]), from which winder angles are
//! derived, and whether the riser below them is closed ([`RiserClosure`]).
//!
//! Handrails ([`HandrailEvidence`]) along a flight or a run are the rule's
//! selection, reported as positions along and across the walking direction
//! and as the height of their top above the pitch line: the nosing line of a
//! flight, the surface of a run. A turning flight's handrails are measured
//! part by part ([`StretchPart`]): each rail along one of its straight runs
//! of treads, in that run's frame. The rails along one side are the pieces of
//! its handrail, ordered bottom to top here ([`HandrailEvidence::side_rail`])
//! with the gaps between them ([`HandrailEvidence::gap`]), never by an
//! adapter.
//!
//! A service refuses a shape it cannot decide rather than approximate it.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::{ConvexPlanRegion, ElevationInterval, MetricDirection};

/// Failure to measure a stair flight, a ramp or headroom.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WalkingSurfaceError {
    /// The service holds no geometry for this object.
    #[error("no geometry for `{0}`")]
    UnknownObject(ObjectId),
    /// The geometry could not be measured: a bodiless object, a body the
    /// host could not mesh, or a mesh that cannot be read.
    #[error("walking surface unavailable: {0}")]
    Unavailable(String),
    /// The body is not a shape this service can decide, such as a winder,
    /// a flight in several pieces or a warped ramp. Never a verdict.
    #[error("walking surface unsupported: {0}")]
    Unsupported(String),
    /// A tessellation of curved faces could change the answer, so the
    /// service refuses rather than report an estimate as a measurement.
    #[error("walking surface is inexact: {0}")]
    InexactGeometry(String),
    /// Positions are unordered, a direction is not horizontal, or the
    /// measurement names another object or request.
    #[error("walking surface measurement is invalid")]
    InvalidMeasurement,
    /// Evidence reported as exact for intervals, or as inexact for points.
    #[error("walking surface evidence does not match its exactness")]
    InexactEvidence,
}

/// A length or ratio known to lie in `[lower, upper]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredInterval {
    lower: f64,
    upper: f64,
}

impl MeasuredInterval {
    /// An interval; bounds must be finite and ordered.
    pub fn try_new(lower: f64, upper: f64) -> Result<Self, WalkingSurfaceError> {
        if !lower.is_finite() || !upper.is_finite() || lower > upper {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self { lower, upper })
    }

    /// Smallest possible value.
    #[must_use]
    pub fn lower(&self) -> f64 {
        self.lower
    }

    /// Largest possible value.
    #[must_use]
    pub fn upper(&self) -> f64 {
        self.upper
    }

    /// Whether the value is a single number.
    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn is_point(&self) -> bool {
        self.lower == self.upper
    }
}

/// `a - b` for positions, as an interval sure to hold the exact difference
/// of any two values the positions may take.
fn between(a: ElevationInterval, b: ElevationInterval) -> MeasuredInterval {
    let low = subtract_down(a.lower_metres(), b.upper_metres());
    let high = subtract_up(a.upper_metres(), b.lower_metres());
    MeasuredInterval {
        lower: low,
        upper: high.max(low),
    }
}

/// `x - y` rounded towards negative infinity.
fn subtract_down(x: f64, y: f64) -> f64 {
    let (rounded, error) = two_difference(x, y);
    if error < 0.0 {
        rounded.next_down()
    } else {
        rounded
    }
}

/// `x - y` rounded towards positive infinity.
fn subtract_up(x: f64, y: f64) -> f64 {
    let (rounded, error) = two_difference(x, y);
    if error > 0.0 {
        rounded.next_up()
    } else {
        rounded
    }
}

/// The rounded difference and its rounding error: `x - y = rounded + error`
/// exactly (two-sum).
fn two_difference(x: f64, y: f64) -> (f64, f64) {
    let rounded = x - y;
    let back = rounded - x;
    let error = (x - (rounded - back)) + (-y - back);
    (rounded, error)
}

/// `x / y` for positive `y`, rounded towards negative (`up == false`) or
/// positive infinity.
fn divide(x: f64, y: f64, up: bool) -> f64 {
    let rounded = x / y;
    // `rounded * y - x` exactly, with one rounding: its sign says on which
    // side of the exact quotient `rounded` lies.
    let residual = rounded.mul_add(y, -x);
    match (up, residual) {
        (true, residual) if residual < 0.0 => rounded.next_up(),
        (false, residual) if residual > 0.0 => rounded.next_down(),
        _ => rounded,
    }
}

/// A straight edge in plan whose ends are known within `radius` metres.
///
/// A tread's nosing is one: the edge the walking line climbs onto it
/// across. An exact mesh gives its ends as points (radius zero); a
/// tessellation widens them by its chord deviation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanSegment {
    from: [f64; 2],
    to: [f64; 2],
    radius: f64,
}

impl PlanSegment {
    /// The edge from `from` to `to`, each end within `radius` of the given
    /// point. Coordinates must be finite, the ends distinct and the radius
    /// finite and not negative.
    pub fn try_new(from: [f64; 2], to: [f64; 2], radius: f64) -> Result<Self, WalkingSurfaceError> {
        let finite = from.iter().chain(&to).all(|value| value.is_finite());
        #[allow(clippy::float_cmp)]
        if !finite || from == to || !radius.is_finite() || radius < 0.0 {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self { from, to, radius })
    }

    /// One end.
    #[must_use]
    pub fn from(&self) -> [f64; 2] {
        self.from
    }

    /// The other end.
    #[must_use]
    pub fn to(&self) -> [f64; 2] {
        self.to
    }

    /// How far each true end may lie from the given one.
    #[must_use]
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// The angle between the lines of two edges, in radians from `0`
    /// (parallel) to `π/2` (square), as an interval sure to hold the angle
    /// between any two edges whose ends lie within the radii. `None` when an
    /// edge is too short for its radius to fix a direction.
    #[must_use]
    pub fn angle_to(&self, other: &PlanSegment) -> Option<MeasuredInterval> {
        // Moving each end of an edge of length `L` by at most `r` turns it
        // by at most `asin(2r / L)`. The subtraction below rounds each
        // component by at most `ε·|coordinate|`, which counts as radius.
        let turn = |segment: &PlanSegment| -> Option<([f64; 2], f64)> {
            let vector = [
                segment.to[0] - segment.from[0],
                segment.to[1] - segment.from[1],
            ];
            let magnitude = segment
                .from
                .iter()
                .chain(&segment.to)
                .fold(0.0_f64, |most, value| most.max(value.abs()));
            let radius = 4.0f64.mul_add(f64::EPSILON * magnitude, segment.radius);
            let length = vector[0].hypot(vector[1]) * 4.0f64.mul_add(-f64::EPSILON, 1.0);
            if length <= 2.0 * radius {
                return None;
            }
            Some((vector, (2.0 * radius / length).asin()))
        };
        let (u, first) = turn(self)?;
        let (v, second) = turn(other)?;
        let cross = u[0].mul_add(v[1], -(u[1] * v[0]));
        let dot = u[0].mul_add(v[0], u[1] * v[1]);
        let angle = cross.abs().atan2(dot.abs());
        // The products and `atan2` round by a few units in the last place
        // of an angle no larger than π/2.
        let spread = first + second + 16.0 * f64::EPSILON;
        let lower = (angle - spread).max(0.0);
        let upper = (angle + spread).min(std::f64::consts::FRAC_PI_2).max(lower);
        MeasuredInterval::try_new(lower, upper).ok()
    }

    fn is_exact(&self) -> bool {
        self.radius == 0.0
    }
}

/// Whether the riser below a tread closes the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiserClosure {
    /// A riser rises from the tread below across the whole step.
    Closed,
    /// The step is open: the tread below ends in a face falling away from
    /// it rather than in a riser climbing to this tread.
    Open,
    /// The service did not decide it, such as the first riser of a flight
    /// whose front face does not reach the level it starts from.
    NotMeasured,
}

/// One tread: an upward-facing horizontal face of a flight.
///
/// `front` and `back` are the positions along the flight's walking line
/// where the line climbs onto the tread and leaves it; the front edge is
/// the nosing. A service may add the nosing edge itself, the tread's extent
/// across the walking line and whether the riser below it is closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tread {
    elevation: ElevationInterval,
    front: ElevationInterval,
    back: ElevationInterval,
    sides: Option<Sides>,
    nosing: Option<PlanSegment>,
    riser_below: RiserClosure,
}

/// The positions of a surface's two sides across a walking direction, along
/// [`across`] it: `left` the lower, `right` the higher.
type Sides = (ElevationInterval, ElevationInterval);

/// The horizontal direction a quarter turn anticlockwise from `direction` in
/// plan: the axis widths and sides are measured along.
#[must_use]
pub fn across(direction: MetricDirection) -> MetricDirection {
    let [x, y, _] = direction.components();
    // A quarter turn swaps and negates the components: still a unit vector,
    // so the normalization leaves it as it is.
    MetricDirection::try_new([-y, x, 0.0]).unwrap_or(direction)
}

fn sides(left: ElevationInterval, right: ElevationInterval) -> Result<Sides, WalkingSurfaceError> {
    if left.lower_metres() > right.lower_metres() || left.upper_metres() > right.upper_metres() {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    }
    Ok((left, right))
}

fn sides_exact(sides: Option<Sides>) -> bool {
    sides.is_none_or(|(left, right)| left.is_exact() && right.is_exact())
}

/// The least of some widths: an interval sure to hold the narrowest.
fn least(widths: impl Iterator<Item = MeasuredInterval>) -> Option<MeasuredInterval> {
    widths.reduce(|least, width| MeasuredInterval {
        lower: least.lower.min(width.lower),
        upper: least.upper.min(width.upper),
    })
}

impl Tread {
    /// A tread at `elevation` spanning `front` to `back` along the walking
    /// line.
    pub fn try_new(
        elevation: ElevationInterval,
        front: ElevationInterval,
        back: ElevationInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        if front.lower_metres() > back.lower_metres() || front.upper_metres() > back.upper_metres()
        {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self {
            elevation,
            front,
            back,
            sides: None,
            nosing: None,
            riser_below: RiserClosure::NotMeasured,
        })
    }

    /// The tread with the positions of its sides along [`across`] its
    /// walking direction: the flight's for a straight flight, the direction
    /// square to its nosing for a turning flight's tread. A service states
    /// them only where the tread fills the rectangle between its front,
    /// back and sides, so the width holds all along its depth; a winder has
    /// none.
    pub fn with_sides(
        mut self,
        left: ElevationInterval,
        right: ElevationInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        self.sides = Some(sides(left, right)?);
        Ok(self)
    }

    /// The tread with its nosing edge in plan.
    #[must_use]
    pub fn with_nosing(mut self, nosing: PlanSegment) -> Self {
        self.nosing = Some(nosing);
        self
    }

    /// The tread with whether the riser below it closes the step.
    #[must_use]
    pub fn with_riser_below(mut self, riser: RiserClosure) -> Self {
        self.riser_below = riser;
        self
    }

    /// The positions of the tread's sides across its walking direction,
    /// when measured.
    #[must_use]
    pub fn sides(&self) -> Option<(ElevationInterval, ElevationInterval)> {
        self.sides
    }

    /// The tread's width across its walking direction, when its sides were
    /// measured. Nothing is deducted for handrails.
    #[must_use]
    pub fn width(&self) -> Option<MeasuredInterval> {
        self.sides.map(|(left, right)| between(right, left))
    }

    /// Elevation of the tread's surface.
    #[must_use]
    pub fn elevation(&self) -> ElevationInterval {
        self.elevation
    }

    /// Position of the front edge (the nosing) along the walking line.
    #[must_use]
    pub fn front(&self) -> ElevationInterval {
        self.front
    }

    /// Position of the back edge along the walking line.
    #[must_use]
    pub fn back(&self) -> ElevationInterval {
        self.back
    }

    /// The nosing edge in plan, when measured.
    #[must_use]
    pub fn nosing(&self) -> Option<PlanSegment> {
        self.nosing
    }

    /// Whether the riser below the tread closes the step.
    #[must_use]
    pub fn riser_below(&self) -> RiserClosure {
        self.riser_below
    }

    /// The tread's depth along the walking line, back less front.
    #[must_use]
    pub fn depth(&self) -> MeasuredInterval {
        between(self.back, self.front)
    }

    /// Whether every position of the tread is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.elevation.is_exact()
            && self.front.is_exact()
            && self.back.is_exact()
            && sides_exact(self.sides)
            && self.nosing.is_none_or(|nosing| nosing.is_exact())
    }
}

/// Where a turning flight's walking line runs across its treads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WalkingLinePlacement {
    /// Midway across each tread: the flight's centre line.
    Centre,
    /// This many metres from the side the flight turns towards, across
    /// each tread. Built through [`TreadFlightRequest::from_inner_side`].
    FromInnerSide(f64),
}

/// A request for an object's stair flight, with where its walking line
/// runs should it turn. A straight flight's goings are the same on every
/// line along it, so it ignores the placement.
#[derive(Clone, Debug, PartialEq)]
pub struct TreadFlightRequest {
    object: ObjectId,
    walking_line: WalkingLinePlacement,
}

impl TreadFlightRequest {
    /// The flight of `object`, its walking line on its centre line.
    #[must_use]
    pub fn new(object: ObjectId) -> Self {
        Self {
            object,
            walking_line: WalkingLinePlacement::Centre,
        }
    }

    /// The flight of `object`, its walking line `offset` metres from the
    /// side it turns towards. The offset must be finite and positive.
    pub fn from_inner_side(object: ObjectId, offset: f64) -> Result<Self, WalkingSurfaceError> {
        if !offset.is_finite() || offset <= 0.0 {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self {
            object,
            walking_line: WalkingLinePlacement::FromInnerSide(offset),
        })
    }

    /// The object whose flight is measured.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// Where the walking line runs.
    #[must_use]
    pub fn walking_line(&self) -> WalkingLinePlacement {
        self.walking_line
    }
}

/// The line a flight is walked along, in plan.
#[derive(Clone, Debug, PartialEq)]
pub enum WalkingLine {
    /// A straight flight climbs along one horizontal direction; positions
    /// along it are projections onto that direction.
    Straight(MetricDirection),
    /// A turning flight climbs along a polyline with a vertex on every
    /// tread, bottom to top; positions along it are arc lengths from its
    /// first vertex, before that vertex and after its last along its end
    /// segments extended.
    Turning(Vec<[f64; 2]>),
}

impl WalkingLine {
    /// Whether the line turns.
    #[must_use]
    pub fn is_turning(&self) -> bool {
        matches!(self, Self::Turning(_))
    }

    #[allow(clippy::float_cmp)]
    fn is_valid(&self) -> bool {
        match self {
            Self::Straight(direction) => direction.components()[2] == 0.0,
            Self::Turning(vertices) => {
                vertices.len() >= 2
                    && vertices.iter().flatten().all(|value| value.is_finite())
                    && vertices.windows(2).all(|pair| pair[0] != pair[1])
            }
        }
    }
}

/// A stair flight measured from its body.
///
/// The flight rises from `base`, its body's lowest point, through its treads
/// in ascending order, to `top`, its body's highest point. The first riser
/// runs from the base to the first tread: the flight is taken to stand on
/// the level it starts from. When the top lies above the last tread, the
/// flight ends in a riser from the last tread to the top, meeting the upper
/// floor; when it lies no higher than the last tread's surface, the last
/// tread is the top step. A top the intervals cannot place either way is
/// refused.
#[derive(Clone, Debug, PartialEq)]
pub struct TreadFlight {
    request: TreadFlightRequest,
    walking_line: WalkingLine,
    base: ElevationInterval,
    top: ElevationInterval,
    treads: Vec<Tread>,
    ends_in_riser: bool,
    final_riser: RiserClosure,
    evidence: Evidence,
}

impl TreadFlight {
    /// The flight answering `request`, climbing along `walking_line`.
    ///
    /// Treads must be given bottom to top, strictly ascending in elevation
    /// and in front position; the base must lie below the first tread and
    /// the top at or above the last, decidably. A straight line must be
    /// horizontal, a turning one at least two distinct plan points. The
    /// evidence is exact exactly when every position is a point.
    pub fn try_new(
        request: TreadFlightRequest,
        walking_line: WalkingLine,
        base: ElevationInterval,
        top: ElevationInterval,
        treads: Vec<Tread>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        if !walking_line.is_valid() {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        let (Some(first), Some(last)) = (treads.first(), treads.last()) else {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        if base.upper_metres() >= first.elevation.lower_metres() {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        for pair in treads.windows(2) {
            if pair[0].elevation.upper_metres() >= pair[1].elevation.lower_metres()
                || pair[0].front.upper_metres() >= pair[1].front.lower_metres()
            {
                return Err(WalkingSurfaceError::InvalidMeasurement);
            }
        }
        let ends_in_riser = if top.lower_metres() > last.elevation.upper_metres() {
            true
        } else if top.upper_metres() <= last.elevation.upper_metres()
            && top.upper_metres() >= last.elevation.lower_metres()
        {
            // The top lies on the last tread's own surface.
            false
        } else {
            // Whether the flight ends in a riser cannot be decided.
            return Err(WalkingSurfaceError::InvalidMeasurement);
        };
        let exact = base.is_exact() && top.is_exact() && treads.iter().all(Tread::is_exact);
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(WalkingSurfaceError::InexactEvidence);
        }
        Ok(Self {
            request,
            walking_line,
            base,
            top,
            treads,
            ends_in_riser,
            final_riser: RiserClosure::NotMeasured,
            evidence,
        })
    }

    /// The flight with whether its final riser, from the last tread to the
    /// top, closes the step; ignored when the flight ends on its last
    /// tread.
    #[must_use]
    pub fn with_final_riser(mut self, riser: RiserClosure) -> Self {
        self.final_riser = riser;
        self
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &TreadFlightRequest {
        &self.request
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.request.object
    }

    /// The line the flight climbs along.
    #[must_use]
    pub fn walking_line(&self) -> &WalkingLine {
        &self.walking_line
    }

    /// Elevation of the body's lowest point, where the first riser starts.
    #[must_use]
    pub fn base(&self) -> ElevationInterval {
        self.base
    }

    /// Elevation of the body's highest point.
    #[must_use]
    pub fn top(&self) -> ElevationInterval {
        self.top
    }

    /// The treads, bottom to top.
    #[must_use]
    pub fn treads(&self) -> &[Tread] {
        &self.treads
    }

    /// Whether the flight ends in a riser above its last tread.
    #[must_use]
    pub fn ends_in_riser(&self) -> bool {
        self.ends_in_riser
    }

    /// Riser heights, bottom to top: from the base to the first tread,
    /// between consecutive treads, and from the last tread to the top when
    /// the flight ends in a riser. Their number is the flight's number of
    /// risers (steps).
    #[must_use]
    pub fn risers(&self) -> Vec<MeasuredInterval> {
        let mut levels = vec![self.base];
        levels.extend(self.treads.iter().map(|tread| tread.elevation));
        if self.ends_in_riser {
            levels.push(self.top);
        }
        levels
            .windows(2)
            .map(|pair| between(pair[1], pair[0]))
            .collect()
    }

    /// Whether each riser closes its step, in the order of
    /// [`Self::risers`]: the riser below each tread, then the final riser
    /// when the flight ends in one.
    #[must_use]
    pub fn riser_closures(&self) -> Vec<RiserClosure> {
        let mut closures: Vec<RiserClosure> =
            self.treads.iter().map(|tread| tread.riser_below).collect();
        if self.ends_in_riser {
            closures.push(self.final_riser);
        }
        closures
    }

    /// Goings, bottom to top: the distance along the walking line from
    /// each tread's nosing to the next one's. A flight of `n` treads has
    /// `n - 1` goings.
    #[must_use]
    pub fn goings(&self) -> Vec<MeasuredInterval> {
        self.treads
            .windows(2)
            .map(|pair| between(pair[1].front, pair[0].front))
            .collect()
    }

    /// Nosing projections, bottom to top: how far each tread above the
    /// first reaches over the tread below it, the lower tread's back less
    /// the upper tread's front. Zero or less where it does not overhang.
    #[must_use]
    pub fn nosings(&self) -> Vec<MeasuredInterval> {
        self.treads
            .windows(2)
            .map(|pair| between(pair[0].back, pair[1].front))
            .collect()
    }

    /// Winder angles, bottom to top: for each tread below the last, the
    /// plan angle between its nosing and the next tread's, `0` for a
    /// straight tread. `None` where a nosing is not measured or too short
    /// for its uncertainty to fix a direction.
    #[must_use]
    pub fn winder_angles(&self) -> Vec<Option<MeasuredInterval>> {
        self.treads
            .windows(2)
            .map(|pair| match (pair[0].nosing, pair[1].nosing) {
                (Some(lower), Some(upper)) => lower.angle_to(&upper),
                _ => None,
            })
            .collect()
    }

    /// The flight's rise: from the base to the top of its last riser.
    #[must_use]
    pub fn rise(&self) -> MeasuredInterval {
        let summit = if self.ends_in_riser {
            self.top
        } else {
            self.treads.last().map_or(self.top, |tread| tread.elevation)
        };
        between(summit, self.base)
    }

    /// The flight's width: its narrowest tread's, when every tread's sides
    /// were measured.
    #[must_use]
    pub fn width(&self) -> Option<MeasuredInterval> {
        let widths: Option<Vec<MeasuredInterval>> = self.treads.iter().map(Tread::width).collect();
        least(widths?.into_iter())
    }

    /// Whether every position is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// One sloped run of a ramp: a planar upward-facing walking face.
///
/// `start` and `end` are the positions of its nearest and farthest points
/// along `direction`, the run's horizontal direction of steepest ascent;
/// `bottom` and `top` the elevations of its lowest and highest points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlopedRun {
    direction: MetricDirection,
    bottom: ElevationInterval,
    top: ElevationInterval,
    start: ElevationInterval,
    end: ElevationInterval,
    sides: Option<Sides>,
}

impl SlopedRun {
    /// A run rising from `bottom` to `top` between `start` and `end` along
    /// the horizontal `direction`. Both must be decidably ordered: a run
    /// with no rise or no length is no sloped run.
    pub fn try_new(
        direction: MetricDirection,
        bottom: ElevationInterval,
        top: ElevationInterval,
        start: ElevationInterval,
        end: ElevationInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        #[allow(clippy::float_cmp)]
        if direction.components()[2] != 0.0
            || bottom.upper_metres() >= top.lower_metres()
            || start.upper_metres() >= end.lower_metres()
        {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self {
            direction,
            bottom,
            top,
            start,
            end,
            sides: None,
        })
    }

    /// The run with the positions of its sides along [`across`] its
    /// direction, stated only where its plan fills the rectangle between its
    /// ends and those sides.
    pub fn with_sides(
        mut self,
        left: ElevationInterval,
        right: ElevationInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        self.sides = Some(sides(left, right)?);
        Ok(self)
    }

    /// The positions of the run's sides across its direction, when measured.
    #[must_use]
    pub fn sides(&self) -> Option<(ElevationInterval, ElevationInterval)> {
        self.sides
    }

    /// The run's width across its direction, when its sides were measured.
    #[must_use]
    pub fn width(&self) -> Option<MeasuredInterval> {
        self.sides.map(|(left, right)| between(right, left))
    }

    /// The run's horizontal direction of steepest ascent.
    #[must_use]
    pub fn direction(&self) -> MetricDirection {
        self.direction
    }

    /// Elevation of the run's lowest point.
    #[must_use]
    pub fn bottom(&self) -> ElevationInterval {
        self.bottom
    }

    /// Elevation of the run's highest point.
    #[must_use]
    pub fn top(&self) -> ElevationInterval {
        self.top
    }

    /// Position of the run's lowest end along its direction.
    #[must_use]
    pub fn start(&self) -> ElevationInterval {
        self.start
    }

    /// Position of the run's highest end along its direction.
    #[must_use]
    pub fn end(&self) -> ElevationInterval {
        self.end
    }

    /// The run's rise, top less bottom.
    #[must_use]
    pub fn rise(&self) -> MeasuredInterval {
        between(self.top, self.bottom)
    }

    /// The run's horizontal length along its direction, end less start.
    #[must_use]
    pub fn length(&self) -> MeasuredInterval {
        between(self.end, self.start)
    }

    /// The run's slope, rise over horizontal length. On a planar face the
    /// height changes linearly along the direction of steepest ascent, so
    /// this is the face's gradient.
    #[must_use]
    pub fn slope(&self) -> MeasuredInterval {
        let (rise, length) = (self.rise(), self.length());
        let lower = divide(rise.lower.max(0.0), length.upper, false);
        let upper = divide(rise.upper, length.lower, true);
        MeasuredInterval {
            lower,
            upper: upper.max(lower),
        }
    }

    /// Whether every position of the run is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.bottom.is_exact()
            && self.top.is_exact()
            && self.start.is_exact()
            && self.end.is_exact()
            && sides_exact(self.sides)
    }
}

/// A ramp measured from its body: its sloped runs, ordered by bottom
/// elevation. Horizontal faces between them (landings) are not runs.
#[derive(Clone, Debug, PartialEq)]
pub struct SlopedSurface {
    object: ObjectId,
    runs: Vec<SlopedRun>,
    evidence: Evidence,
}

impl SlopedSurface {
    /// The runs of `object`, at least one. The evidence is exact exactly
    /// when every position is a point.
    pub fn try_new(
        object: ObjectId,
        runs: Vec<SlopedRun>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        if runs.is_empty() {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        let exact = runs.iter().all(SlopedRun::is_exact);
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(WalkingSurfaceError::InexactEvidence);
        }
        Ok(Self {
            object,
            runs,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The sloped runs.
    #[must_use]
    pub fn runs(&self) -> &[SlopedRun] {
        &self.runs
    }

    /// Whether every position is known exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.evidence.exact
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// A request for the headroom above an object's walking surface.
///
/// The obstacles are the rule's selection, never the service's: sorted,
/// deduplicated, and without the subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadroomRequest {
    subject: ObjectId,
    obstacles: Vec<ObjectId>,
}

impl HeadroomRequest {
    /// Headroom above `subject` against `obstacles`.
    #[must_use]
    pub fn new(subject: ObjectId, obstacles: impl IntoIterator<Item = ObjectId>) -> Self {
        let mut obstacles: Vec<ObjectId> = obstacles
            .into_iter()
            .filter(|obstacle| *obstacle != subject)
            .collect();
        obstacles.sort();
        obstacles.dedup();
        Self { subject, obstacles }
    }

    /// The object whose walking surface is measured.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// The objects that may stand above it.
    #[must_use]
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }
}

/// The vertical clearance above an object's walking surface.
///
/// The walking surface is the subject's upward-facing faces no steeper than
/// 45°: treads, ramp slopes and landings. The clearance is the least
/// vertical distance from a point of it to an obstacle's body directly
/// above; `None` when no requested obstacle stands above it anywhere.
#[derive(Clone, Debug, PartialEq)]
pub struct Headroom {
    request: HeadroomRequest,
    clearance: Option<MeasuredInterval>,
    governing: Vec<ObjectId>,
    evidence: Evidence,
}

impl Headroom {
    /// The headroom answering `request`. `governing` names the obstacles
    /// whose clearance may be the least one: at least one when there is a
    /// clearance, none otherwise, every one requested. A clearance interval
    /// is never exact evidence.
    pub fn try_new(
        request: HeadroomRequest,
        clearance: Option<MeasuredInterval>,
        governing: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        let governing = governed(&request.obstacles, clearance, governing, &evidence)?;
        Ok(Self {
            request,
            clearance,
            governing,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &HeadroomRequest {
        &self.request
    }

    /// The least vertical clearance, or `None` when nothing stands above.
    #[must_use]
    pub fn clearance(&self) -> Option<MeasuredInterval> {
        self.clearance
    }

    /// The obstacles whose clearance may be the least.
    #[must_use]
    pub fn governing(&self) -> &[ObjectId] {
        &self.governing
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// One end of a stair flight or of a ramp's run.
///
/// A flight's end is placed along the direction its end tread climbs: the
/// flight's own for a straight flight, the direction square to the end
/// tread's nosing for a turning flight, whose landing positions are then
/// projections onto that direction in plan, never arc lengths along its
/// walking line. A turning flight ending on a winder has no such direction,
/// and a service refuses that end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WalkingEnd {
    /// Where a flight starts, in front of its first riser.
    FlightBottom,
    /// Where a flight arrives, beyond its last riser.
    FlightTop,
    /// The lower end of a ramp's run, by its index in
    /// [`SlopedSurface::runs`].
    RunBottom(usize),
    /// The upper end of a ramp's run, by its index.
    RunTop(usize),
}

/// A request for the landing at one end of a flight or run.
///
/// The candidates are the objects that may carry it, the rule's selection
/// (slabs, landings, floors): sorted, deduplicated and without the subject.
/// A service may also take a ramp's own level faces as its landing, never a
/// flight's own treads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandingRequest {
    subject: ObjectId,
    end: WalkingEnd,
    candidates: Vec<ObjectId>,
}

impl LandingRequest {
    /// The landing at `end` of `subject` among `candidates`.
    #[must_use]
    pub fn new(
        subject: ObjectId,
        end: WalkingEnd,
        candidates: impl IntoIterator<Item = ObjectId>,
    ) -> Self {
        let mut candidates: Vec<ObjectId> = candidates
            .into_iter()
            .filter(|candidate| *candidate != subject)
            .collect();
        candidates.sort();
        candidates.dedup();
        Self {
            subject,
            end,
            candidates,
        }
    }

    /// The flight or ramp whose end is measured.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// Which end.
    #[must_use]
    pub fn end(&self) -> WalkingEnd {
        self.end
    }

    /// The objects that may carry the landing.
    #[must_use]
    pub fn candidates(&self) -> &[ObjectId] {
        &self.candidates
    }
}

/// The rectangle of a landing along the direction leaving the flight or
/// run: the position of its far side and of its two sides along [`across`]
/// that direction. A service states it only where the landing's level
/// surface fills that rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandingExtent {
    far: ElevationInterval,
    left: ElevationInterval,
    right: ElevationInterval,
}

impl LandingExtent {
    /// A landing reaching `far` along the leaving direction, between the
    /// sides `left` and `right` across it.
    pub fn try_new(
        far: ElevationInterval,
        left: ElevationInterval,
        right: ElevationInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        let (left, right) = sides(left, right)?;
        Ok(Self { far, left, right })
    }

    /// Position of the landing's far side along the leaving direction.
    #[must_use]
    pub fn far(&self) -> ElevationInterval {
        self.far
    }

    /// Positions of the landing's sides across the leaving direction.
    #[must_use]
    pub fn sides(&self) -> (ElevationInterval, ElevationInterval) {
        (self.left, self.right)
    }

    fn is_exact(&self) -> bool {
        self.far.is_exact() && sides_exact(Some((self.left, self.right)))
    }
}

/// The object carrying a landing and, when it is a rectangle along the
/// leaving direction, its extent.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    carrier: ObjectId,
    extent: Option<LandingExtent>,
}

impl Landing {
    /// A landing on `carrier`, measured when `extent` is given.
    #[must_use]
    pub fn new(carrier: ObjectId, extent: Option<LandingExtent>) -> Self {
        Self { carrier, extent }
    }

    /// The object whose level surface meets the end: a requested candidate,
    /// or the subject itself for a ramp's own landing.
    #[must_use]
    pub fn carrier(&self) -> &ObjectId {
        &self.carrier
    }

    /// The landing's rectangle, `None` when its surface is no rectangle
    /// along the leaving direction and so was not measured.
    #[must_use]
    pub fn extent(&self) -> Option<LandingExtent> {
        self.extent
    }
}

/// The landing at one end of a flight or run.
///
/// `direction` is the horizontal direction leaving the subject at that end
/// (back down the flight's direction at its bottom, on at its top); `edge`
/// the position along it of the end's arrival line: the first riser at a
/// flight's bottom, the last riser at its top, a run's end. A landing's
/// positions are along the same direction, so its depth is its far side
/// less the edge. `landing` is `None` when no candidate's level surface at
/// the end's elevation meets the end: nothing selected carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct LandingEvidence {
    request: LandingRequest,
    direction: MetricDirection,
    edge: ElevationInterval,
    landing: Option<Landing>,
    evidence: Evidence,
}

impl LandingEvidence {
    /// The landing answering `request`. The carrier must be the subject or a
    /// requested candidate, and a measured landing's far side must lie
    /// decidably beyond the edge; the evidence is exact exactly when every
    /// position is a point.
    pub fn try_new(
        request: LandingRequest,
        direction: MetricDirection,
        edge: ElevationInterval,
        landing: Option<Landing>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        #[allow(clippy::float_cmp)]
        if direction.components()[2] != 0.0 {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        if let Some(landing) = &landing {
            let named = landing.carrier == request.subject
                || request.candidates.binary_search(&landing.carrier).is_ok();
            let beyond = landing
                .extent
                .is_none_or(|extent| extent.far.lower_metres() > edge.upper_metres());
            if !named || !beyond {
                return Err(WalkingSurfaceError::InvalidMeasurement);
            }
        }
        let exact = edge.is_exact()
            && landing
                .as_ref()
                .and_then(|landing| landing.extent)
                .is_none_or(|extent| extent.is_exact());
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(WalkingSurfaceError::InexactEvidence);
        }
        Ok(Self {
            request,
            direction,
            edge,
            landing,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &LandingRequest {
        &self.request
    }

    /// The horizontal direction leaving the subject at the end.
    #[must_use]
    pub fn direction(&self) -> MetricDirection {
        self.direction
    }

    /// Position of the end's arrival line along the leaving direction.
    #[must_use]
    pub fn edge(&self) -> ElevationInterval {
        self.edge
    }

    /// The landing, or `None` when nothing requested carries one.
    #[must_use]
    pub fn landing(&self) -> Option<&Landing> {
        self.landing.as_ref()
    }

    /// The landing's depth along the leaving direction, from the arrival
    /// line to its far side, when its extent was measured.
    #[must_use]
    pub fn depth(&self) -> Option<MeasuredInterval> {
        let extent = self.landing.as_ref()?.extent?;
        Some(between(extent.far, self.edge))
    }

    /// The landing's width across the leaving direction, when its extent was
    /// measured.
    #[must_use]
    pub fn width(&self) -> Option<MeasuredInterval> {
        let extent = self.landing.as_ref()?.extent?;
        Some(between(extent.right, extent.left))
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// A request for the clearance below a flight or ramp: its height above the
/// floors of the spaces people walk in beneath it.
///
/// The spaces are the rule's selection, sorted, deduplicated and without the
/// subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearanceBelowRequest {
    subject: ObjectId,
    spaces: Vec<ObjectId>,
}

impl ClearanceBelowRequest {
    /// The clearance below `subject` over the floors of `spaces`.
    #[must_use]
    pub fn new(subject: ObjectId, spaces: impl IntoIterator<Item = ObjectId>) -> Self {
        let mut spaces: Vec<ObjectId> = spaces
            .into_iter()
            .filter(|space| *space != subject)
            .collect();
        spaces.sort();
        spaces.dedup();
        Self { subject, spaces }
    }

    /// The object whose underside is measured.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// The spaces whose floors may lie beneath it.
    #[must_use]
    pub fn spaces(&self) -> &[ObjectId] {
        &self.spaces
    }
}

/// The clearance below a flight or ramp.
///
/// A space's floor is its body's downward-facing level faces. The clearance
/// is the least vertical distance from a point of a requested space's floor
/// up to the subject's underside directly above it, leaving out where the
/// subject rests on that floor; `None` when the subject stands above no
/// requested floor.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceBelow {
    request: ClearanceBelowRequest,
    clearance: Option<MeasuredInterval>,
    governing: Vec<ObjectId>,
    evidence: Evidence,
}

impl ClearanceBelow {
    /// The clearance answering `request`. `governing` names the spaces whose
    /// floor may lie closest below: at least one when there is a clearance,
    /// none otherwise, every one requested. A clearance interval is never
    /// exact evidence.
    pub fn try_new(
        request: ClearanceBelowRequest,
        clearance: Option<MeasuredInterval>,
        governing: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        let governing = governed(&request.spaces, clearance, governing, &evidence)?;
        Ok(Self {
            request,
            clearance,
            governing,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &ClearanceBelowRequest {
        &self.request
    }

    /// The least vertical clearance, or `None` when no requested floor lies
    /// beneath.
    #[must_use]
    pub fn clearance(&self) -> Option<MeasuredInterval> {
        self.clearance
    }

    /// The spaces whose floor may lie closest below.
    #[must_use]
    pub fn governing(&self) -> &[ObjectId] {
        &self.governing
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// `governing` sorted and checked against the requested `named` objects, a
/// clearance and its evidence, as [`Headroom`] and [`ClearanceBelow`] share.
fn governed(
    named: &[ObjectId],
    clearance: Option<MeasuredInterval>,
    mut governing: Vec<ObjectId>,
    evidence: &Evidence,
) -> Result<Vec<ObjectId>, WalkingSurfaceError> {
    governing.sort();
    governing.dedup();
    let requested = governing
        .iter()
        .all(|object| named.binary_search(object).is_ok());
    if !requested || clearance.is_some() == governing.is_empty() {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    }
    if clearance.is_some_and(|clearance| clearance.lower < 0.0) {
        return Err(WalkingSurfaceError::InvalidMeasurement);
    }
    if evidence.locator.trim().is_empty()
        || (evidence.exact && clearance.is_some_and(|clearance| !clearance.is_point()))
    {
        return Err(WalkingSurfaceError::InexactEvidence);
    }
    Ok(governing)
}

/// The stretch of walking surface a handrail is measured along: a stair
/// flight, or one run of a ramp by its index in [`SlopedSurface::runs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WalkingStretch {
    /// A stair flight, along its nosing line: in one part when straight, in
    /// its straight parts when it turns ([`StretchPart`]).
    Flight,
    /// A ramp's run, along its surface.
    Run(usize),
}

/// A request for the handrails along a flight or a run.
///
/// The rails are the rule's selection (railings of a handrail type, say):
/// sorted, deduplicated and without the subject. `reach` is how far outside
/// the walking surface's sides a rail may run and still be measured along
/// it, `above` how far above the pitch line's highest point its lowest point
/// may lie (so the rail of a flight stacked above is not taken for this
/// one's), and `extension` how far beyond each end of the pitch line the
/// rise of a rail's top is measured, zero for none.
#[derive(Clone, Debug, PartialEq)]
pub struct HandrailRequest {
    subject: ObjectId,
    stretch: WalkingStretch,
    rails: Vec<ObjectId>,
    reach: f64,
    above: f64,
    extension: f64,
}

impl HandrailRequest {
    /// The handrails along `stretch` of `subject` among `rails`. `reach`,
    /// `above` and `extension` must be finite and not negative.
    pub fn try_new(
        subject: ObjectId,
        stretch: WalkingStretch,
        rails: impl IntoIterator<Item = ObjectId>,
        (reach, above): (f64, f64),
        extension: f64,
    ) -> Result<Self, WalkingSurfaceError> {
        if [reach, above, extension]
            .iter()
            .any(|length| !length.is_finite() || *length < 0.0)
        {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        let mut rails: Vec<ObjectId> = rails.into_iter().filter(|rail| *rail != subject).collect();
        rails.sort();
        rails.dedup();
        Ok(Self {
            subject,
            stretch,
            rails,
            reach,
            above,
            extension,
        })
    }

    /// The flight or ramp the rails run along.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// Which stretch of it.
    #[must_use]
    pub fn stretch(&self) -> WalkingStretch {
        self.stretch
    }

    /// The objects that may be its handrails.
    #[must_use]
    pub fn rails(&self) -> &[ObjectId] {
        &self.rails
    }

    /// How far outside the walking surface's sides a rail is still measured.
    #[must_use]
    pub fn reach(&self) -> f64 {
        self.reach
    }

    /// How far above the pitch line's highest point a rail's lowest point
    /// may lie and still be measured.
    #[must_use]
    pub fn above(&self) -> f64 {
        self.above
    }

    /// How far beyond each end of the pitch line a rail's rise is measured.
    #[must_use]
    pub fn extension(&self) -> f64 {
        self.extension
    }
}

/// A request for the clear width of a flight or a ramp's run: the free
/// width across it that the requested obstacles leave (handrails, walls,
/// anything standing beside or over the walking surface) between two
/// heights above its pitch line.
///
/// The obstacles are the rule's selection, sorted, deduplicated and without
/// the subject. `band` is `(from, to)`, how far above the pitch line (a
/// flight's nosing line, a run's surface) the band starts and ends, `0 <=
/// from < to`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearWidthRequest {
    subject: ObjectId,
    stretch: WalkingStretch,
    obstacles: Vec<ObjectId>,
    band: (f64, f64),
}

impl ClearWidthRequest {
    /// The clear width along `stretch` of `subject` that `obstacles` leave
    /// within `band` above its pitch line.
    pub fn try_new(
        subject: ObjectId,
        stretch: WalkingStretch,
        obstacles: impl IntoIterator<Item = ObjectId>,
        band: (f64, f64),
    ) -> Result<Self, WalkingSurfaceError> {
        let (from, to) = band;
        if !from.is_finite() || !to.is_finite() || from < 0.0 || to <= from {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        let mut obstacles: Vec<ObjectId> = obstacles
            .into_iter()
            .filter(|obstacle| *obstacle != subject)
            .collect();
        obstacles.sort();
        obstacles.dedup();
        Ok(Self {
            subject,
            stretch,
            obstacles,
            band,
        })
    }

    /// The flight or ramp measured.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// Which stretch of it.
    #[must_use]
    pub fn stretch(&self) -> WalkingStretch {
        self.stretch
    }

    /// The objects that may narrow it.
    #[must_use]
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }

    /// How far above the pitch line the band starts and ends.
    #[must_use]
    pub fn band(&self) -> (f64, f64) {
        self.band
    }
}

/// The narrowest clear width along a flight or run.
///
/// At each position along the stretch, between its ends, the free width is
/// the distance across between the innermost points the requested
/// obstacles reach within the band from the left and from the right, where
/// no obstacle reaches in, the walking surface's own side; the clear width
/// is the least of these. `governing` names the obstacles bounding the
/// narrowest place, none when the walking surface's own sides do.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearWidthEvidence {
    request: ClearWidthRequest,
    width: MeasuredInterval,
    governing: Vec<ObjectId>,
    evidence: Evidence,
}

impl ClearWidthEvidence {
    /// The clear width answering `request`: never negative, `governing`
    /// among the requested obstacles, and exact evidence only for a point.
    pub fn try_new(
        request: ClearWidthRequest,
        width: MeasuredInterval,
        mut governing: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        governing.sort();
        governing.dedup();
        if width.lower < 0.0
            || !governing
                .iter()
                .all(|object| request.obstacles.binary_search(object).is_ok())
        {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        if evidence.locator.trim().is_empty() || (evidence.exact && !width.is_point()) {
            return Err(WalkingSurfaceError::InexactEvidence);
        }
        Ok(Self {
            request,
            width,
            governing,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &ClearWidthRequest {
        &self.request
    }

    /// The narrowest clear width.
    #[must_use]
    pub fn width(&self) -> MeasuredInterval {
        self.width
    }

    /// The obstacles bounding the narrowest place.
    #[must_use]
    pub fn governing(&self) -> &[ObjectId] {
        &self.governing
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// A request for the clear width of the landing at one end of a flight or
/// run: the free width across the direction leaving it that the requested
/// obstacles (walls, handrails, anything standing beside or over the
/// landing) leave between two heights above the landing's level.
///
/// The landing is looked for as [`LandingRequest`] says, among its
/// candidates. The obstacles are the rule's selection, sorted, deduplicated
/// and without the subject. `band` is `(from, to)` above the landing's
/// level, `0 <= from < to`.
#[derive(Clone, Debug, PartialEq)]
pub struct LandingClearWidthRequest {
    landing: LandingRequest,
    obstacles: Vec<ObjectId>,
    band: (f64, f64),
}

impl LandingClearWidthRequest {
    /// The clear width of the landing `landing` asks for that `obstacles`
    /// leave within `band` above its level.
    pub fn try_new(
        landing: LandingRequest,
        obstacles: impl IntoIterator<Item = ObjectId>,
        band: (f64, f64),
    ) -> Result<Self, WalkingSurfaceError> {
        let (from, to) = band;
        if !from.is_finite() || !to.is_finite() || from < 0.0 || to <= from {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        let subject = landing.subject().clone();
        let mut obstacles: Vec<ObjectId> = obstacles
            .into_iter()
            .filter(|obstacle| *obstacle != subject)
            .collect();
        obstacles.sort();
        obstacles.dedup();
        Ok(Self {
            landing,
            obstacles,
            band,
        })
    }

    /// The landing measured: the subject, its end and the candidates that
    /// may carry it.
    #[must_use]
    pub fn landing(&self) -> &LandingRequest {
        &self.landing
    }

    /// The flight or ramp whose landing is measured.
    #[must_use]
    pub fn subject(&self) -> &ObjectId {
        self.landing.subject()
    }

    /// The objects that may bound or narrow it.
    #[must_use]
    pub fn obstacles(&self) -> &[ObjectId] {
        &self.obstacles
    }

    /// How far above the landing's level the band starts and ends.
    #[must_use]
    pub fn band(&self) -> (f64, f64) {
        self.band
    }
}

/// The clear width of a measured landing.
///
/// Over the landing's rectangle ([`LandingExtent`]), from the arrival line
/// to its far side, the free width at each position along the leaving
/// direction is the distance across between the innermost points the
/// requested obstacles reach within the band from either side, the
/// landing's own side where none reaches in; the clear width is the least
/// of these. `bounds` names the obstacles reaching the landing's lower and
/// higher side across (the order of [`LandingExtent::sides`]) anywhere
/// along it: a side none reaches is bounded by nothing selected, which a
/// rule judges. `governing` names the obstacles bounding the narrowest
/// place.
#[derive(Clone, Debug, PartialEq)]
pub struct LandingClearWidth {
    carrier: ObjectId,
    width: MeasuredInterval,
    governing: Vec<ObjectId>,
    bounds: (Vec<ObjectId>, Vec<ObjectId>),
}

impl LandingClearWidth {
    /// The clear width `width` of the landing on `carrier`, bounded by
    /// `bounds` on its two sides and at its narrowest by `governing`.
    #[must_use]
    pub fn new(
        carrier: ObjectId,
        width: MeasuredInterval,
        mut governing: Vec<ObjectId>,
        (mut low, mut high): (Vec<ObjectId>, Vec<ObjectId>),
    ) -> Self {
        for objects in [&mut governing, &mut low, &mut high] {
            objects.sort();
            objects.dedup();
        }
        Self {
            carrier,
            width,
            governing,
            bounds: (low, high),
        }
    }

    /// The object carrying the landing.
    #[must_use]
    pub fn carrier(&self) -> &ObjectId {
        &self.carrier
    }

    /// The narrowest clear width over the landing.
    #[must_use]
    pub fn width(&self) -> MeasuredInterval {
        self.width
    }

    /// The obstacles bounding the narrowest place.
    #[must_use]
    pub fn governing(&self) -> &[ObjectId] {
        &self.governing
    }

    /// The obstacles reaching the landing's lower and higher side across.
    #[must_use]
    pub fn bounds(&self) -> (&[ObjectId], &[ObjectId]) {
        (&self.bounds.0, &self.bounds.1)
    }
}

/// The clear width of the landing at one end of a flight or run, `None`
/// when no candidate carries a landing there.
#[derive(Clone, Debug, PartialEq)]
pub struct LandingClearWidthEvidence {
    request: LandingClearWidthRequest,
    landing: Option<LandingClearWidth>,
    evidence: Evidence,
}

impl LandingClearWidthEvidence {
    /// The landing clear width answering `request`: its carrier the subject
    /// or a requested candidate, every named obstacle requested, the width
    /// never negative and exact evidence only for a point.
    pub fn try_new(
        request: LandingClearWidthRequest,
        landing: Option<LandingClearWidth>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        if let Some(landing) = &landing {
            let carrier = &landing.carrier;
            let named = carrier == request.subject()
                || request.landing.candidates.binary_search(carrier).is_ok();
            let requested = landing
                .governing
                .iter()
                .chain(&landing.bounds.0)
                .chain(&landing.bounds.1)
                .all(|object| request.obstacles.binary_search(object).is_ok());
            if !named || !requested || landing.width.lower < 0.0 {
                return Err(WalkingSurfaceError::InvalidMeasurement);
            }
        }
        let exact = landing
            .as_ref()
            .is_none_or(|landing| landing.width.is_point());
        if evidence.locator.trim().is_empty() || (evidence.exact && !exact) {
            return Err(WalkingSurfaceError::InexactEvidence);
        }
        Ok(Self {
            request,
            landing,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &LandingClearWidthRequest {
        &self.request
    }

    /// The landing's clear width, or `None` when nothing requested carries
    /// a landing at the end.
    #[must_use]
    pub fn landing(&self) -> Option<&LandingClearWidth> {
        self.landing.as_ref()
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// The side of a flight or run a handrail runs along, as seen by someone
/// climbing it. [`across`] points to the climber's left, so the left side
/// lies at the higher positions across.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RailSide {
    /// The climber's right: the lower half of the positions across.
    Right,
    /// The climber's left: the higher half.
    Left,
}

/// One straight part of a stretch a handrail is measured along: the
/// horizontal direction it climbs and the positions of the walking
/// surface's sides along [`across`] it.
///
/// A straight flight or a ramp's run is one part. A turning flight's parts
/// are its runs of consecutive treads filling rectangles square to parallel
/// nosings, bottom to top; its winders belong to none, and a rail beside
/// them is measured in the frame of the part it runs on from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchPart {
    direction: MetricDirection,
    sides: (ElevationInterval, ElevationInterval),
}

impl StretchPart {
    /// A part climbing along the horizontal `direction`, its walking
    /// surface between the ordered sides `left` and `right` across it.
    pub fn try_new(
        direction: MetricDirection,
        (left, right): (ElevationInterval, ElevationInterval),
    ) -> Result<Self, WalkingSurfaceError> {
        #[allow(clippy::float_cmp)]
        if direction.components()[2] != 0.0 {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self {
            direction,
            sides: sides(left, right)?,
        })
    }

    /// The horizontal direction the part climbs.
    #[must_use]
    pub fn direction(&self) -> MetricDirection {
        self.direction
    }

    /// Positions of the walking surface's sides across the direction.
    #[must_use]
    pub fn sides(&self) -> (ElevationInterval, ElevationInterval) {
        self.sides
    }
}

/// One handrail measured along a stretch.
///
/// A rail is measured in the frame of one part of the stretch, the first
/// unless stated ([`Self::in_part`]). `start` and `end` are the positions
/// of its body's nearest and farthest points along that part's direction,
/// `left` and `right` of its lowest and highest points across it. `lowest`
/// and `highest` bound the height of the top of its body above the pitch
/// line (the nosing line of a flight, the surface of a run) where both run:
/// the least and the greatest height along it, each as an interval sure to
/// hold it. `bottom_rise` and `top_rise` are how much the top of its body
/// rises and falls over the requested extension beyond each end of the
/// pitch line, `None` where it does not reach that far or no extension was
/// requested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailMeasurement {
    part: usize,
    start: ElevationInterval,
    end: ElevationInterval,
    left: ElevationInterval,
    right: ElevationInterval,
    lowest: MeasuredInterval,
    highest: MeasuredInterval,
    bottom_rise: Option<MeasuredInterval>,
    top_rise: Option<MeasuredInterval>,
}

impl RailMeasurement {
    /// A rail spanning `start` to `end` along the stretch and `left` to
    /// `right` across it, its top `lowest` to `highest` above the pitch
    /// line. Each pair must be ordered.
    pub fn try_new(
        (start, end): (ElevationInterval, ElevationInterval),
        (left, right): (ElevationInterval, ElevationInterval),
        lowest: MeasuredInterval,
        highest: MeasuredInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        let (start, end) = sides(start, end)?;
        let (left, right) = sides(left, right)?;
        if lowest.lower > highest.lower || lowest.upper > highest.upper {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(Self {
            part: 0,
            start,
            end,
            left,
            right,
            lowest,
            highest,
            bottom_rise: None,
            top_rise: None,
        })
    }

    /// The rail measured in the frame of the stretch's part `part`, by its
    /// index in [`HandrailEvidence::parts`].
    #[must_use]
    pub fn in_part(mut self, part: usize) -> Self {
        self.part = part;
        self
    }

    /// The index of the part the rail is measured in.
    #[must_use]
    pub fn part(&self) -> usize {
        self.part
    }

    /// The rail with the rise of its top over the extension beyond the
    /// bottom and the top of the pitch line. A rise is never negative.
    pub fn with_rises(
        mut self,
        bottom: Option<MeasuredInterval>,
        top: Option<MeasuredInterval>,
    ) -> Result<Self, WalkingSurfaceError> {
        if [bottom, top].iter().flatten().any(|rise| rise.lower < 0.0) {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        self.bottom_rise = bottom;
        self.top_rise = top;
        Ok(self)
    }

    /// Position of the rail's nearest point along the stretch.
    #[must_use]
    pub fn start(&self) -> ElevationInterval {
        self.start
    }

    /// Position of the rail's farthest point along the stretch.
    #[must_use]
    pub fn end(&self) -> ElevationInterval {
        self.end
    }

    /// Positions of the rail's lowest and highest points across the stretch.
    #[must_use]
    pub fn sides(&self) -> (ElevationInterval, ElevationInterval) {
        (self.left, self.right)
    }

    /// The least height of the rail's top above the pitch line.
    #[must_use]
    pub fn lowest(&self) -> MeasuredInterval {
        self.lowest
    }

    /// The greatest height of the rail's top above the pitch line.
    #[must_use]
    pub fn highest(&self) -> MeasuredInterval {
        self.highest
    }

    /// How much the rail's top rises and falls over the extension beyond
    /// the bottom of the pitch line, when it reaches that far.
    #[must_use]
    pub fn bottom_rise(&self) -> Option<MeasuredInterval> {
        self.bottom_rise
    }

    /// The same beyond the top of the pitch line.
    #[must_use]
    pub fn top_rise(&self) -> Option<MeasuredInterval> {
        self.top_rise
    }

    /// The rectangle the rail fills in plan along `direction`: the one
    /// holding every rectangle its positions allow (`outer`), or the one
    /// every such rectangle holds. `None` when that is empty.
    fn plan(&self, direction: MetricDirection, outer: bool) -> Option<ConvexPlanRegion> {
        let ((near, far), (low, high)) = if outer {
            (
                (self.start.lower_metres(), self.end.upper_metres()),
                (self.left.lower_metres(), self.right.upper_metres()),
            )
        } else {
            (
                (self.start.upper_metres(), self.end.lower_metres()),
                (self.left.upper_metres(), self.right.lower_metres()),
            )
        };
        if near >= far || low >= high {
            return None;
        }
        let [dx, dy, _] = direction.components();
        let [ax, ay, _] = across(direction).components();
        let at = |along: f64, beside: f64| {
            [
                along.mul_add(dx, beside * ax),
                along.mul_add(dy, beside * ay),
            ]
        };
        // Along, then across a quarter turn anticlockwise: anticlockwise.
        ConvexPlanRegion::try_new(vec![
            at(near, low),
            at(far, low),
            at(far, high),
            at(near, high),
        ])
        .ok()
    }
}

/// The handrails along a flight or run.
///
/// `parts` are the stretch's straight parts ([`StretchPart`]), bottom to
/// top: one for a straight flight or a run. `pitch` holds the positions
/// where the pitch line starts, along the first part's direction (a
/// flight's first nosing, a run's lower end), and where it ends, along the
/// last part's (the last nosing or the upper floor's edge, a run's upper
/// end). `rails` names every requested rail whose body runs along the
/// stretch, each measured in the frame of one part: it overlaps the
/// stretch along that part's direction and lies within the request's reach
/// of its sides across it. A rail's heights are computed, so the evidence
/// is never exact.
#[derive(Clone, Debug, PartialEq)]
pub struct HandrailEvidence {
    request: HandrailRequest,
    parts: Vec<StretchPart>,
    pitch: (ElevationInterval, ElevationInterval),
    rails: Vec<(ObjectId, RailMeasurement)>,
    evidence: Evidence,
}

impl HandrailEvidence {
    /// The handrails answering `request` along one straight part: the
    /// horizontal `direction` and the walking surface's sides across it.
    /// Every rail must be requested, named once and measured in that part;
    /// they are kept in identity order.
    pub fn try_new(
        request: HandrailRequest,
        direction: MetricDirection,
        pitch: (ElevationInterval, ElevationInterval),
        walking_sides: (ElevationInterval, ElevationInterval),
        rails: Vec<(ObjectId, RailMeasurement)>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        let part = StretchPart::try_new(direction, walking_sides)?;
        Self::try_in_parts(request, vec![part], pitch, rails, evidence)
    }

    /// The handrails answering `request` along the straight `parts`, bottom
    /// to top, at least one. Every rail must be requested, named once and
    /// measured in one of the parts; they are kept in identity order.
    pub fn try_in_parts(
        request: HandrailRequest,
        parts: Vec<StretchPart>,
        pitch: (ElevationInterval, ElevationInterval),
        mut rails: Vec<(ObjectId, RailMeasurement)>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        if parts.is_empty() {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        if parts.len() == 1 {
            // Both ends lie along the one direction, so they are ordered.
            sides(pitch.0, pitch.1)?;
        }
        rails.sort_by(|a, b| a.0.cmp(&b.0));
        let unique = rails.windows(2).all(|pair| pair[0].0 != pair[1].0);
        let requested = rails.iter().all(|(rail, measurement)| {
            request.rails.binary_search(rail).is_ok() && measurement.part < parts.len()
        });
        if !unique || !requested {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        if evidence.exact || evidence.locator.trim().is_empty() {
            return Err(WalkingSurfaceError::InexactEvidence);
        }
        Ok(Self {
            request,
            parts,
            pitch,
            rails,
            evidence,
        })
    }

    /// The request this answers.
    #[must_use]
    pub fn request(&self) -> &HandrailRequest {
        &self.request
    }

    /// The stretch's straight parts, bottom to top.
    #[must_use]
    pub fn parts(&self) -> &[StretchPart] {
        &self.parts
    }

    /// The first part's horizontal walking direction: the stretch's, when
    /// it is one part.
    #[must_use]
    pub fn direction(&self) -> MetricDirection {
        self.first().direction
    }

    /// Where the pitch line starts, along the first part's direction, and
    /// where it ends, along the last part's.
    #[must_use]
    pub fn pitch(&self) -> (ElevationInterval, ElevationInterval) {
        self.pitch
    }

    /// Positions of the walking surface's sides across the first part's
    /// direction: the stretch's, when it is one part.
    #[must_use]
    pub fn sides(&self) -> (ElevationInterval, ElevationInterval) {
        self.first().sides
    }

    fn first(&self) -> &StretchPart {
        &self.parts[0]
    }

    fn part(&self, rail: &RailMeasurement) -> &StretchPart {
        &self.parts[rail.part]
    }

    /// The rails running along the stretch, in identity order.
    #[must_use]
    pub fn rails(&self) -> &[(ObjectId, RailMeasurement)] {
        &self.rails
    }

    /// How far a rail reaches beyond the bottom of the pitch line, along
    /// the first part's direction: negative where it starts above it.
    /// `None` for a rail measured along a later part, which does not run
    /// along the first.
    #[must_use]
    pub fn bottom_extension(&self, rail: &RailMeasurement) -> Option<MeasuredInterval> {
        (rail.part == 0).then(|| between(self.pitch.0, rail.start))
    }

    /// How far a rail reaches beyond the top of the pitch line, along the
    /// last part's direction; `None` for a rail measured along an earlier
    /// part.
    #[must_use]
    pub fn top_extension(&self, rail: &RailMeasurement) -> Option<MeasuredInterval> {
        (rail.part + 1 == self.parts.len()).then(|| between(rail.end, self.pitch.1))
    }

    /// The pieces of the handrail along `side`: every measured rail running
    /// along it ([`Self::side`]), bottom to top, part by part. Within a part,
    /// pieces are consecutive when each starts and ends decidably further
    /// along than the one before; two starting or ending where the
    /// positions cannot tell apart, or one lying within another's stretch
    /// (a second rail beside or below it), leave the order undecided, and
    /// the pieces are returned as `Err`, named. No rail along the side is an
    /// empty rail.
    pub fn side_rail(
        &self,
        side: RailSide,
    ) -> Result<Vec<&(ObjectId, RailMeasurement)>, Vec<ObjectId>> {
        let mut pieces: Vec<&(ObjectId, RailMeasurement)> = self
            .rails
            .iter()
            .filter(|(_, rail)| self.side(rail) == Some(side))
            .collect();
        pieces.sort_by(|a, b| {
            a.1.part
                .cmp(&b.1.part)
                .then_with(|| {
                    a.1.start
                        .lower_metres()
                        .total_cmp(&b.1.start.lower_metres())
                })
                .then_with(|| a.0.cmp(&b.0))
        });
        let ordered = pieces.windows(2).all(|pair| {
            let (lower, upper) = (&pair[0].1, &pair[1].1);
            lower.part < upper.part
                || (lower.start.upper_metres() < upper.start.lower_metres()
                    && lower.end.upper_metres() < upper.end.lower_metres())
        });
        if ordered {
            Ok(pieces)
        } else {
            Err(pieces.into_iter().map(|(rail, _)| rail.clone()).collect())
        }
    }

    /// The gap in plan between two rails: the least horizontal distance
    /// between the rectangles their bodies fill, each along its own part,
    /// zero where they touch or overlap, as an interval sure to hold it.
    /// `None` when a rail's positions leave no rectangle it surely fills,
    /// so no upper bound.
    #[must_use]
    pub fn gap(&self, a: &RailMeasurement, b: &RailMeasurement) -> Option<MeasuredInterval> {
        let (along_a, along_b) = (self.part(a).direction, self.part(b).direction);
        let (outer_a, inner_a) = (a.plan(along_a, true)?, a.plan(along_a, false));
        let (outer_b, inner_b) = (b.plan(along_b, true)?, b.plan(along_b, false));
        let (inner_a, inner_b) = (inner_a?, inner_b?);
        let scale = [&outer_a, &outer_b]
            .iter()
            .flat_map(|region| region.ring())
            .flatten()
            .fold(1.0_f64, |scale, value| scale.max(value.abs()));
        // Placing the corners rounds by a few units in the last place of
        // the positions, and so does the separation.
        let margin = 64.0 * f64::EPSILON * scale;
        let lower = (outer_a.separation(&outer_b) - margin).max(0.0);
        let upper = inner_a.separation(&inner_b).max(0.0) + margin;
        MeasuredInterval::try_new(lower, upper.max(lower)).ok()
    }

    /// The side a rail runs along: the one whose half of its part's walking
    /// surface holds it wholly across, `None` for a rail that may reach over
    /// the middle.
    #[must_use]
    pub fn side(&self, rail: &RailMeasurement) -> Option<RailSide> {
        let (left, right) = self.part(rail).sides;
        // The midpoint rounds once, by at most half a unit in the last
        // place, which the neighbouring value covers.
        let low = f64::midpoint(left.lower_metres(), right.lower_metres()).next_down();
        let high = f64::midpoint(left.upper_metres(), right.upper_metres()).next_up();
        if rail.right.upper_metres() < low {
            Some(RailSide::Right)
        } else if rail.left.lower_metres() > high {
            Some(RailSide::Left)
        } else {
            None
        }
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Measures stair flights, ramps and the headroom above them.
pub trait WalkingSurfaceService: Send + Sync + 'static {
    /// The treads, base and top of the request's object as a stair flight,
    /// walked along the requested line should it turn.
    fn measure_tread_flight(
        &self,
        request: &TreadFlightRequest,
    ) -> Result<TreadFlight, WalkingSurfaceError>;

    /// The sloped runs of `object` as a ramp.
    fn measure_sloped_runs(&self, object: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError>;

    /// The headroom above the request subject's walking surface.
    fn measure_headroom(&self, request: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError>;

    /// The landing at the requested end of a flight or run. The default
    /// refuses: a service that does not look for landings never answers
    /// that there is none.
    fn measure_landing(
        &self,
        request: &LandingRequest,
    ) -> Result<LandingEvidence, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "landings of {} are not measured by this service",
            request.subject()
        )))
    }

    /// The clearance below the request subject over the requested spaces'
    /// floors. The default refuses.
    fn measure_clearance_below(
        &self,
        request: &ClearanceBelowRequest,
    ) -> Result<ClearanceBelow, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "the clearance below {} is not measured by this service",
            request.subject()
        )))
    }

    /// The handrails along the requested stretch. The default refuses: a
    /// service that does not look for handrails never answers that there is
    /// none.
    fn measure_handrails(
        &self,
        request: &HandrailRequest,
    ) -> Result<HandrailEvidence, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "handrails along {} are not measured by this service",
            request.subject()
        )))
    }

    /// The clear width along the requested stretch. The default refuses: a
    /// service that does not measure clear widths never answers with the
    /// walking surface's own width.
    fn measure_clear_width(
        &self,
        request: &ClearWidthRequest,
    ) -> Result<ClearWidthEvidence, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "the clear width along {} is not measured by this service",
            request.subject()
        )))
    }

    /// The clear width of the requested landing. The default refuses: a
    /// service that does not measure landings' clear widths never answers
    /// with the landing's own width or that there is no landing.
    fn measure_landing_clear_width(
        &self,
        request: &LandingClearWidthRequest,
    ) -> Result<LandingClearWidthEvidence, WalkingSurfaceError> {
        Err(WalkingSurfaceError::Unsupported(format!(
            "the clear width of the landing at the end of {} is not measured by this service",
            request.subject()
        )))
    }
}

/// Registry handle for a [`WalkingSurfaceService`].
#[derive(Clone)]
pub struct WalkingSurfaceServiceHandle(Arc<dyn WalkingSurfaceService>);

impl WalkingSurfaceServiceHandle {
    /// Wraps a trusted walking-surface service.
    #[must_use]
    pub fn new(service: Arc<dyn WalkingSurfaceService>) -> Self {
        Self(service)
    }

    /// The flight answering `request`; an answer to another request is
    /// refused.
    pub fn measure_tread_flight(
        &self,
        request: &TreadFlightRequest,
    ) -> Result<TreadFlight, WalkingSurfaceError> {
        let flight = self.0.measure_tread_flight(request)?;
        if flight.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(flight)
    }

    /// The ramp of `object`; one naming another object is refused.
    pub fn measure_sloped_runs(
        &self,
        object: &ObjectId,
    ) -> Result<SlopedSurface, WalkingSurfaceError> {
        let ramp = self.0.measure_sloped_runs(object)?;
        if ramp.object() != object {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(ramp)
    }

    /// The headroom answering `request`; an answer to another request is
    /// refused.
    pub fn measure_headroom(
        &self,
        request: &HeadroomRequest,
    ) -> Result<Headroom, WalkingSurfaceError> {
        let headroom = self.0.measure_headroom(request)?;
        if headroom.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(headroom)
    }

    /// The landing answering `request`; an answer to another request is
    /// refused.
    pub fn measure_landing(
        &self,
        request: &LandingRequest,
    ) -> Result<LandingEvidence, WalkingSurfaceError> {
        let landing = self.0.measure_landing(request)?;
        if landing.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(landing)
    }

    /// The clearance below answering `request`; an answer to another request
    /// is refused.
    pub fn measure_clearance_below(
        &self,
        request: &ClearanceBelowRequest,
    ) -> Result<ClearanceBelow, WalkingSurfaceError> {
        let below = self.0.measure_clearance_below(request)?;
        if below.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(below)
    }

    /// The handrails answering `request`; an answer to another request is
    /// refused.
    pub fn measure_handrails(
        &self,
        request: &HandrailRequest,
    ) -> Result<HandrailEvidence, WalkingSurfaceError> {
        let rails = self.0.measure_handrails(request)?;
        if rails.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(rails)
    }

    /// The clear width answering `request`; an answer to another request is
    /// refused.
    pub fn measure_clear_width(
        &self,
        request: &ClearWidthRequest,
    ) -> Result<ClearWidthEvidence, WalkingSurfaceError> {
        let width = self.0.measure_clear_width(request)?;
        if width.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(width)
    }

    /// The landing clear width answering `request`; an answer to another
    /// request is refused.
    pub fn measure_landing_clear_width(
        &self,
        request: &LandingClearWidthRequest,
    ) -> Result<LandingClearWidthEvidence, WalkingSurfaceError> {
        let width = self.0.measure_landing_clear_width(request)?;
        if width.request() != request {
            return Err(WalkingSurfaceError::InvalidMeasurement);
        }
        Ok(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn source() -> SourceId {
        SourceId::new("cad", "m").unwrap()
    }

    fn id(local: &str) -> ObjectId {
        ObjectId::new(source(), local).unwrap()
    }

    fn point(value: f64) -> ElevationInterval {
        ElevationInterval::exact(value).unwrap()
    }

    fn request(local: &str) -> TreadFlightRequest {
        TreadFlightRequest::new(id(local))
    }

    fn x() -> MetricDirection {
        MetricDirection::try_new([1.0, 0.0, 0.0]).unwrap()
    }

    fn tread(z: f64, front: f64, back: f64) -> Tread {
        Tread::try_new(point(z), point(front), point(back)).unwrap()
    }

    fn evidence(exact: bool) -> Evidence {
        Evidence {
            source: source(),
            locator: "tread-flight:a".into(),
            exact,
        }
    }

    /// Whether `interval` holds the decimal `value`, up to the binary
    /// rounding of the decimal positions it was computed from.
    fn contains(interval: MeasuredInterval, value: f64) -> bool {
        interval.lower() - 1e-15 <= value && value <= interval.upper() + 1e-15
    }

    #[test]
    fn risers_goings_and_nosings_come_from_the_positions() {
        let treads = vec![
            tread(0.17, 0.0, 0.30),
            tread(0.34, 0.28, 0.58),
            tread(0.55, 0.56, 0.86),
        ];
        let flight = TreadFlight::try_new(
            request("a"),
            WalkingLine::Straight(x()),
            point(0.0),
            point(0.72),
            treads,
            evidence(true),
        )
        .unwrap();
        assert!(flight.ends_in_riser());
        let risers = flight.risers();
        assert_eq!(risers.len(), 4);
        // One closure per riser, the final one as the service states it.
        let closures = flight
            .clone()
            .with_final_riser(RiserClosure::Open)
            .riser_closures();
        assert_eq!(closures.len(), 4);
        assert_eq!(closures[3], RiserClosure::Open);
        assert_eq!(flight.riser_closures()[3], RiserClosure::NotMeasured);
        for (riser, expected) in risers.iter().zip([0.17, 0.17, 0.21, 0.17]) {
            assert!(contains(*riser, expected), "{riser:?} {expected}");
            assert!(riser.upper() - riser.lower() <= 2.0 * f64::EPSILON);
        }
        let goings = flight.goings();
        assert_eq!(goings.len(), 2);
        assert!(goings.iter().all(|going| contains(*going, 0.28)));
        assert!(
            flight
                .nosings()
                .iter()
                .all(|nosing| contains(*nosing, 0.02))
        );
        assert!(contains(flight.rise(), 0.72));
        assert!(contains(flight.treads()[0].depth(), 0.30));
    }

    #[test]
    fn a_flight_whose_top_is_its_last_tread_has_no_final_riser() {
        let treads = vec![tread(0.2, 0.0, 0.3), tread(0.4, 0.3, 0.6)];
        let flight = TreadFlight::try_new(
            request("a"),
            WalkingLine::Straight(x()),
            point(0.0),
            point(0.4),
            treads,
            evidence(true),
        )
        .unwrap();
        assert!(!flight.ends_in_riser());
        assert_eq!(flight.risers().len(), 2);
        assert!(contains(flight.rise(), 0.4));
    }

    #[test]
    fn incoherent_flights_are_refused() {
        let ordered = || vec![tread(0.2, 0.0, 0.3), tread(0.4, 0.3, 0.6)];
        // Treads out of order.
        let reversed = vec![tread(0.4, 0.3, 0.6), tread(0.2, 0.0, 0.3)];
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.4),
                reversed,
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        // A base above the first tread, and a top below the last.
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.3),
                point(0.4),
                ordered(),
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.3),
                ordered(),
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        // No treads, or a sloped direction.
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.4),
                vec![],
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        let sloped = MetricDirection::try_new([1.0, 0.0, 1.0]).unwrap();
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(sloped),
                point(0.0),
                point(0.4),
                ordered(),
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        // Exactness must match the positions.
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.4),
                ordered(),
                evidence(false)
            ),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        let widened = ElevationInterval::try_new(0.39, 0.41).unwrap();
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.0),
                widened,
                ordered(),
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
    }

    #[test]
    fn a_run_slope_is_its_rise_over_its_length() {
        let run = SlopedRun::try_new(x(), point(0.0), point(0.5), point(1.0), point(7.0)).unwrap();
        assert!(contains(run.slope(), 0.5 / 6.0));
        assert!(run.slope().upper() - run.slope().lower() <= 4.0 * f64::EPSILON);
        assert!(contains(run.length(), 6.0) && contains(run.rise(), 0.5));
        assert_eq!(
            SlopedRun::try_new(x(), point(0.5), point(0.5), point(1.0), point(7.0)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        let ramp = SlopedSurface::try_new(id("a"), vec![run], evidence(true)).unwrap();
        assert!(ramp.is_exact());
        assert_eq!(
            SlopedSurface::try_new(id("a"), vec![], evidence(true)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
    }

    #[test]
    fn headroom_names_requested_obstacles_only() {
        let request = HeadroomRequest::new(id("a"), [id("c"), id("a"), id("b"), id("c")]);
        assert_eq!(request.obstacles(), &[id("b"), id("c")]);
        let clearance = MeasuredInterval::try_new(2.0, 2.0 + 1e-9).ok();
        assert!(
            Headroom::try_new(request.clone(), clearance, vec![id("b")], evidence(false)).is_ok()
        );
        assert_eq!(
            Headroom::try_new(request.clone(), clearance, vec![id("d")], evidence(false)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            Headroom::try_new(request.clone(), clearance, vec![], evidence(false)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            Headroom::try_new(request.clone(), clearance, vec![id("b")], evidence(true)),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        assert!(Headroom::try_new(request, None, vec![], evidence(false)).is_ok());
    }

    struct Other;
    impl WalkingSurfaceService for Other {
        fn measure_tread_flight(
            &self,
            _: &TreadFlightRequest,
        ) -> Result<TreadFlight, WalkingSurfaceError> {
            TreadFlight::try_new(
                request("b"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.2),
                vec![tread(0.2, 0.0, 0.3)],
                evidence(true),
            )
        }
        fn measure_sloped_runs(&self, _: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError> {
            let run = SlopedRun::try_new(x(), point(0.0), point(0.5), point(0.0), point(6.0))?;
            SlopedSurface::try_new(id("b"), vec![run], evidence(true))
        }
        fn measure_headroom(&self, _: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError> {
            Headroom::try_new(
                HeadroomRequest::new(id("b"), []),
                None,
                vec![],
                evidence(false),
            )
        }
    }

    #[test]
    fn answers_about_another_object_are_refused() {
        let handle = WalkingSurfaceServiceHandle::new(Arc::new(Other));
        assert_eq!(
            handle.measure_tread_flight(&request("a")),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            handle.measure_sloped_runs(&id("a")),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            handle.measure_headroom(&HeadroomRequest::new(id("a"), [])),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert!(handle.measure_tread_flight(&request("b")).is_ok());
        // Landings and the clearance below are refused by default, never
        // answered empty.
        assert!(matches!(
            handle.measure_landing(&LandingRequest::new(id("b"), WalkingEnd::FlightTop, [])),
            Err(WalkingSurfaceError::Unsupported(_))
        ));
        assert!(matches!(
            handle.measure_clearance_below(&ClearanceBelowRequest::new(id("b"), [])),
            Err(WalkingSurfaceError::Unsupported(_))
        ));
        let rails =
            HandrailRequest::try_new(id("b"), WalkingStretch::Flight, [], (0.1, 1.5), 0.3).unwrap();
        assert!(matches!(
            handle.measure_handrails(&rails),
            Err(WalkingSurfaceError::Unsupported(_))
        ));
    }

    fn rail(left: f64, right: f64, lowest: f64) -> RailMeasurement {
        RailMeasurement::try_new(
            (point(-0.3), point(1.5)),
            (point(left), point(right)),
            MeasuredInterval::try_new(lowest, lowest + 1e-9).unwrap(),
            MeasuredInterval::try_new(lowest + 0.01, lowest + 0.01 + 1e-9).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn handrails_are_requested_rails_on_a_side_with_extensions() {
        let request = HandrailRequest::try_new(
            id("a"),
            WalkingStretch::Flight,
            [id("r"), id("a"), id("l"), id("r")],
            (0.2, 1.5),
            0.3,
        )
        .unwrap();
        assert_eq!(request.rails(), &[id("l"), id("r")]);
        assert!(
            HandrailRequest::try_new(id("a"), WalkingStretch::Flight, [], (-0.1, 1.5), 0.0)
                .is_err()
        );
        let evidence = |exact: bool| Evidence {
            source: source(),
            locator: "handrails:a".into(),
            exact,
        };
        let measure = |rails: Vec<(ObjectId, RailMeasurement)>, exact: bool| {
            HandrailEvidence::try_new(
                request.clone(),
                x(),
                (point(0.0), point(1.12)),
                (point(0.0), point(1.2)),
                rails,
                evidence(exact),
            )
        };
        let rails = vec![
            (id("r"), rail(-0.05, 0.0, 0.9)),
            (id("l"), rail(1.2, 1.25, 0.85)),
        ];
        let measured = measure(rails.clone(), false).unwrap();
        assert_eq!(measured.rails()[0].0, id("l"));
        let (left, right) = (measured.rails()[0].1, measured.rails()[1].1);
        assert_eq!(measured.side(&left), Some(RailSide::Left));
        assert_eq!(measured.side(&right), Some(RailSide::Right));
        assert_eq!(measured.side(&rail(0.5, 0.7, 0.9)), None);
        assert!(contains(measured.bottom_extension(&left).unwrap(), 0.3));
        assert!(contains(measured.top_extension(&left).unwrap(), 0.38));
        // Exact evidence, an unrequested rail and one named twice are
        // refused.
        assert_eq!(
            measure(rails, true),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        for rails in [
            vec![(id("x"), rail(0.0, 0.1, 0.9))],
            vec![
                (id("l"), rail(0.0, 0.1, 0.9)),
                (id("l"), rail(0.0, 0.1, 0.9)),
            ],
        ] {
            assert_eq!(
                measure(rails, false),
                Err(WalkingSurfaceError::InvalidMeasurement)
            );
        }
        // A negative rise or unordered heights are refused.
        let rise = MeasuredInterval::try_new(-0.1, 0.0).ok();
        assert!(rail(0.0, 0.1, 0.9).with_rises(rise, None).is_err());
        assert!(
            RailMeasurement::try_new(
                (point(0.0), point(1.0)),
                (point(0.0), point(0.1)),
                MeasuredInterval::try_new(1.0, 1.0).unwrap(),
                MeasuredInterval::try_new(0.9, 0.9).unwrap(),
            )
            .is_err()
        );
    }

    fn piece(start: f64, end: f64, left: f64, right: f64) -> RailMeasurement {
        RailMeasurement::try_new(
            (point(start), point(end)),
            (point(left), point(right)),
            MeasuredInterval::try_new(0.9, 0.9 + 1e-9).unwrap(),
            MeasuredInterval::try_new(0.9, 0.9 + 1e-9).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn the_pieces_along_a_side_are_ordered_with_their_gaps() {
        let request = HandrailRequest::try_new(
            id("a"),
            WalkingStretch::Flight,
            [id("p"), id("q"), id("r"), id("s"), id("m")],
            (0.2, 1.5),
            0.3,
        )
        .unwrap();
        let measure = |rails: Vec<(ObjectId, RailMeasurement)>| {
            HandrailEvidence::try_new(
                request.clone(),
                x(),
                (point(0.0), point(0.84)),
                (point(0.0), point(1.2)),
                rails,
                evidence(false),
            )
            .unwrap()
        };
        // Left: `q` then `p` with a 0.1 m gap; right: `r` then `s` meeting
        // end to end; `m` reaches over the middle.
        let measured = measure(vec![
            (id("p"), piece(0.5, 1.14, 1.25, 1.3)),
            (id("q"), piece(-0.3, 0.4, 1.25, 1.3)),
            (id("r"), piece(-0.3, 0.4, -0.1, -0.05)),
            (id("s"), piece(0.4, 1.14, -0.1, -0.05)),
            (id("m"), piece(-0.3, 1.14, 0.55, 0.65)),
        ]);
        let left = measured.side_rail(RailSide::Left).unwrap();
        let names: Vec<&ObjectId> = left.iter().map(|(rail, _)| rail).collect();
        assert_eq!(names, [&id("q"), &id("p")]);
        let gap = measured.gap(&left[0].1, &left[1].1).unwrap();
        assert!(
            contains(gap, 0.1) && gap.upper() - gap.lower() < 1e-12,
            "{gap:?}"
        );
        let right = measured.side_rail(RailSide::Right).unwrap();
        let gap = measured.gap(&right[0].1, &right[1].1).unwrap();
        assert!(gap.lower() == 0.0 && gap.upper() < 1e-12, "{gap:?}");
        // Offset across, pieces are apart diagonally.
        let apart = measured
            .gap(&piece(-0.3, 0.4, 1.25, 1.3), &piece(0.7, 1.14, 1.6, 1.65))
            .unwrap();
        assert!(contains(apart, 0.3_f64.hypot(0.3)), "{apart:?}");
        // A second rail within the first's stretch leaves the order open.
        let nested = measure(vec![
            (id("p"), piece(-0.3, 1.14, 1.25, 1.3)),
            (id("q"), piece(0.0, 0.84, 1.3, 1.35)),
        ]);
        assert_eq!(
            nested.side_rail(RailSide::Left),
            Err(vec![id("p"), id("q")])
        );
        assert_eq!(nested.side_rail(RailSide::Right), Ok(vec![]));
        // A piece too uncertain to surely fill a rectangle has no upper
        // bound on its gap.
        let blurred = RailMeasurement::try_new(
            (around(0.5, 0.2), around(0.6, 0.2)),
            (point(1.25), point(1.3)),
            MeasuredInterval::try_new(0.9, 0.9).unwrap(),
            MeasuredInterval::try_new(0.9, 0.9).unwrap(),
        )
        .unwrap();
        assert_eq!(measured.gap(&left[0].1, &blurred), None);
    }

    #[test]
    fn a_turning_flights_rails_are_measured_part_by_part() {
        // A quarter turn: along +x over y 0 .. 1, then along +y over x
        // 0.84 .. 1.84, whose sides across +y (towards -x) lie at -1.84 and
        // -0.84. The outer rail is two pieces meeting at the corner.
        let y = MetricDirection::try_new([0.0, 1.0, 0.0]).unwrap();
        let parts = vec![
            StretchPart::try_new(x(), (point(0.0), point(1.0))).unwrap(),
            StretchPart::try_new(y, (point(-1.84), point(-0.84))).unwrap(),
        ];
        let request = HandrailRequest::try_new(
            id("a"),
            WalkingStretch::Flight,
            [id("p"), id("q"), id("r")],
            (0.2, 1.5),
            0.3,
        )
        .unwrap();
        let measure = |rails: Vec<(ObjectId, RailMeasurement)>| {
            HandrailEvidence::try_in_parts(
                request.clone(),
                parts.clone(),
                (point(0.0), point(1.56)),
                rails,
                evidence(false),
            )
        };
        // `p` along +x at y -0.1 .. -0.05 up to the corner, `q` along +y at
        // x 1.84 .. 1.89 from it, `r` along the inner side of the top part.
        let measured = measure(vec![
            (id("p"), piece(-0.3, 1.89, -0.1, -0.05)),
            (id("q"), piece(-0.1, 1.86, -1.89, -1.84).in_part(1)),
            (id("r"), piece(1.0, 1.86, -0.84, -0.79).in_part(1)),
        ])
        .unwrap();
        assert_eq!(measured.parts().len(), 2);
        let (p, q, r) = (
            &measured.rails()[0].1,
            &measured.rails()[1].1,
            &measured.rails()[2].1,
        );
        assert_eq!(measured.side(p), Some(RailSide::Right));
        assert_eq!(measured.side(q), Some(RailSide::Right));
        assert_eq!(measured.side(r), Some(RailSide::Left));
        let outer = measured.side_rail(RailSide::Right).unwrap();
        let names: Vec<&ObjectId> = outer.iter().map(|(rail, _)| rail).collect();
        assert_eq!(names, [&id("p"), &id("q")]);
        // They meet at the corner.
        let gap = measured.gap(p, q).unwrap();
        assert!(gap.lower() == 0.0 && gap.upper() < 1e-12, "{gap:?}");
        // The first piece reaches beyond the bottom, the last beyond the
        // top; neither is measured beyond the other end.
        assert!(contains(measured.bottom_extension(p).unwrap(), 0.3));
        assert_eq!(measured.top_extension(p), None);
        assert!(contains(measured.top_extension(q).unwrap(), 0.3));
        assert_eq!(measured.bottom_extension(q), None);
        // A rail in a part the stretch does not have is refused.
        assert_eq!(
            measure(vec![(id("p"), piece(0.0, 1.0, -0.1, -0.05).in_part(2))]),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            HandrailEvidence::try_in_parts(
                request.clone(),
                vec![],
                (point(0.0), point(1.56)),
                vec![],
                evidence(false)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        let sloped = MetricDirection::try_new([1.0, 0.0, 1.0]).unwrap();
        assert!(StretchPart::try_new(sloped, (point(0.0), point(1.0))).is_err());
    }

    fn around(value: f64, margin: f64) -> ElevationInterval {
        ElevationInterval::try_new(value - margin, value + margin).unwrap()
    }

    #[test]
    fn widths_come_from_the_sides_and_the_narrowest_tread_governs() {
        let sided = |z: f64, front: f64, right: f64| {
            tread(z, front, front + 0.3)
                .with_sides(point(0.0), point(right))
                .unwrap()
        };
        let treads = vec![sided(0.2, 0.0, 1.2), sided(0.4, 0.3, 1.1)];
        let flight = TreadFlight::try_new(
            request("a"),
            WalkingLine::Straight(x()),
            point(0.0),
            point(0.4),
            treads,
            evidence(true),
        )
        .unwrap();
        assert!(contains(flight.width().unwrap(), 1.1));
        // One tread without sides leaves the flight's width unknown.
        let treads = vec![sided(0.2, 0.0, 1.2), tread(0.4, 0.3, 0.6)];
        let flight = TreadFlight::try_new(
            request("a"),
            WalkingLine::Straight(x()),
            point(0.0),
            point(0.4),
            treads,
            evidence(true),
        )
        .unwrap();
        assert_eq!(flight.width(), None);
        // Sides out of order, and widened sides reported as exact.
        assert_eq!(
            tread(0.2, 0.0, 0.3).with_sides(point(1.0), point(0.0)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        let widened = tread(0.2, 0.0, 0.3)
            .with_sides(point(0.0), ElevationInterval::try_new(1.0, 1.1).unwrap())
            .unwrap();
        assert!(!widened.is_exact());
        assert_eq!(
            TreadFlight::try_new(
                request("a"),
                WalkingLine::Straight(x()),
                point(0.0),
                point(0.2),
                vec![widened],
                evidence(true)
            ),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        let run = SlopedRun::try_new(x(), point(0.0), point(0.5), point(1.0), point(7.0))
            .unwrap()
            .with_sides(point(-0.75), point(0.75))
            .unwrap();
        assert!(contains(run.width().unwrap(), 1.5) && run.is_exact());
        let [ax, ay, _] = across(x()).components();
        assert!(ax.abs() < f64::EPSILON && (ay - 1.0).abs() < f64::EPSILON);
    }

    fn landing_evidence(
        request: &LandingRequest,
        landing: Option<Landing>,
        exact: bool,
    ) -> Result<LandingEvidence, WalkingSurfaceError> {
        LandingEvidence::try_new(
            request.clone(),
            x(),
            point(1.0),
            landing,
            Evidence {
                source: source(),
                locator: "landing:a".into(),
                exact,
            },
        )
    }

    #[test]
    fn a_landing_is_carried_by_a_requested_object_beyond_the_edge() {
        let request = LandingRequest::new(id("a"), WalkingEnd::FlightTop, [id("c"), id("a")]);
        assert_eq!(request.candidates(), &[id("c")]);
        let extent = LandingExtent::try_new(point(2.5), point(-0.1), point(1.4)).unwrap();
        let found =
            landing_evidence(&request, Some(Landing::new(id("c"), Some(extent))), true).unwrap();
        assert!(contains(found.depth().unwrap(), 1.5));
        assert!(contains(found.width().unwrap(), 1.5));
        // The subject may carry its own landing (a ramp's).
        assert!(
            landing_evidence(&request, Some(Landing::new(id("a"), Some(extent))), true).is_ok()
        );
        // An unrequested carrier, a far side short of the edge, and
        // exactness that does not match the positions are refused.
        assert_eq!(
            landing_evidence(&request, Some(Landing::new(id("d"), None)), true),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        let short = LandingExtent::try_new(point(0.5), point(0.0), point(1.0)).unwrap();
        assert_eq!(
            landing_evidence(&request, Some(Landing::new(id("c"), Some(short))), true),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            landing_evidence(&request, None, false),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        assert_eq!(
            LandingExtent::try_new(point(2.0), point(1.0), point(0.0)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        // Found but not a rectangle: no size.
        let unmeasured =
            landing_evidence(&request, Some(Landing::new(id("c"), None)), true).unwrap();
        assert_eq!(unmeasured.depth(), None);
        assert_eq!(unmeasured.width(), None);
    }

    #[test]
    fn the_clearance_below_names_requested_spaces_only() {
        let request = ClearanceBelowRequest::new(id("a"), [id("s"), id("a"), id("s")]);
        assert_eq!(request.spaces(), &[id("s")]);
        let clearance = MeasuredInterval::try_new(1.5, 1.5 + 1e-9).ok();
        assert!(
            ClearanceBelow::try_new(request.clone(), clearance, vec![id("s")], evidence(false))
                .is_ok()
        );
        assert_eq!(
            ClearanceBelow::try_new(request.clone(), clearance, vec![id("t")], evidence(false)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            ClearanceBelow::try_new(request.clone(), clearance, vec![id("s")], evidence(true)),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        assert!(ClearanceBelow::try_new(request, None, vec![], evidence(false)).is_ok());
    }

    #[test]
    fn winder_angles_and_widths_come_from_nosings_and_sides() {
        // A quarter turn over three winders: nosings at 0°, 30°, 60° and
        // 90° about the inner corner at the origin.
        let nosing = |degrees: f64| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            PlanSegment::try_new([0.0, 0.0], [cos, sin], 0.0).unwrap()
        };
        let treads: Vec<Tread> = [0.0, 30.0, 60.0, 90.0]
            .iter()
            .enumerate()
            .map(|(step, degrees)| {
                #[allow(clippy::cast_precision_loss)]
                let step = step as f64;
                tread(0.18 * (step + 1.0), 0.3 * step, 0.3 * step + 0.3)
                    .with_nosing(nosing(*degrees))
                    .with_sides(point(-0.4), point(0.5))
                    .unwrap()
                    .with_riser_below(RiserClosure::Closed)
            })
            .collect();
        let line = WalkingLine::Turning(vec![[0.5, 0.1], [0.4, 0.3], [0.3, 0.4], [0.1, 0.5]]);
        let flight = TreadFlight::try_new(
            request("a"),
            line.clone(),
            point(0.0),
            point(0.72),
            treads,
            evidence(false),
        )
        .unwrap_err();
        // Exact positions and nosings make exact evidence.
        assert_eq!(flight, WalkingSurfaceError::InexactEvidence);
        let treads: Vec<Tread> = [0.0, 30.0, 60.0, 90.0]
            .iter()
            .enumerate()
            .map(|(step, degrees)| {
                #[allow(clippy::cast_precision_loss)]
                let step = step as f64;
                let tread = tread(0.18 * (step + 1.0), 0.3 * step, 0.3 * step + 0.3)
                    .with_nosing(nosing(*degrees));
                // Only the first tread fills a rectangle; the winders taper.
                if step == 0.0 {
                    tread.with_sides(point(-0.4), point(0.5)).unwrap()
                } else {
                    tread
                }
            })
            .collect();
        let flight = TreadFlight::try_new(
            request("a"),
            line,
            point(0.0),
            point(0.72),
            treads,
            evidence(true),
        )
        .unwrap();
        assert!(flight.walking_line().is_turning());
        let angles = flight.winder_angles();
        assert_eq!(angles.len(), 3);
        for angle in angles {
            let angle = angle.unwrap();
            assert!(contains(angle, 30.0_f64.to_radians()), "{angle:?}");
            assert!(angle.upper() - angle.lower() < 1e-12);
        }
        let width = flight.treads()[0].width().unwrap();
        assert!(contains(width, 0.9));
        // A winder has no sides, so the flight has no width.
        assert_eq!(flight.treads()[1].width(), None);
        assert_eq!(flight.width(), None);
        assert_eq!(flight.treads()[0].riser_below(), RiserClosure::NotMeasured);
    }

    #[test]
    fn an_uncertain_nosing_widens_its_angle_and_a_short_one_has_none() {
        let a = PlanSegment::try_new([0.0, 0.0], [1.0, 0.0], 0.001).unwrap();
        let b = PlanSegment::try_new([0.0, 0.0], [0.0, 1.0], 0.001).unwrap();
        let square = a.angle_to(&b).unwrap();
        assert!(square.upper() <= std::f64::consts::FRAC_PI_2);
        assert!(square.lower() < std::f64::consts::FRAC_PI_2 - 0.0039);
        let parallel = a.angle_to(&a).unwrap();
        assert!(parallel.lower() == 0.0 && parallel.upper() > 0.0039);
        let short = PlanSegment::try_new([0.0, 0.0], [0.001, 0.0], 0.001).unwrap();
        assert_eq!(short.angle_to(&a), None);
        assert_eq!(
            PlanSegment::try_new([0.0, 0.0], [0.0, 0.0], 0.0),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            PlanSegment::try_new([0.0, 0.0], [1.0, 0.0], -1.0),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
    }

    #[test]
    fn clear_widths_name_requested_obstacles_and_a_band_above_the_pitch_line() {
        let request = ClearWidthRequest::try_new(
            id("f"),
            WalkingStretch::Flight,
            [id("rail"), id("f"), id("rail")],
            (0.5, 1.5),
        )
        .unwrap();
        assert_eq!(request.obstacles(), [id("rail")]);
        assert_eq!(request.band(), (0.5, 1.5));
        for band in [(1.5, 0.5), (-0.1, 1.0), (0.5, f64::NAN)] {
            assert!(ClearWidthRequest::try_new(id("f"), WalkingStretch::Flight, [], band).is_err());
        }
        let width = MeasuredInterval::try_new(0.99, 1.01).unwrap();
        let measured =
            ClearWidthEvidence::try_new(request.clone(), width, vec![id("rail")], evidence(false))
                .unwrap();
        assert_eq!(measured.governing(), [id("rail")]);
        assert_eq!(
            ClearWidthEvidence::try_new(request.clone(), width, vec![id("wall")], evidence(false)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            ClearWidthEvidence::try_new(request, width, vec![], evidence(true)),
            Err(WalkingSurfaceError::InexactEvidence)
        );
    }

    #[test]
    fn landing_clear_widths_name_a_carrier_and_requested_bounds() {
        let landing = LandingRequest::new(id("f"), WalkingEnd::FlightTop, [id("slab")]);
        let request = LandingClearWidthRequest::try_new(
            landing.clone(),
            [id("wall"), id("f"), id("rail"), id("wall")],
            (0.5, 1.5),
        )
        .unwrap();
        assert_eq!(request.obstacles(), [id("rail"), id("wall")]);
        assert_eq!(request.subject(), &id("f"));
        for band in [(1.5, 0.5), (-0.1, 1.0), (0.5, f64::INFINITY)] {
            assert!(LandingClearWidthRequest::try_new(landing.clone(), [], band).is_err());
        }
        let width = MeasuredInterval::try_new(0.99, 1.01).unwrap();
        let measured = |carrier: &str, bounds: (Vec<ObjectId>, Vec<ObjectId>), exact: bool| {
            LandingClearWidthEvidence::try_new(
                request.clone(),
                Some(LandingClearWidth::new(
                    id(carrier),
                    width,
                    vec![id("wall")],
                    bounds,
                )),
                evidence(exact),
            )
        };
        let found = measured(
            "slab",
            (vec![id("wall"), id("wall")], vec![id("rail")]),
            false,
        )
        .unwrap();
        let landing = found.landing().unwrap();
        assert_eq!(landing.bounds(), (&[id("wall")][..], &[id("rail")][..]));
        assert_eq!(landing.governing(), [id("wall")]);
        // The carrier is a candidate or the subject, every bound requested.
        assert!(measured("f", (vec![], vec![]), false).is_ok());
        assert_eq!(
            measured("floor", (vec![], vec![]), false),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            measured("slab", (vec![id("door")], vec![]), false),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            measured("slab", (vec![], vec![]), true),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        // No landing is an answer of its own.
        let none = LandingClearWidthEvidence::try_new(request, None, evidence(true)).unwrap();
        assert!(none.landing().is_none());
    }

    #[test]
    fn walking_lines_and_offsets_are_checked() {
        assert!(TreadFlightRequest::from_inner_side(id("a"), 0.0).is_err());
        assert!(TreadFlightRequest::from_inner_side(id("a"), f64::NAN).is_err());
        let request = TreadFlightRequest::from_inner_side(id("a"), 0.4).unwrap();
        assert_eq!(
            request.walking_line(),
            WalkingLinePlacement::FromInnerSide(0.4)
        );
        let treads = || vec![tread(0.2, 0.0, 0.3), tread(0.4, 0.3, 0.6)];
        for line in [
            WalkingLine::Turning(vec![[0.0, 0.0]]),
            WalkingLine::Turning(vec![[0.0, 0.0], [0.0, 0.0]]),
            WalkingLine::Turning(vec![[0.0, 0.0], [f64::INFINITY, 0.0]]),
        ] {
            assert_eq!(
                TreadFlight::try_new(
                    request.clone(),
                    line,
                    point(0.0),
                    point(0.4),
                    treads(),
                    evidence(true)
                ),
                Err(WalkingSurfaceError::InvalidMeasurement)
            );
        }
        assert!(
            tread(0.2, 0.0, 0.3)
                .with_sides(point(1.0), point(0.0))
                .is_err()
        );
    }
}
