//! Positions along an alignment: an object's station, signed lateral
//! offset and height above the gradient line, and the alignment's own
//! parameters (horizontal curvature, gradient, cant) over a stretch.
//!
//! ADR 0004: this seam measures. Whether a pier stands within a station
//! range or a mast far enough from the track axis is a rule's judgement
//! over these measurements.
//!
//! An alignment is an object the host can read as a 3D centreline: a plan
//! curve measured by its own length from its start, and a gradient line
//! giving the height along it. Every value is an interval sure to hold the
//! exact one, and the evidence is exact exactly when every interval is a
//! point: locating a point on a curve is a numerical search, so a position
//! is practically never exact.
//!
//! The located point is the object's reference point, its placement
//! origin. Its foot is where the plan perpendicular from the point meets
//! the plan curve, the nearest such foot when there are several, so:
//!
//! - the **distance** is the plan length from the alignment's start to the
//!   foot;
//! - the **station** is the label of that distance, through the station
//!   equations the alignment states, the distance itself when it states
//!   none;
//! - the **offset** is the plan distance from the foot to the point, signed
//!   positive to the left of the direction of travel (the lateral axis of
//!   the section frame: tangent, left, up);
//! - the **height** is the point's elevation above the gradient line at the
//!   foot.
//!
//! A point whose nearest foot would lie before the start or beyond the end
//! is off the alignment's range and is refused with that reason, never
//! clamped to an end. Two feet whose distances cannot be told apart are
//! ambiguous and refused too.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

use crate::section::{EnvelopeRequest, EnvelopeSweep, Section, SectionRequest};

/// Failure to locate a point along an alignment or read its parameters.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AlignmentError {
    /// The service holds no reference point for this object.
    #[error("no reference point for `{0}`")]
    UnknownObject(ObjectId),
    /// The object is no alignment the service can read as a centreline.
    #[error("`{0}` is no alignment the service can read")]
    NotAlignment(ObjectId),
    /// The point's nearest foot lies before the start or beyond the end of
    /// the alignment.
    #[error("off the alignment's range: {0}")]
    OffRange(String),
    /// Two feet are equally near within the measurement's bounds, or the
    /// nearest one cannot be decided.
    #[error("the position along the alignment is ambiguous: {0}")]
    Ambiguous(String),
    /// The measurement could not be made: an unbounded curve family, a
    /// stationing that cannot be read, a placement that cannot be resolved.
    #[error("{0}")]
    Unavailable(String),
    /// A request naming the object as its own alignment, a non-finite or
    /// reversed interval, or an answer about another request.
    #[error("alignment measurement is invalid")]
    InvalidMeasurement,
    /// Evidence reported as exact for an interval, or as inexact for
    /// points, or without a locator.
    #[error("alignment evidence does not match its exactness")]
    InexactEvidence,
}

/// A value known to lie in `[lower, upper]`: metres, or a plain number
/// for a curvature (per metre) or a gradient (rise over plan run).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlignmentInterval {
    lower: f64,
    upper: f64,
}

impl AlignmentInterval {
    /// An interval; bounds must be finite and ordered.
    pub fn try_new(lower: f64, upper: f64) -> Result<Self, AlignmentError> {
        if !lower.is_finite() || !upper.is_finite() || lower > upper {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self { lower, upper })
    }

    /// A value known exactly.
    pub fn exact(value: f64) -> Result<Self, AlignmentError> {
        Self::try_new(value, value)
    }

    /// Lowest possible value.
    #[must_use]
    pub fn lower(&self) -> f64 {
        self.lower
    }

    /// Highest possible value.
    #[must_use]
    pub fn upper(&self) -> f64 {
        self.upper
    }

    /// Whether the value is a single number.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.lower.to_bits() == self.upper.to_bits()
    }
}

/// Where `object`'s reference point lies along `alignment`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlignmentRequest {
    object: ObjectId,
    alignment: ObjectId,
}

