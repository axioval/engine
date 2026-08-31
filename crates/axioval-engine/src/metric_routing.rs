//! Source-neutral metric-routing evidence and host-service contracts.
//!
//! Geometry algorithms do not live here. A trusted Axiolid or alternate backend
//! supplies this interface after adapting its native geometry into validated,
//! source-qualified evidence.

use std::sync::Arc;

use axioval_ir::{Evidence, ObjectId};
use thiserror::Error;

/// Fail-closed metric routing errors.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MetricRoutingError {
    /// A coordinate was NaN or infinite.
    #[error("metric point coordinates must be finite")]
    InvalidCoordinate,
    /// A scalar length was negative, non-finite, or had reversed bounds.
    #[error("metric length interval is invalid")]
    InvalidLengthInterval,
    /// A mobility dimension was negative or non-finite.
    #[error("mobility profile contains an invalid dimension")]
    InvalidMobilityProfile,
    /// A route response omitted its path or traversed-object evidence.
    #[error("metric route evidence is empty")]
    EmptyRouteEvidence,
    /// Route provenance was approximate or blank.
    #[error("metric route provenance is not exact and reviewable")]
    InexactRouteEvidence,
    /// A blocked verdict did not prove complete obstacle/topology coverage.
    #[error("metric evidence is incomplete")]
    IncompleteMetricEvidence,
    /// A backend returned a route for different endpoints than requested.
    #[error("metric routing backend returned mismatched endpoints")]
    ResponseEndpointMismatch,
    /// Required geometry was not available for the named object.
    #[error("metric geometry is unavailable for `{0}`")]
    MissingGeometry(Box<ObjectId>),
    /// The backend deliberately refused an unsupported or partial query.
    #[error("metric routing query unavailable: {0}")]
    Unavailable(String),
}

/// Three-valued result for comparing bounded evidence with a policy threshold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThresholdVerdict {
    /// Every value in the interval meets the maximum.
    Satisfied,
    /// Every value in the interval exceeds the maximum.
    Violated,
    /// Bounds straddle the maximum, so policy evaluation must not guess.
    Indeterminate,
}

/// Conservative bounds for a non-negative metric length in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthInterval {
    lower_metres: f64,
    upper_metres: f64,
}

impl LengthInterval {
    /// Validates inclusive lower and upper distance bounds.
    pub fn try_new(lower_metres: f64, upper_metres: f64) -> Result<Self, MetricRoutingError> {
        if !valid_non_negative(lower_metres)
            || !valid_non_negative(upper_metres)
            || lower_metres > upper_metres
        {
            return Err(MetricRoutingError::InvalidLengthInterval);
        }
        Ok(Self {
            lower_metres,
            upper_metres,
        })
    }

    /// Creates a zero-error interval.
    pub fn exact(metres: f64) -> Result<Self, MetricRoutingError> {
        Self::try_new(metres, metres)
    }

    /// Inclusive lower bound in metres.
    pub fn lower_metres(&self) -> f64 {
        self.lower_metres
    }

    /// Inclusive upper bound in metres.
    pub fn upper_metres(&self) -> f64 {
        self.upper_metres
    }

    /// Whether the interval proves one exact value.
    #[allow(clippy::float_cmp)]
    pub fn is_exact(&self) -> bool {
        // The exact constructor writes the same validated scalar to both fields;
        // this tests evidence identity, not numerical convergence.
        self.lower_metres == self.upper_metres
    }

    /// Compares this interval to an inclusive maximum without collapsing uncertainty.
    pub fn compare_maximum(
        &self,
        maximum_metres: f64,
    ) -> Result<ThresholdVerdict, MetricRoutingError> {
        if !valid_non_negative(maximum_metres) {
            return Err(MetricRoutingError::InvalidLengthInterval);
        }
        if self.upper_metres <= maximum_metres {
            Ok(ThresholdVerdict::Satisfied)
        } else if self.lower_metres > maximum_metres {
            Ok(ThresholdVerdict::Violated)
        } else {
            Ok(ThresholdVerdict::Indeterminate)
        }
    }
}

/// A source-qualified object-grounded point expressed in canonical metres.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricPoint {
    subject: ObjectId,
    coordinates_metres: [f64; 3],
}

impl MetricPoint {
    /// Validates a model-grounded point.
    pub fn try_new(
        subject: ObjectId,
        coordinates_metres: [f64; 3],
    ) -> Result<Self, MetricRoutingError> {
        if !coordinates_metres.iter().all(|value| value.is_finite()) {
            return Err(MetricRoutingError::InvalidCoordinate);
        }
        Ok(Self {
            subject,
            coordinates_metres,
        })
    }

    /// Object grounding this point.
    pub fn subject(&self) -> &ObjectId {
        &self.subject
    }

    /// Canonical coordinates in metres.
    pub fn coordinates_metres(&self) -> [f64; 3] {
        self.coordinates_metres
    }
}

/// Geometry-independent mobility envelope used by route providers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MobilityProfile {
    radius_metres: f64,
    height_metres: f64,
    maximum_step_metres: f64,
    maximum_slope: f64,
}

impl MobilityProfile {
    /// Validates non-negative finite mobility dimensions.
    pub fn try_new(
        radius_metres: f64,
        height_metres: f64,
        maximum_step_metres: f64,
        maximum_slope: f64,
    ) -> Result<Self, MetricRoutingError> {
        if ![
            radius_metres,
            height_metres,
            maximum_step_metres,
            maximum_slope,
        ]
        .into_iter()
        .all(valid_non_negative)
        {
            return Err(MetricRoutingError::InvalidMobilityProfile);
        }
        Ok(Self {
            radius_metres,
            height_metres,
            maximum_step_metres,
            maximum_slope,
        })
    }

