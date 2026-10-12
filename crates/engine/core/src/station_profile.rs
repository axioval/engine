//! Station-parameterised values: an alignment parameter (gradient,
//! curvature, cant) as a function of plan distance over a stretch, and the
//! measures read off it, its extremes and its rate of change.
//!
//! ADR 0004: this seam measures. Whether the steepest gradient of a ramp
//! or the cant ramp of a transition keeps a limit is a rule's judgement
//! over these measurements.
//!
//! A profile is a run of [`ProfilePiece`]s laid end to end along plan
//! distance. Each piece states, as intervals sure to hold the exact
//! values, the value at its start and at its end, and a **rate**: an
//! interval holding every derivative of the value along plan distance
//! over the piece (one-sided at its ends). That rate is the sampling
//! certificate: between the two ends the value can move no faster, so
//!
//! ```text
//! v(a + t) <= min(v(a) + rate.upper * t, v(b) - rate.lower * (w - t))
//! ```
//!
//! for every `t` in the piece's width `w`, and the greatest value over the
//! piece is bounded where those two lines meet. Where the rate keeps one
//! sign (a linear gradient over a vertical curve, a clothoid's curvature,
//! every cant transition) the extreme is an end and is computed exactly;
//! elsewhere it is bounded, and a service may sample a law more finely
//! (`step`) to narrow the bound. A service never states a piece whose rate
//! it cannot bound: it refuses the profile with the reason instead, so a
//! value is never sampled without a bound and called exact.
//!
//! The station labelling a point of a piece is its start station plus the
//! plan distance into it: a service splits pieces at station equations.

use axioval_ir::{Evidence, ObjectId};

use crate::alignment::{AlignmentError, AlignmentInterval, AlignmentParameter};

/// The rounding allowed on a bound computed by arithmetic: a few units in
/// the last place, outward.
fn up(value: f64) -> f64 {
    value + 4.0 * f64::EPSILON * value.abs() + f64::MIN_POSITIVE
}

fn down(value: f64) -> f64 {
    value - 4.0 * f64::EPSILON * value.abs() - f64::MIN_POSITIVE
}

fn interval(lower: f64, upper: f64) -> Result<AlignmentInterval, AlignmentError> {
    AlignmentInterval::try_new(lower, upper)
}

fn negated(value: AlignmentInterval) -> AlignmentInterval {
    AlignmentInterval::try_new(-value.upper(), -value.lower())
        .unwrap_or_else(|_| unreachable!("a negated interval stays ordered and finite"))
}

/// `[lower, upper]` multiplied by the non-negative `factor`, outward.
fn times(value: AlignmentInterval, factor: f64) -> Result<AlignmentInterval, AlignmentError> {
    let (lower, upper) = (value.lower() * factor, value.upper() * factor);
    if value.is_exact() && factor == 0.0 {
        return interval(0.0, 0.0);
    }
    interval(down(lower.min(upper)), up(lower.max(upper)))
}

/// The magnitudes of every value in `value`.
fn magnitude(value: AlignmentInterval) -> AlignmentInterval {
    let (lower, upper) = (value.lower(), value.upper());
    let (lower, upper) = if lower >= 0.0 {
        (lower, upper)
    } else if upper <= 0.0 {
        (-upper, -lower)
    } else {
        (0.0, (-lower).max(upper))
    };
    AlignmentInterval::try_new(lower, upper)
        .unwrap_or_else(|_| unreachable!("a magnitude stays ordered and finite"))
}

/// One stretch of a profile (module documentation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfilePiece {
    distance: AlignmentInterval,
    station: f64,
    start: AlignmentInterval,
    end: AlignmentInterval,
    rate: AlignmentInterval,
}

