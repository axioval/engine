//! Stair flights and ramps: treads, risers, goings, sloped runs and the
//! headroom above a walking surface.
//!
//! ADR 0004: this seam measures. Whether a riser is too high, a flight too
//! irregular or a ramp too steep for its length is a rule's judgement over
//! these measurements.
//!
//! A service reports **positions**, never the derived lengths: tread
//! elevations, the positions of each tread's front and back edge along the
//! flight's walking direction, a run's lowest and highest points and its
//! extent along its own ascending direction. Risers, goings, tread depths,
//! nosings, rises, run lengths and slopes are computed here from those
//! positions as intervals sure to hold the exact value, so no adapter can
//! report a riser that disagrees with its treads. A position is an interval;
//! the evidence is exact exactly when every position is a point.
//!
//! Widths come the same way: a tread or a run may report the positions of
//! its two sides across the walking direction ([`across`]), stated only
//! where it fills the rectangle between its ends and those sides. A landing
//! ([`LandingEvidence`]) is the level surface a request's candidate carries
//! at one end of a flight or run, reported as the positions of its far side
//! and its sides along the direction leaving that end; its depth is taken
//! from the end's arrival line. The clearance below a subject
//! ([`ClearanceBelow`]) is its height above the floors of the spaces a
//! request names.
//!
//! What the seam does not measure yet (Refs #85): winders and turning
//! flights, open risers, handrails and doors on landings. A service refuses
//! a shape it cannot decide rather than approximate it.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::{ElevationInterval, MetricDirection};

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

/// One tread: an upward-facing horizontal face of a flight.
///
/// `front` and `back` are the positions of its nearest and farthest points
/// along the flight's walking direction; the front edge is the nosing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tread {
    elevation: ElevationInterval,
    front: ElevationInterval,
    back: ElevationInterval,
    sides: Option<Sides>,
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
    /// direction.
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
        })
    }

    /// The tread with the positions of its sides along [`across`] the
    /// walking direction. A service states them only where the tread fills
    /// the rectangle between its front, back and sides, so the width holds
    /// all along its depth.
    pub fn with_sides(
        mut self,
        left: ElevationInterval,
        right: ElevationInterval,
    ) -> Result<Self, WalkingSurfaceError> {
        self.sides = Some(sides(left, right)?);
        Ok(self)
    }

    /// The positions of the tread's sides across the walking direction, when
    /// measured.
    #[must_use]
    pub fn sides(&self) -> Option<(ElevationInterval, ElevationInterval)> {
        self.sides
    }

    /// The tread's width across the walking direction, when its sides were
    /// measured.
    #[must_use]
    pub fn width(&self) -> Option<MeasuredInterval> {
        self.sides.map(|(left, right)| between(right, left))
    }

    /// Elevation of the tread's surface.
    #[must_use]
    pub fn elevation(&self) -> ElevationInterval {
        self.elevation
    }

    /// Position of the front edge (the nosing) along the walking direction.
    #[must_use]
    pub fn front(&self) -> ElevationInterval {
        self.front
    }

    /// Position of the back edge along the walking direction.
    #[must_use]
    pub fn back(&self) -> ElevationInterval {
        self.back
    }

    /// The tread's depth along the walking direction, back less front.
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
    }
}

/// A straight stair flight measured from its body.
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
    object: ObjectId,
    direction: MetricDirection,
    base: ElevationInterval,
    top: ElevationInterval,
    treads: Vec<Tread>,
    ends_in_riser: bool,
    evidence: Evidence,
}