    /// Agent radius in metres.
    pub fn radius_metres(&self) -> f64 {
        self.radius_metres
    }

    /// Required clear height in metres.
    pub fn height_metres(&self) -> f64 {
        self.height_metres
    }

    /// Maximum traversable step in metres.
    pub fn maximum_step_metres(&self) -> f64 {
        self.maximum_step_metres
    }

    /// Maximum dimensionless slope ratio.
    pub fn maximum_slope(&self) -> f64 {
        self.maximum_slope
    }
}

/// One source-neutral metric routing request.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricRouteRequest {
    origin: MetricPoint,
    destination: MetricPoint,
    profile: MobilityProfile,
}

impl MetricRouteRequest {
    /// Creates a request from already validated values.
    pub fn new(origin: MetricPoint, destination: MetricPoint, profile: MobilityProfile) -> Self {
        Self {
            origin,
            destination,
            profile,
        }
    }

    /// Route origin.
    pub fn origin(&self) -> &MetricPoint {
        &self.origin
    }

    /// Route destination.
    pub fn destination(&self) -> &MetricPoint {
        &self.destination
    }

    /// Mobility envelope.
    pub fn profile(&self) -> MobilityProfile {
        self.profile
    }
}

/// Provenance proving complete topology and obstacle coverage for a negative verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteMetricEvidence(Evidence);

impl CompleteMetricEvidence {
    /// Promotes only exact, reviewable completeness evidence.
    pub fn try_new(evidence: Evidence) -> Result<Self, MetricRoutingError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(MetricRoutingError::IncompleteMetricEvidence);
        }
        Ok(Self(evidence))
    }

    /// Completeness provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.0
    }
}

/// A negative route verdict bound to the exact request and complete evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockedMetricRouteEvidence {
    request: MetricRouteRequest,
    completeness: CompleteMetricEvidence,
}

impl BlockedMetricRouteEvidence {
    /// Binds complete topology and obstacle evidence to one request.
    pub fn new(request: MetricRouteRequest, completeness: CompleteMetricEvidence) -> Self {
        Self {
            request,
            completeness,
        }
    }

    /// Request proven blocked.
    pub fn request(&self) -> &MetricRouteRequest {
        &self.request
    }

    /// Exact completeness provenance.
    pub fn completeness(&self) -> &CompleteMetricEvidence {
        &self.completeness
    }
}

/// A known route and conservative shortest-distance bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricRouteEvidence {
    shortest_distance: LengthInterval,
    waypoints: Vec<MetricPoint>,
    traversed_objects: Vec<ObjectId>,
    evidence: Evidence,
}

impl MetricRouteEvidence {
    /// Validates known-route evidence without upgrading bounded distance to exact.
    pub fn try_new(
        shortest_distance: LengthInterval,
        waypoints: Vec<MetricPoint>,
        traversed_objects: Vec<ObjectId>,
        evidence: Evidence,
    ) -> Result<Self, MetricRoutingError> {
        if waypoints.is_empty() || traversed_objects.is_empty() {
            return Err(MetricRoutingError::EmptyRouteEvidence);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(MetricRoutingError::InexactRouteEvidence);
        }
        Ok(Self {
            shortest_distance,
            waypoints,
            traversed_objects,
            evidence,
        })
    }

    /// Conservative shortest-distance bounds.
    pub fn shortest_distance(&self) -> &LengthInterval {
        &self.shortest_distance
    }

    /// Object-grounded route points in traversal order.
    pub fn waypoints(&self) -> &[MetricPoint] {
        &self.waypoints
    }

    /// Source-qualified objects traversed by the route.
    pub fn traversed_objects(&self) -> &[ObjectId] {
        &self.traversed_objects
    }

    /// Route computation provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Evaluated route result. Backend incompleteness is an error, not a third verdict.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricRouteOutcome {
    /// At least one route exists; the distance may remain conservatively bounded.
    Reachable(MetricRouteEvidence),
    /// No route exists under exact, complete topology and obstacle evidence.
    Blocked(BlockedMetricRouteEvidence),
}

/// Backend-neutral metric routing interface implemented by trusted host code.
pub trait MetricRoutingService: Send + Sync + 'static {
    /// Evaluates one route request or explicitly refuses unavailable evidence.
    fn route(&self, request: &MetricRouteRequest)
    -> Result<MetricRouteOutcome, MetricRoutingError>;
}

/// Concrete type-indexable wrapper around a metric routing service.
#[derive(Clone)]
pub struct MetricRoutingServiceHandle(Arc<dyn MetricRoutingService>);

impl MetricRoutingServiceHandle {
    /// Wraps an Axiolid or alternate backend implementation for service registration.
    pub fn new(service: Arc<dyn MetricRoutingService>) -> Self {
        Self(service)
    }

    /// Executes and validates endpoint identity in the backend response.
    pub fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        let outcome = self.0.route(request)?;
        if let MetricRouteOutcome::Reachable(route) = &outcome {
            let (Some(first), Some(last)) = (route.waypoints.first(), route.waypoints.last())
            else {
                return Err(MetricRoutingError::EmptyRouteEvidence);
            };
            if first != request.origin() || last != request.destination() {
                return Err(MetricRoutingError::ResponseEndpointMismatch);
            }
        } else if let MetricRouteOutcome::Blocked(blocked) = &outcome
            && blocked.request() != request
        {
            return Err(MetricRoutingError::ResponseEndpointMismatch);
        }
        Ok(outcome)
    }
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn reviewable_exact_evidence(evidence: &Evidence) -> bool {
    evidence.exact && !evidence.locator.trim().is_empty()
}