impl ProfilePiece {
    /// A piece over the plan distances `distance`, labelled `station` at
    /// its start, with the value `start` and `end` at its ends and `rate`
    /// holding every derivative along plan distance over it. A piece of no
    /// width is a point: its two values must overlap.
    pub fn try_new(
        distance: AlignmentInterval,
        station: f64,
        start: AlignmentInterval,
        end: AlignmentInterval,
        rate: AlignmentInterval,
    ) -> Result<Self, AlignmentError> {
        if distance.lower() < 0.0 || !station.is_finite() {
            return Err(AlignmentError::InvalidMeasurement);
        }
        if distance.is_exact() && (start.upper() < end.lower() || end.upper() < start.lower()) {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            distance,
            station,
            start,
            end,
            rate,
        })
    }

    /// The plan distances the piece spans, in metres.
    #[must_use]
    pub fn distance(&self) -> AlignmentInterval {
        self.distance
    }

    /// The station labelling the piece's start, in metres.
    #[must_use]
    pub fn station(&self) -> f64 {
        self.station
    }

    /// The value at the piece's start.
    #[must_use]
    pub fn start(&self) -> AlignmentInterval {
        self.start
    }

    /// The value at the piece's end.
    #[must_use]
    pub fn end(&self) -> AlignmentInterval {
        self.end
    }

    /// Every derivative of the value along plan distance over the piece.
    #[must_use]
    pub fn rate(&self) -> AlignmentInterval {
        self.rate
    }

    fn width(&self) -> f64 {
        self.distance.upper() - self.distance.lower()
    }

    fn is_exact(&self) -> bool {
        self.start.is_exact() && self.end.is_exact() && self.rate.is_exact()
    }

    fn negated(&self) -> Self {
        Self {
            start: negated(self.start),
            end: negated(self.end),
            rate: negated(self.rate),
            ..*self
        }
    }

    /// The piece scaled by the positive `factor`: values and rate alike,
    /// rounded outward.
    pub fn scaled(&self, factor: f64) -> Result<Self, AlignmentError> {
        if !(factor.is_finite() && factor > 0.0) {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            start: times(self.start, factor)?,
            end: times(self.end, factor)?,
            rate: times(self.rate, factor)?,
            ..*self
        })
    }

    /// The magnitude of the value over the piece, as pieces: unchanged
    /// where the value keeps its sign, mirrored where it is negative, and
    /// split where a value of one sign crosses zero, the stretch the
    /// crossing may lie in taking every rate of either sign.
    pub fn magnitude(&self) -> Result<Vec<Self>, AlignmentError> {
        let (lowest, highest) = (self.least_bound(), self.greatest_bound());
        if lowest >= 0.0 {
            return Ok(vec![*self]);
        }
        if highest <= 0.0 {
            return Ok(vec![self.negated()]);
        }
        let steepest = self.rate.lower().abs().max(self.rate.upper().abs());
        let either = interval(-steepest, steepest)?;
        let width = self.width();
        // A monotone value of opposite signs at its ends crosses zero once,
        // within the stretch both rate bounds allow.
        let rising = self.rate.lower() > 0.0 && self.start.upper() < 0.0 && self.end.lower() > 0.0;
        let falling = self.rate.upper() < 0.0 && self.start.lower() > 0.0 && self.end.upper() < 0.0;
        if !(rising || falling) || width <= 0.0 {
            return Ok(vec![Self {
                start: magnitude(self.start),
                end: magnitude(self.end),
                rate: either,
                ..*self
            }]);
        }
        let base = if rising { *self } else { self.negated() };
        // `base` rises from below zero to above it: it reaches zero no
        // sooner than at its fastest rate from its highest start, and no
        // later than at its slowest from its lowest.
        let soonest = down((-base.start.upper()) / base.rate.upper()).clamp(0.0, width);
        let latest = up((-base.start.lower()) / base.rate.lower()).clamp(soonest, width);
        let at = |t: f64| -> Result<AlignmentInterval, AlignmentError> {
            let from_start = (
                down(base.start.lower() + base.rate.lower() * t),
                up(base.start.upper() + base.rate.upper() * t),
            );
            let from_end = (
                down(base.end.lower() - base.rate.upper() * (width - t)),
                up(base.end.upper() - base.rate.lower() * (width - t)),
            );
            let (lower, upper) = (from_start.0.max(from_end.0), from_start.1.min(from_end.1));
            interval(lower.min(upper), upper.max(lower))
        };
        let (before, after) = (at(soonest)?, at(latest)?);
        // The split points, the piece's own end kept exactly.
        let at_distance = |t: f64| {
            if t >= width {
                self.distance.upper()
            } else {
                (self.distance.lower() + t).min(self.distance.upper())
            }
        };
        let distance = |from: f64, to: f64| interval(at_distance(from), at_distance(to));
        let mut pieces = Vec::new();
        let below = |value: AlignmentInterval| {
            interval(value.lower().min(0.0), value.upper().min(0.0)).map(magnitude)
        };
        let above =
            |value: AlignmentInterval| interval(value.lower().max(0.0), value.upper().max(0.0));
        pieces.push(Self {
            distance: distance(0.0, soonest)?,
            station: self.station,
            start: magnitude(base.start),
            end: below(before)?,
            rate: negated(base.rate),
        });
        pieces.push(Self {
            distance: distance(soonest, latest)?,
            station: self.station + soonest,
            start: magnitude(before),
            end: magnitude(after),
            rate: either,
        });
        pieces.push(Self {
            distance: distance(latest, width)?,
            station: self.station + latest,
            start: above(after)?,
            end: magnitude(base.end),
            rate: base.rate,
        });
        Ok(pieces)
    }

    /// An upper bound of the value over the piece (module documentation).
    fn greatest_bound(&self) -> f64 {
        let width = self.width();
        let (ua, ub) = (self.start.upper(), self.end.upper());
        let (sl, sh) = (self.rate.lower(), self.rate.upper());
        if width <= 0.0 {
            return ua.min(ub);
        }
        if sl >= 0.0 {
            return ub.min(up(ua + sh * width));
        }
        if sh <= 0.0 {
            return ua.min(up(ub - sl * width));
        }
        // The two lines meet inside the piece.
        let t = ((ub - ua - sl * width) / (sh - sl)).clamp(0.0, width);
        up((ua + sh * t).min(ub - sl * (width - t)))
    }

    fn least_bound(&self) -> f64 {
        -self.negated().greatest_bound()
    }

    /// The offsets into the piece where its value may be at least `least`,
    /// `None` where it surely is not.
    fn reaching(&self, least: f64) -> Option<(f64, f64)> {
        if self.greatest_bound() < least {
            return None;
        }
        let width = self.width();
        let (ua, ub) = (self.start.upper(), self.end.upper());
        let (sl, sh) = (self.rate.lower(), self.rate.upper());
        let (mut from, mut to) = (0.0_f64, width);
        // From the start: ua + sh t >= least.
        if sh > 0.0 {
            from = from.max((least - ua) / sh);
        } else if sh < 0.0 {
            to = to.min((ua - least) / -sh);
        }
        // From the end: ub - sl (w - t) >= least.
        if sl > 0.0 {
            from = from.max(width - (ub - least) / sl);
        } else if sl < 0.0 {
            to = to.min(width - (ub - least) / sl);
        }
        let slack = 1e-9 * width.max(1.0);
        let (from, to) = ((from - slack).max(0.0), (to + slack).min(width));
        // Rounding may cross the two; the whole piece is then reported.
        Some(if from <= to { (from, to) } else { (0.0, width) })
    }
}

