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
//! What the seam does not measure yet (Refs #85): winders and turning
//! flights, open risers, landing sizes, clear width, handrails and the slab
//! connection. A service refuses a shape it cannot decide rather than
//! approximate it.

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
        })
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

    fn is_exact(&self) -> bool {
        self.elevation.is_exact() && self.front.is_exact() && self.back.is_exact()
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
        })
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

    fn is_exact(&self) -> bool {
        self.bottom.is_exact()
            && self.top.is_exact()
            && self.start.is_exact()
            && self.end.is_exact()
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
        mut governing: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, WalkingSurfaceError> {
        governing.sort();
        governing.dedup();
        let named = governing
            .iter()
            .all(|obstacle| request.obstacles.binary_search(obstacle).is_ok());
        if !named || clearance.is_some() == governing.is_empty() {
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

/// Measures stair flights, ramps and the headroom above them.
pub trait WalkingSurfaceService: Send + Sync + 'static {
    /// The treads, base and top of `object` as a straight stair flight.
    fn measure_tread_flight(&self, object: &ObjectId) -> Result<TreadFlight, WalkingSurfaceError>;

    /// The sloped runs of `object` as a ramp.
    fn measure_sloped_runs(&self, object: &ObjectId) -> Result<SlopedSurface, WalkingSurfaceError>;

    /// The headroom above the request subject's walking surface.
    fn measure_headroom(&self, request: &HeadroomRequest) -> Result<Headroom, WalkingSurfaceError>;
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
    }
}