impl TreadFlight {
    /// A flight of `object` climbing along the horizontal `direction`.
    ///
    /// Treads must be given bottom to top, strictly ascending in elevation
    /// and in front position; the base must lie below the first tread and
    /// the top at or above the last, decidably. The evidence is exact
    /// exactly when every position is a point.
    pub fn try_new(
        object: ObjectId,
        direction: MetricDirection,
        base: ElevationInterval,
        top: ElevationInterval,
        treads: Vec<Tread>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        #[allow(clippy::float_cmp)]
        if direction.components()[2] != 0.0 {
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
            object,
            direction,
            base,
            top,
            treads,
            ends_in_riser,
            evidence,
        })
    }

    /// The measured object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The horizontal direction the flight climbs along.
    #[must_use]
    pub fn direction(&self) -> MetricDirection {
        self.direction
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

    /// Goings, bottom to top: the horizontal distance along the walking
    /// direction from each tread's nosing to the next one's. A flight of
    /// `n` treads has `n - 1` goings.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WalkingEnd {
    /// Where a straight flight starts, in front of its first riser.
    FlightBottom,
    /// Where a straight flight arrives, beyond its last riser.
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

/// Measures stair flights, ramps and the headroom above them.
pub trait WalkingSurfaceService: Send + Sync + 'static {
    /// The treads, base and top of `object` as a straight stair flight.
    fn measure_tread_flight(&self, object: &ObjectId) -> Result<TreadFlight, WalkingSurfaceError>;

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

    /// The flight of `object`; one naming another object is refused.
    pub fn measure_tread_flight(
        &self,
        object: &ObjectId,
    ) -> Result<TreadFlight, WalkingSurfaceError> {
        let flight = self.0.measure_tread_flight(object)?;
        if flight.object() != object {
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
            id("a"),
            x(),
            point(0.0),
            point(0.72),
            treads,
            evidence(true),
        )
        .unwrap();
        assert!(flight.ends_in_riser());
        let risers = flight.risers();
        assert_eq!(risers.len(), 4);
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
        let flight =
            TreadFlight::try_new(id("a"), x(), point(0.0), point(0.4), treads, evidence(true))
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
                id("a"),
                x(),
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
                id("a"),
                x(),
                point(0.3),
                point(0.4),
                ordered(),
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        assert_eq!(
            TreadFlight::try_new(
                id("a"),
                x(),
                point(0.0),
                point(0.3),
                ordered(),
                evidence(true)
            ),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        // No treads, or a sloped direction.
        assert_eq!(
            TreadFlight::try_new(id("a"), x(), point(0.0), point(0.4), vec![], evidence(true)),
            Err(WalkingSurfaceError::InvalidMeasurement)
        );
        let sloped = MetricDirection::try_new([1.0, 0.0, 1.0]).unwrap();
        assert_eq!(
            TreadFlight::try_new(
                id("a"),
                sloped,
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
                id("a"),
                x(),
                point(0.0),
                point(0.4),
                ordered(),
                evidence(false)
            ),
            Err(WalkingSurfaceError::InexactEvidence)
        );
        let widened = ElevationInterval::try_new(0.39, 0.41).unwrap();
        assert_eq!(
            TreadFlight::try_new(id("a"), x(), point(0.0), widened, ordered(), evidence(true)),
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
        fn measure_tread_flight(&self, _: &ObjectId) -> Result<TreadFlight, WalkingSurfaceError> {
            TreadFlight::try_new(
                id("b"),
                x(),
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
            handle.measure_tread_flight(&id("a")),
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
        assert!(handle.measure_tread_flight(&id("b")).is_ok());
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
    }

    #[test]
    fn widths_come_from_the_sides_and_the_narrowest_tread_governs() {
        let sided = |z: f64, front: f64, right: f64| {
            tread(z, front, front + 0.3)
                .with_sides(point(0.0), point(right))
                .unwrap()
        };
        let treads = vec![sided(0.2, 0.0, 1.2), sided(0.4, 0.3, 1.1)];
        let flight =
            TreadFlight::try_new(id("a"), x(), point(0.0), point(0.4), treads, evidence(true))
                .unwrap();
        assert!(contains(flight.width().unwrap(), 1.1));
        // One tread without sides leaves the flight's width unknown.
        let treads = vec![sided(0.2, 0.0, 1.2), tread(0.4, 0.3, 0.6)];
        let flight =
            TreadFlight::try_new(id("a"), x(), point(0.0), point(0.4), treads, evidence(true))
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
                id("a"),
                x(),
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
}