/// `parameter` of `alignment` over the stations `from` to `to` (the
/// alignment's start and end where not stated), sampled at most `step`
/// apart where stated.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentProfileRequest {
    alignment: ObjectId,
    parameter: AlignmentParameter,
    from: Option<f64>,
    to: Option<f64>,
    step: Option<f64>,
}

impl AlignmentProfileRequest {
    /// A request; stations must be finite and in order, a step positive.
    pub fn try_new(
        alignment: ObjectId,
        parameter: AlignmentParameter,
        from: Option<f64>,
        to: Option<f64>,
        step: Option<f64>,
    ) -> Result<Self, AlignmentError> {
        let finite = [from, to].into_iter().flatten().all(f64::is_finite);
        let ordered = match (from, to) {
            (Some(from), Some(to)) => from <= to,
            _ => true,
        };
        let stepped = step.is_none_or(|step| step.is_finite() && step > 0.0);
        if !(finite && ordered && stepped) {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            alignment,
            parameter,
            from,
            to,
            step,
        })
    }

    /// The alignment read.
    #[must_use]
    pub fn alignment(&self) -> &ObjectId {
        &self.alignment
    }

    /// The parameter read.
    #[must_use]
    pub fn parameter(&self) -> AlignmentParameter {
        self.parameter
    }

    /// The station the stretch starts at; `None` at the alignment's start.
    #[must_use]
    pub fn from(&self) -> Option<f64> {
        self.from
    }

    /// The station the stretch ends at; `None` at the alignment's end.
    #[must_use]
    pub fn to(&self) -> Option<f64> {
        self.to
    }

    /// The greatest width of a piece, in metres of plan distance, where
    /// stated.
    #[must_use]
    pub fn step(&self) -> Option<f64> {
        self.step
    }
}