impl AlignmentRequest {
    /// A request; an object is never located along itself.
    pub fn try_new(object: ObjectId, alignment: ObjectId) -> Result<Self, AlignmentError> {
        if object == alignment {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self { object, alignment })
    }

    /// The located object.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// The alignment it is located along.
    #[must_use]
    pub fn alignment(&self) -> &ObjectId {
        &self.alignment
    }
}

/// An object's reference point located along an alignment (module
/// documentation): distance, station, signed offset and height, with
/// evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentPosition {
    request: AlignmentRequest,
    distance: AlignmentInterval,
    station: AlignmentInterval,
    offset: AlignmentInterval,
    height: AlignmentInterval,
    evidence: Evidence,
}

impl AlignmentPosition {
    /// A position answering `request`. The distance may not be negative,
    /// and the evidence is exact exactly when every interval is a point.
    pub fn try_new(
        request: AlignmentRequest,
        distance: AlignmentInterval,
        station: AlignmentInterval,
        offset: AlignmentInterval,
        height: AlignmentInterval,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        if distance.lower < 0.0 {
            return Err(AlignmentError::InvalidMeasurement);
        }
        let exact = [distance, station, offset, height]
            .iter()
            .all(AlignmentInterval::is_exact);
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            distance,
            station,
            offset,
            height,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &AlignmentRequest {
        &self.request
    }

    /// Plan length from the alignment's start to the foot, in metres.
    #[must_use]
    pub fn distance(&self) -> AlignmentInterval {
        self.distance
    }

    /// The station labelling that distance, in metres.
    #[must_use]
    pub fn station(&self) -> AlignmentInterval {
        self.station
    }

    /// Plan offset from the foot, positive to the left, in metres.
    #[must_use]
    pub fn offset(&self) -> AlignmentInterval {
        self.offset
    }

    /// Height above the gradient line at the foot, in metres.
    #[must_use]
    pub fn height(&self) -> AlignmentInterval {
        self.height
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// A parameter of an alignment over a stretch of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlignmentParameter {
    /// The plan's signed curvature, per metre, positive turning left.
    Curvature,
    /// The gradient line's rise over plan run, positive rising in the
    /// direction of travel.
    Gradient,
    /// The cant's magnitude: how far one rail head stands above the other,
    /// in metres.
    Cant,
}

/// `parameter` of `alignment` over the plan distances `distance`.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentParameterRequest {
    alignment: ObjectId,
    parameter: AlignmentParameter,
    distance: AlignmentInterval,
}

impl AlignmentParameterRequest {
    /// A request over a stretch of non-negative plan distances.
    pub fn try_new(
        alignment: ObjectId,
        parameter: AlignmentParameter,
        distance: AlignmentInterval,
    ) -> Result<Self, AlignmentError> {
        if distance.lower < 0.0 {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(Self {
            alignment,
            parameter,
            distance,
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

    /// The plan distances it is read over, in metres.
    #[must_use]
    pub fn distance(&self) -> AlignmentInterval {
        self.distance
    }
}

/// A parameter's values over a stretch: an interval holding every value
/// it takes there, or `None` when the alignment states none (an alignment
/// without cant), with evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct AlignmentParameterValue {
    request: AlignmentParameterRequest,
    value: Option<AlignmentInterval>,
    evidence: Evidence,
}

impl AlignmentParameterValue {
    /// A value answering `request`. Only cant may be stated as none, and
    /// the evidence is exact exactly when the value is a point or none.
    pub fn try_new(
        request: AlignmentParameterRequest,
        value: Option<AlignmentInterval>,
        evidence: Evidence,
    ) -> Result<Self, AlignmentError> {
        if value.is_none() && request.parameter != AlignmentParameter::Cant {
            return Err(AlignmentError::InvalidMeasurement);
        }
        if value.is_some_and(|value| value.lower < 0.0)
            && request.parameter == AlignmentParameter::Cant
        {
            return Err(AlignmentError::InvalidMeasurement);
        }
        let exact = value.is_none_or(|value| value.is_exact());
        if evidence.exact != exact || evidence.locator.trim().is_empty() {
            return Err(AlignmentError::InexactEvidence);
        }
        Ok(Self {
            request,
            value,
            evidence,
        })
    }

    /// The request answered.
    #[must_use]
    pub fn request(&self) -> &AlignmentParameterRequest {
        &self.request
    }

    /// Every value the parameter takes over the stretch, or `None` when the
    /// alignment states none.
    #[must_use]
    pub fn value(&self) -> Option<AlignmentInterval> {
        self.value
    }

    /// Reviewable provenance of the measurement.
    #[must_use]
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Trusted adapter seam locating points along alignments.
pub trait AlignmentService: Send + Sync + 'static {
    /// Where `request`'s object lies along its alignment, or why that
    /// cannot be told.
    // gate: measures length
    fn measure_alignment_position(
        &self,
        request: &AlignmentRequest,
    ) -> Result<AlignmentPosition, AlignmentError>;

    /// A parameter of an alignment over a stretch.
    ///
    /// The default refuses, so a service that reads no parameters fails
    /// closed rather than answering with a straight, level, uncanted line.
    // gate: measures length, number
    fn measure_alignment_parameter(
        &self,
        request: &AlignmentParameterRequest,
    ) -> Result<AlignmentParameterValue, AlignmentError> {
        let _ = request;
        Err(AlignmentError::Unavailable(
            "this alignment service reads no alignment parameters".into(),
        ))
    }

    /// The sections of bodies at a station ([`Section`]).
    ///
    /// The default refuses, so a service that cuts no bodies fails closed
    /// rather than answering with empty sections.
    // gate: measures length, area
    fn measure_section(&self, request: &SectionRequest) -> Result<Section, AlignmentError> {
        let _ = request;
        Err(AlignmentError::Unavailable(
            "this alignment service cuts no sections".into(),
        ))
    }

    /// Which bodies reach into a clearance envelope swept along a range
    /// ([`EnvelopeSweep`]).
    ///
    /// The default refuses, so a service that sweeps no envelope fails
    /// closed rather than answering that every body is clear.
    // gate: measures length, area
    fn measure_envelope(&self, request: &EnvelopeRequest) -> Result<EnvelopeSweep, AlignmentError> {
        let _ = request;
        Err(AlignmentError::Unavailable(
            "this alignment service sweeps no clearance envelope".into(),
        ))
    }
}

/// Registry handle for an [`AlignmentService`].
#[derive(Clone)]
pub struct AlignmentServiceHandle(Arc<dyn AlignmentService>);

impl AlignmentServiceHandle {
    /// Wraps a trusted alignment service.
    #[must_use]
    pub fn new(service: Arc<dyn AlignmentService>) -> Self {
        Self(service)
    }

    /// Where `request`'s object lies along its alignment. An answer about
    /// another request is refused.
    pub fn measure_alignment_position(
        &self,
        request: &AlignmentRequest,
    ) -> Result<AlignmentPosition, AlignmentError> {
        let position = self.0.measure_alignment_position(request)?;
        if position.request() != request {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(position)
    }

    /// A parameter of an alignment over a stretch. An answer about another
    /// request is refused.
    pub fn measure_alignment_parameter(
        &self,
        request: &AlignmentParameterRequest,
    ) -> Result<AlignmentParameterValue, AlignmentError> {
        let value = self.0.measure_alignment_parameter(request)?;
        if value.request() != request {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(value)
    }

    /// The sections of bodies at a station. An answer about another
    /// request is refused.
    pub fn measure_section(&self, request: &SectionRequest) -> Result<Section, AlignmentError> {
        let section = self.0.measure_section(request)?;
        if section.request() != request {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(section)
    }

    /// Which bodies reach into a clearance envelope swept along a range.
    /// An answer about another request is refused.
    pub fn measure_envelope(
        &self,
        request: &EnvelopeRequest,
    ) -> Result<EnvelopeSweep, AlignmentError> {
        let sweep = self.0.measure_envelope(request)?;
        if sweep.request() != request {
            return Err(AlignmentError::InvalidMeasurement);
        }
        Ok(sweep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axioval_ir::SourceId;

    fn id(local: &str) -> ObjectId {
        ObjectId::new(SourceId::new("test", "model").unwrap(), local).unwrap()
    }

    fn evidence(exact: bool) -> Evidence {
        let mut evidence = Evidence::exact(SourceId::new("test", "model").unwrap(), "along");
        evidence.exact = exact;
        evidence
    }

    fn interval(lower: f64, upper: f64) -> AlignmentInterval {
        AlignmentInterval::try_new(lower, upper).unwrap()
    }

    #[test]
    fn an_object_is_never_located_along_itself() {
        assert_eq!(
            AlignmentRequest::try_new(id("a"), id("a")),
            Err(AlignmentError::InvalidMeasurement)
        );
    }

    #[test]
    fn exactness_and_intervals_must_agree() {
        let request = AlignmentRequest::try_new(id("pier"), id("axis")).unwrap();
        let point = interval(5.0, 5.0);
        let wide = interval(4.9, 5.1);
        let position = |offset, exact| {
            AlignmentPosition::try_new(
                request.clone(),
                point,
                point,
                offset,
                point,
                evidence(exact),
            )
        };
        assert!(position(point, true).is_ok());
        assert!(position(wide, false).is_ok());
        assert_eq!(position(wide, true), Err(AlignmentError::InexactEvidence));
        assert_eq!(position(point, false), Err(AlignmentError::InexactEvidence));
        assert!(AlignmentInterval::try_new(1.0, 0.0).is_err());
        assert!(AlignmentInterval::try_new(f64::NAN, 0.0).is_err());
        assert_eq!(
            AlignmentPosition::try_new(
                request,
                interval(-1.0, -1.0),
                point,
                point,
                point,
                evidence(true)
            ),
            Err(AlignmentError::InvalidMeasurement)
        );
    }

    #[test]
    fn only_cant_may_be_stated_as_none() {
        let request = |parameter| {
            AlignmentParameterRequest::try_new(id("axis"), parameter, interval(0.0, 1.0)).unwrap()
        };
        assert!(
            AlignmentParameterValue::try_new(
                request(AlignmentParameter::Cant),
                None,
                evidence(true)
            )
            .is_ok()
        );
        assert_eq!(
            AlignmentParameterValue::try_new(
                request(AlignmentParameter::Gradient),
                None,
                evidence(true)
            ),
            Err(AlignmentError::InvalidMeasurement)
        );
        assert_eq!(
            AlignmentParameterValue::try_new(
                request(AlignmentParameter::Cant),
                Some(interval(-0.1, 0.0)),
                evidence(false)
            ),
            Err(AlignmentError::InvalidMeasurement)
        );
    }

    struct Elsewhere;

    impl AlignmentService for Elsewhere {
        fn measure_alignment_position(
            &self,
            request: &AlignmentRequest,
        ) -> Result<AlignmentPosition, AlignmentError> {
            let other = AlignmentRequest::try_new(request.object().clone(), id("other"))?;
            let point = AlignmentInterval::exact(1.0)?;
            AlignmentPosition::try_new(other, point, point, point, point, evidence(true))
        }
    }

    #[test]
    fn the_handle_refuses_an_answer_about_another_request_and_defaults_refuse() {
        let handle = AlignmentServiceHandle::new(Arc::new(Elsewhere));
        let request = AlignmentRequest::try_new(id("pier"), id("axis")).unwrap();
        assert_eq!(
            handle.measure_alignment_position(&request),
            Err(AlignmentError::InvalidMeasurement)
        );
        let parameter = AlignmentParameterRequest::try_new(
            id("axis"),
            AlignmentParameter::Gradient,
            interval(0.0, 1.0),
        )
        .unwrap();
        assert!(matches!(
            handle.measure_alignment_parameter(&parameter),
            Err(AlignmentError::Unavailable(_))
        ));
    }
}