/// Which extreme of a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileExtreme {
    /// The greatest value.
    Greatest,
    /// The least value.
    Least,
}

/// An extreme of a profile: an interval sure to hold it, and the station
/// stretches it may be attained in, ascending and disjoint.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileExtremeValue {
    value: AlignmentInterval,
    stations: Vec<(f64, f64)>,
    exact: bool,
}

impl ProfileExtremeValue {
    /// The extreme value.
    #[must_use]
    pub fn value(&self) -> AlignmentInterval {
        self.value
    }

    /// The station stretches it may be attained in: every point where it
    /// is lies in one of them.
    #[must_use]
    pub fn stations(&self) -> &[(f64, f64)] {
        &self.stations
    }

    /// Whether the value is exact: a point of a profile stated exactly.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.exact
    }
}

/// A parameter's profile over a stretch, or `None` when the alignment
/// states none (an alignment without cant), with evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentProfile {
    request: AlignmentProfileRequest,
    pieces: Option<Vec<ProfilePiece>>,
    evidence: Evidence,
}

impl AlignmentProfile {
    /// A profile answering `request`. Only cant may be stated as none;
    /// otherwise the pieces are at least one, laid end to end in
    /// ascending plan distance, and the evidence is exact exactly when
    /// every value and rate is a point.
    pub fn try_new(
        request: AlignmentProfileRequest,
        pieces: Option<Vec<ProfilePiece>>,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        match &pieces {
            None if !matches!(
                request.parameter,
                AlignmentParameter::Cant | AlignmentParameter::CantRatio
            ) =>
            {
                return Err(AlignmentError::InvalidMeasurement);
            }
            Some(pieces) => {
                let contiguous = pieces.windows(2).all(|pair| {
                    pair[0].distance.upper().to_bits() == pair[1].distance.lower().to_bits()
                });
                if pieces.is_empty() || !contiguous {
                    return Err(AlignmentError::InvalidMeasurement);
                }
            }
            None => {}
        }
        let exact = pieces
            .as_ref()
            .is_none_or(|pieces| pieces.iter().all(ProfilePiece::is_exact));
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            pieces,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &AlignmentProfileRequest {
        &self.request
    }

    /// The pieces, or `None` when the alignment states none.
    #[must_use]
    pub fn pieces(&self) -> Option<&[ProfilePiece]> {
        self.pieces.as_deref()
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }

    /// The greatest or least value over the stretch and where it may be
    /// attained; `None` when the alignment states none.
    #[must_use]
    pub fn extreme(&self, which: ProfileExtreme) -> Option<ProfileExtremeValue> {
        let pieces = self.pieces.as_deref()?;
        let flip = which == ProfileExtreme::Least;
        let pieces: Vec<ProfilePiece> = pieces
            .iter()
            .map(|piece| if flip { piece.negated() } else { *piece })
            .collect();
        // The values surely attained: the ends of every piece.
        let least = pieces
            .iter()
            .map(|piece| piece.start.lower().max(piece.end.lower()))
            .fold(f64::NEG_INFINITY, f64::max);
        let most = pieces
            .iter()
            .map(ProfilePiece::greatest_bound)
            .fold(f64::NEG_INFINITY, f64::max)
            .max(least);
        let stations = merged(pieces.iter().filter_map(|piece| {
            piece
                .reaching(least)
                .map(|(from, to)| (piece.station + from, piece.station + to))
        }));
        let value = if flip {
            AlignmentInterval::try_new(-most, -least)
        } else {
            AlignmentInterval::try_new(least, most)
        }
        .ok()?;
        Some(ProfileExtremeValue {
            value,
            stations,
            exact: self.evidence.exact && value.is_exact(),
        })
    }
}

/// Station stretches sorted and joined where they touch.
fn merged(stretches: impl Iterator<Item = (f64, f64)>) -> Vec<(f64, f64)> {
    let mut stretches: Vec<(f64, f64)> = stretches.collect();
    stretches.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (from, to) in stretches {
        match out.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    fn point(value: f64) -> AlignmentInterval {
        AlignmentInterval::exact(value).unwrap()
    }

    fn span(lower: f64, upper: f64) -> AlignmentInterval {
        AlignmentInterval::try_new(lower, upper).unwrap()
    }

    /// A piece from `a` to `b` (labelled as its distance), linear from
    /// `start` to `end`, its rate stated exactly.
    fn linear(a: f64, b: f64, start: f64, end: f64) -> ProfilePiece {
        let rate = if b > a { (end - start) / (b - a) } else { 0.0 };
        ProfilePiece::try_new(span(a, b), a, point(start), point(end), point(rate)).unwrap()
    }

    fn profile(parameter: AlignmentParameter, pieces: Vec<ProfilePiece>) -> AlignmentProfile {
        let exact = pieces.iter().all(ProfilePiece::is_exact);
        let mut evidence = Evidence::exact(SourceId::new("test", "model").unwrap(), "profile");
        evidence.exact = exact;
        let request =
            AlignmentProfileRequest::try_new(id("axis"), parameter, None, None, None).unwrap();
        AlignmentProfile::try_new(request, Some(pieces), evidence).unwrap()
    }

    /// #339: a gradient profile rising from 2 % over a vertical curve to a
    /// peak of 6.2 % at 160 m, falling to 3 % over another and holding it,
    /// as piecewise linear gradients.
    fn peaked() -> AlignmentProfile {
        profile(
            AlignmentParameter::Gradient,
            vec![
                linear(0.0, 100.0, 0.02, 0.02),
                linear(100.0, 160.0, 0.02, 0.062),
                linear(160.0, 240.0, 0.062, 0.03),
                linear(240.0, 300.0, 0.03, 0.03),
            ],
        )
    }

    #[test]
    fn the_steepest_gradient_is_its_peak_at_the_peak_station() {
        let greatest = peaked().extreme(ProfileExtreme::Greatest).unwrap();
        assert_eq!(greatest.value(), point(0.062));
        assert!(greatest.is_exact());
        let [(from, to)] = greatest.stations() else {
            panic!("one stretch: {:?}", greatest.stations());
        };
        assert!(
            *from <= 160.0 && *to >= 160.0 && to - from < 1e-6,
            "{from}..{to}"
        );
        // The least is the 2 % held over the first 100 m.
        let least = peaked().extreme(ProfileExtreme::Least).unwrap();
        assert_eq!(least.value(), point(0.02));
        let [(from, to)] = least.stations() else {
            panic!("one stretch: {:?}", least.stations());
        };
        assert!(
            *from <= 0.0 && *to >= 100.0 && *to < 100.0 + 1e-6,
            "{from}..{to}"
        );
    }

    #[test]
    fn a_bounded_rate_bounds_an_extreme_inside_a_piece() {
        // Zero at both ends of 10 m, rising and falling by at most 0.01 per
        // metre: it can reach 0.05 midway, and surely reaches 0.
        let hump = ProfilePiece::try_new(span(0.0, 10.0), 0.0, point(0.0), point(0.0), {
            span(-0.01, 0.01)
        })
        .unwrap();
        let mut evidence = Evidence::exact(SourceId::new("test", "model").unwrap(), "hump");
        evidence.exact = false;
        let request = AlignmentProfileRequest::try_new(
            id("axis"),
            AlignmentParameter::Gradient,
            None,
            None,
            None,
        )
        .unwrap();
        let profile = AlignmentProfile::try_new(request, Some(vec![hump]), evidence).unwrap();
        let greatest = profile.extreme(ProfileExtreme::Greatest).unwrap();
        assert_eq!(greatest.value().lower(), 0.0);
        assert!((greatest.value().upper() - 0.05).abs() < 1e-12);
        assert!(!greatest.is_exact());
        assert_eq!(greatest.stations(), &[(0.0, 10.0)]);
    }

    #[test]
    fn a_magnitude_splits_where_the_value_crosses_zero() {
        // Cant from -0.02 to 0.06 over 40 m: zero at 10 m.
        let crossing = linear(0.0, 40.0, -0.02, 0.06);
        let pieces = crossing.magnitude().unwrap();
        assert_eq!(pieces.len(), 3);
        let unsigned = profile(AlignmentParameter::Cant, pieces.clone());
        let least = unsigned.extreme(ProfileExtreme::Least).unwrap();
        assert!(least.value().lower().abs() < 1e-12 && least.value().upper() < 1e-12);
        let [(from, to)] = least.stations() else {
            panic!("{:?}", least.stations());
        };
        assert!(*from <= 10.0 && *to >= 10.0 && to - from < 1e-6);
        let greatest = unsigned.extreme(ProfileExtreme::Greatest).unwrap();
        assert_eq!(greatest.value(), point(0.06));
        // Falling magnitude before the crossing, rising after.
        assert!(pieces[0].rate().upper() < 0.0 && pieces[2].rate().lower() > 0.0);
        // A value of one sign is unchanged, a negative one mirrored.
        assert_eq!(linear(0.0, 1.0, 0.1, 0.2).magnitude().unwrap().len(), 1);
        let mirrored = linear(0.0, 1.0, -0.1, -0.2).magnitude().unwrap();
        assert_eq!(mirrored[0].end(), point(0.2));
    }

    #[test]
    fn requests_and_profiles_are_validated() {
        let request = |from, to, step| {
            AlignmentProfileRequest::try_new(
                id("axis"),
                AlignmentParameter::Gradient,
                from,
                to,
                step,
            )
        };
        assert!(request(Some(1.0), Some(0.0), None).is_err());
        assert!(request(None, None, Some(0.0)).is_err());
        assert!(request(Some(f64::NAN), None, None).is_err());
        let ok = request(None, Some(5.0), Some(1.0)).unwrap();
        let mut evidence = Evidence::exact(SourceId::new("test", "model").unwrap(), "p");
        // Only cant may be none.
        assert_eq!(
            AlignmentProfile::try_new(ok.clone(), None, evidence.clone()),
            Err(AlignmentError::InvalidMeasurement)
        );
        // Pieces must be laid end to end.
        let gap = vec![linear(0.0, 1.0, 0.0, 0.0), linear(2.0, 3.0, 0.0, 0.0)];
        assert_eq!(
            AlignmentProfile::try_new(ok.clone(), Some(gap), evidence.clone()),
            Err(AlignmentError::InvalidMeasurement)
        );
        // Exactness must agree with the pieces.
        evidence.exact = false;
        assert_eq!(
            AlignmentProfile::try_new(ok, Some(vec![linear(0.0, 1.0, 0.0, 1.0)]), evidence),
            Err(AlignmentError::InexactEvidence)
        );
        // A point whose two values cannot both hold.
        assert!(
            ProfilePiece::try_new(span(1.0, 1.0), 1.0, point(0.0), point(1.0), point(0.0)).is_err()
        );
    }
}
