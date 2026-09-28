//! Source-neutral metric-routing evidence and host-service contracts.
//!
//! Geometry algorithms do not live here. A trusted Axiolid or alternate backend
//! supplies this interface after adapting its native geometry into validated,
//! source-qualified evidence.

use std::sync::Arc;

use crate::services::reviewable_exact_evidence;
use crate::walkability::VerticalConnector;
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
    /// A many-target query named no target.
    #[error("a metric routing query needs at least one target")]
    NoTargets,
    /// A farthest-point tolerance was negative or non-finite.
    #[error("metric routing tolerance must be finite and non-negative")]
    InvalidTolerance,
    /// A backend answered with a target the request does not have, or
    /// claimed convergence for an interval wider than the tolerance.
    #[error("metric routing backend answered inconsistently with the request")]
    InconsistentResponse,
    /// A climb's vertical factor was negative or non-finite.
    #[error("a climb's vertical factor must be finite and non-negative")]
    InvalidClimb,
    /// One connector was given twice with different kinds.
    #[error("a vertical connector is given twice with different kinds")]
    ConflictingConnector,
    /// A travel cost factor was below one or non-finite.
    #[error("a travel cost factor must be finite and at least one")]
    InvalidCostFactor,
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

/// How a climb through a stair or ramp is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StairLength {
    /// Along the slope: `sqrt(h² + (f·v)²)` for a climb `h` long in plan
    /// and `v` high, with the vertical factor `f`.
    Slope,
    /// The horizontal length plus the rise times the vertical factor:
    /// `h + f·v`.
    HorizontalPlusVertical,
}

impl StairLength {
    /// The measure's name in rule parameters: `slope` or
    /// `horizontal-plus-vertical`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slope => "slope",
            Self::HorizontalPlusVertical => "horizontal-plus-vertical",
        }
    }
}

/// How much a climb through a vertical connector adds to a route's length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimbLength {
    measure: StairLength,
    vertical_factor: f64,
}

impl ClimbLength {
    /// Validates a finite, non-negative vertical factor.
    ///
    /// # Errors
    ///
    /// [`MetricRoutingError::InvalidClimb`] otherwise.
    pub fn try_new(measure: StairLength, vertical_factor: f64) -> Result<Self, MetricRoutingError> {
        if !valid_non_negative(vertical_factor) {
            return Err(MetricRoutingError::InvalidClimb);
        }
        Ok(Self {
            measure,
            vertical_factor,
        })
    }

    /// The true slope length: [`StairLength::Slope`] with a factor of one.
    #[must_use]
    pub fn slope() -> Self {
        Self {
            measure: StairLength::Slope,
            vertical_factor: 1.0,
        }
    }

    /// How the climb is measured.
    #[must_use]
    pub fn measure(&self) -> StairLength {
        self.measure
    }

    /// What a metre of rise counts.
    #[must_use]
    pub fn vertical_factor(&self) -> f64 {
        self.vertical_factor
    }

    /// Bounds the length of a climb whose plan length and rise lie in the
    /// given intervals. Both measures grow with either, so the bounds come
    /// from the ends, rounded outwards.
    #[must_use]
    pub fn length(&self, horizontal: LengthInterval, rise: LengthInterval) -> LengthInterval {
        let at = |h: f64, v: f64| match self.measure {
            StairLength::Slope => h.hypot(self.vertical_factor * v),
            StairLength::HorizontalPlusVertical => self.vertical_factor.mul_add(v, h),
        };
        let lower = at(horizontal.lower_metres(), rise.lower_metres());
        let upper = at(horizontal.upper_metres(), rise.upper_metres());
        LengthInterval {
            lower_metres: (lower * (1.0 - CLIMB_ROUNDING)).max(0.0),
            upper_metres: upper * (1.0 + CLIMB_ROUNDING),
        }
    }
}

/// Relative allowance for the rounding of a climb's length.
const CLIMB_ROUNDING: f64 = 4.0 * f64::EPSILON;

/// The vertical connectors a route may climb through, and how a climb
/// counts.
///
/// A route enters and leaves a connector at its **landings**, the two ends
/// of its walking line, and counts the climb between them by
/// [`ClimbLength`]. A request carrying this routes through these
/// connectors only: any other connector is no way between levels for it.
/// A backend that cannot prove a connector's length or passability leaves
/// every route through it undecided, never shorter and never blocked.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectorRouting {
    connectors: Vec<VerticalConnector>,
    climb: ClimbLength,
}

impl ConnectorRouting {
    /// The connectors (sorted, deduplicated) and the climb length.
    ///
    /// # Errors
    ///
    /// [`MetricRoutingError::ConflictingConnector`] when one object is
    /// given two kinds.
    pub fn try_new(
        mut connectors: Vec<VerticalConnector>,
        climb: ClimbLength,
    ) -> Result<Self, MetricRoutingError> {
        connectors.sort();
        connectors.dedup();
        if connectors
            .windows(2)
            .any(|pair| pair[0].object() == pair[1].object())
        {
            return Err(MetricRoutingError::ConflictingConnector);
        }
        Ok(Self { connectors, climb })
    }

    /// The connectors a route may climb through, sorted.
    #[must_use]
    pub fn connectors(&self) -> &[VerticalConnector] {
        &self.connectors
    }

    /// How a climb counts.
    #[must_use]
    pub fn climb(&self) -> ClimbLength {
        self.climb
    }
}

/// Travel over an object counted by a factor: a metre walked over its
/// plan footprint counts `factor` metres.
///
/// Where footprints overlap the greatest factor counts, and along a
/// footprint's edge the cheaper side does. A walk under a stair lies over
/// it, as for [`PathTraceRequest`].
///
/// A cost weighs only walks on its object's own level: the level it lies
/// in, or every level it spans. A walk on a floor above or below the
/// object is not weighed by it. Across levels ([`ConnectorRouting`]) a
/// climb counts its length times a factor between one and the largest
/// factor of a cost meeting its connector: the answer's lower bound takes
/// one, its upper bound the largest. A backend that cannot tell an
/// object's level refuses the request rather than weigh a level the
/// object is not on.
#[derive(Clone, Debug, PartialEq)]
pub struct TravelCost {
    object: ObjectId,
    factor: f64,
}

impl TravelCost {
    /// Validates a finite factor of at least one.
    ///
    /// # Errors
    ///
    /// [`MetricRoutingError::InvalidCostFactor`] otherwise.
    pub fn try_new(object: ObjectId, factor: f64) -> Result<Self, MetricRoutingError> {
        if !(factor.is_finite() && factor >= 1.0) {
            return Err(MetricRoutingError::InvalidCostFactor);
        }
        Ok(Self { object, factor })
    }

    /// The object whose plan footprint costs more.
    #[must_use]
    pub fn object(&self) -> &ObjectId {
        &self.object
    }

    /// What a metre over it counts.
    #[must_use]
    pub fn factor(&self) -> f64 {
        self.factor
    }
}

/// Sorts costs by object, keeps each object's greatest factor and drops a
/// factor of one, which changes nothing.
fn settled(mut costs: Vec<TravelCost>) -> Vec<TravelCost> {
    costs.retain(|cost| cost.factor > 1.0);
    costs.sort_by(|a, b| a.object.cmp(&b.object).then(b.factor.total_cmp(&a.factor)));
    costs.dedup_by(|later, earlier| later.object == earlier.object);
    costs
}

/// One source-neutral metric routing request.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricRouteRequest {
    origin: MetricPoint,
    destination: MetricPoint,
    profile: MobilityProfile,
    connectors: Option<ConnectorRouting>,
}

impl MetricRouteRequest {
    /// Creates a request from already validated values.
    pub fn new(origin: MetricPoint, destination: MetricPoint, profile: MobilityProfile) -> Self {
        Self {
            origin,
            destination,
            profile,
            connectors: None,
        }
    }

    /// The same request, climbing through `connectors` between levels
    /// (see [`ConnectorRouting`]). Only a backend that [climbs
    /// connectors](MetricRoutingService::climbs_connectors) is asked.
    #[must_use]
    pub fn with_connectors(mut self, connectors: ConnectorRouting) -> Self {
        self.connectors = Some(connectors);
        self
    }

    /// The connectors a route may climb through; `None` for a request that
    /// leaves them to the backend.
    pub fn connectors(&self) -> Option<&ConnectorRouting> {
        self.connectors.as_ref()
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

/// The distance from one point to the nearest of several targets.
///
/// With [`Self::with_avoided`], every walk keeps out of the named objects:
/// each is an obstacle wherever its body stands in the walking band, even a
/// surface or portal the backend would otherwise walk on or through. The
/// answer then bounds the shortest walk avoiding them all, so a lower bound
/// beyond the plain walk's upper bound proves that every shortest walk
/// enters one of them. Only a backend that [avoids
/// objects](MetricRoutingService::avoids_objects) is asked.
///
/// With [`Self::with_costs`], the distance is the least weighted cost of a
/// walk (see [`TravelCost`]), and the answer's route is a walk whose
/// weighted cost is at most its upper bound. Only a backend that [weighs
/// travel](MetricRoutingService::weighs_travel) is asked.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestTargetRequest {
    origin: MetricPoint,
    targets: Vec<MetricPoint>,
    profile: MobilityProfile,
    avoided: Vec<ObjectId>,
    connectors: Option<ConnectorRouting>,
    costs: Vec<TravelCost>,
}

impl NearestTargetRequest {
    /// Creates a request; targets keep their order, which answers index.
    pub fn try_new(
        origin: MetricPoint,
        targets: Vec<MetricPoint>,
        profile: MobilityProfile,
    ) -> Result<Self, MetricRoutingError> {
        if targets.is_empty() {
            return Err(MetricRoutingError::NoTargets);
        }
        Ok(Self {
            origin,
            targets,
            profile,
            avoided: Vec::new(),
            connectors: None,
            costs: Vec::new(),
        })
    }

    /// The same request, counting travel over objects by their factors
    /// (sorted by object, each object's greatest factor kept, factors of
    /// one dropped).
    #[must_use]
    pub fn with_costs(mut self, costs: Vec<TravelCost>) -> Self {
        self.costs = settled(costs);
        self
    }

    /// The costs travel counts by, sorted by object; empty for plain
    /// length.
    pub fn costs(&self) -> &[TravelCost] {
        &self.costs
    }

    /// The same request, walking around `avoided` (sorted, deduplicated).
    /// An avoided connector is not climbed.
    #[must_use]
    pub fn with_avoided(mut self, mut avoided: Vec<ObjectId>) -> Self {
        avoided.sort();
        avoided.dedup();
        self.avoided = avoided;
        self
    }

    /// The same request, climbing through `connectors` between levels
    /// (see [`ConnectorRouting`]).
    #[must_use]
    pub fn with_connectors(mut self, connectors: ConnectorRouting) -> Self {
        self.connectors = Some(connectors);
        self
    }

    /// The connectors a route may climb through; `None` for a request that
    /// leaves them to the backend.
    pub fn connectors(&self) -> Option<&ConnectorRouting> {
        self.connectors.as_ref()
    }

    /// Where every route starts.
    pub fn origin(&self) -> &MetricPoint {
        &self.origin
    }

    /// The targets, in request order.
    pub fn targets(&self) -> &[MetricPoint] {
        &self.targets
    }

    /// Mobility envelope.
    pub fn profile(&self) -> MobilityProfile {
        self.profile
    }

    /// The objects every walk keeps out of, sorted; empty for a plain walk.
    pub fn avoided(&self) -> &[ObjectId] {
        &self.avoided
    }
}

/// Bounds on the distance to the nearest target, and a route that realises
/// the upper bound.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestTargetEvidence {
    target: usize,
    shortest_distance: LengthInterval,
    waypoints: Vec<MetricPoint>,
    evidence: Evidence,
}

impl NearestTargetEvidence {
    /// Validates the answer: `target` indexes the request's targets and is
    /// the one the waypoints reach; `shortest_distance` bounds the distance
    /// to the nearest of all targets, which may be another one.
    pub fn try_new(
        target: usize,
        shortest_distance: LengthInterval,
        waypoints: Vec<MetricPoint>,
        evidence: Evidence,
    ) -> Result<Self, MetricRoutingError> {
        if waypoints.is_empty() {
            return Err(MetricRoutingError::EmptyRouteEvidence);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(MetricRoutingError::InexactRouteEvidence);
        }
        Ok(Self {
            target,
            shortest_distance,
            waypoints,
            evidence,
        })
    }

    /// Index of the target the route reaches.
    pub fn target(&self) -> usize {
        self.target
    }

    /// Conservative bounds on the distance to the nearest target.
    pub fn shortest_distance(&self) -> &LengthInterval {
        &self.shortest_distance
    }

    /// The route, from the origin to [`Self::target`].
    pub fn waypoints(&self) -> &[MetricPoint] {
        &self.waypoints
    }

    /// Measurement provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// No target is reachable from the origin, under complete exact evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct UnreachableTargetsEvidence {
    request: NearestTargetRequest,
    completeness: CompleteMetricEvidence,
}

impl UnreachableTargetsEvidence {
    /// Binds complete evidence to one request.
    pub fn new(request: NearestTargetRequest, completeness: CompleteMetricEvidence) -> Self {
        Self {
            request,
            completeness,
        }
    }

    /// Request proven unreachable.
    pub fn request(&self) -> &NearestTargetRequest {
        &self.request
    }

    /// Exact completeness provenance.
    pub fn completeness(&self) -> &CompleteMetricEvidence {
        &self.completeness
    }
}

/// Answer to a [`NearestTargetRequest`].
#[derive(Clone, Debug, PartialEq)]
pub enum NearestTargetOutcome {
    /// Some target is reachable; the nearest distance is bounded.
    Reached(NearestTargetEvidence),
    /// No target is reachable, with complete evidence.
    Unreachable(UnreachableTargetsEvidence),
}

/// The largest distance from any point of a region to the nearest of
/// several targets.
///
/// The region is an object's walkable area: the points of its plan, on its
/// floor, that the mobility profile leaves free. Only those points count;
/// a point inside an obstacle is none of the region's.
///
/// With [`Self::with_costs`], distances are weighted as for a
/// [`NearestTargetRequest`]; only a backend that [weighs
/// travel](MetricRoutingService::weighs_travel) is asked.
#[derive(Clone, Debug, PartialEq)]
pub struct FarthestPointRequest {
    region: ObjectId,
    targets: Vec<MetricPoint>,
    profile: MobilityProfile,
    tolerance_metres: f64,
    connectors: Option<ConnectorRouting>,
    costs: Vec<TravelCost>,
}

impl FarthestPointRequest {
    /// Creates a request. `tolerance_metres` is how narrow the interval
    /// should become; a backend may answer wider without claiming
    /// convergence, never narrower than the truth.
    pub fn try_new(
        region: ObjectId,
        targets: Vec<MetricPoint>,
        profile: MobilityProfile,
        tolerance_metres: f64,
    ) -> Result<Self, MetricRoutingError> {
        if targets.is_empty() {
            return Err(MetricRoutingError::NoTargets);
        }
        if !valid_non_negative(tolerance_metres) {
            return Err(MetricRoutingError::InvalidTolerance);
        }
        Ok(Self {
            region,
            targets,
            profile,
            tolerance_metres,
            connectors: None,
            costs: Vec::new(),
        })
    }

    /// The same request, counting travel over objects by their factors
    /// (sorted by object, each object's greatest factor kept, factors of
    /// one dropped).
    #[must_use]
    pub fn with_costs(mut self, costs: Vec<TravelCost>) -> Self {
        self.costs = settled(costs);
        self
    }

    /// The costs travel counts by, sorted by object; empty for plain
    /// length.
    pub fn costs(&self) -> &[TravelCost] {
        &self.costs
    }

    /// The same request, climbing through `connectors` between levels
    /// (see [`ConnectorRouting`]).
    #[must_use]
    pub fn with_connectors(mut self, connectors: ConnectorRouting) -> Self {
        self.connectors = Some(connectors);
        self
    }

    /// The connectors a route may climb through; `None` for a request that
    /// leaves them to the backend.
    pub fn connectors(&self) -> Option<&ConnectorRouting> {
        self.connectors.as_ref()
    }

    /// The object whose walkable area is measured.
    pub fn region(&self) -> &ObjectId {
        &self.region
    }

    /// The targets, in request order.
    pub fn targets(&self) -> &[MetricPoint] {
        &self.targets
    }

    /// Mobility envelope.
    pub fn profile(&self) -> MobilityProfile {
        self.profile
    }

    /// Requested interval width in metres.
    pub fn tolerance_metres(&self) -> f64 {
        self.tolerance_metres
    }
}

/// A certified bracket on the largest distance to the nearest target.
#[derive(Clone, Debug, PartialEq)]
pub struct FarthestPointEvidence {
    distance: LengthInterval,
    witness: MetricPoint,
    converged: bool,
    evidence: Evidence,
}

impl FarthestPointEvidence {
    /// Validates the bracket.
    ///
    /// `distance` contains the largest distance over the region; `witness`
    /// is a point of the region whose distance to every target is at least
    /// `distance`'s lower bound. `converged` claims the interval is no wider
    /// than the requested tolerance; the handle checks the claim.
    pub fn try_new(
        distance: LengthInterval,
        witness: MetricPoint,
        converged: bool,
        evidence: Evidence,
    ) -> Result<Self, MetricRoutingError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(MetricRoutingError::InexactRouteEvidence);
        }
        Ok(Self {
            distance,
            witness,
            converged,
            evidence,
        })
    }

    /// Bounds on the largest distance to the nearest target.
    pub fn distance(&self) -> &LengthInterval {
        &self.distance
    }

    /// A point of the region at least the lower bound from every target.
    pub fn witness(&self) -> &MetricPoint {
        &self.witness
    }

    /// Whether the interval is no wider than the requested tolerance.
    pub fn converged(&self) -> bool {
        self.converged
    }

    /// Measurement provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// Part of the region reaches no target, under complete exact evidence, so
/// the largest distance is unbounded.
#[derive(Clone, Debug, PartialEq)]
pub struct UnreachableRegionEvidence {
    request: FarthestPointRequest,
    witness: MetricPoint,
    completeness: CompleteMetricEvidence,
}

impl UnreachableRegionEvidence {
    /// Binds a point of the region no target reaches, and complete
    /// evidence, to one request.
    pub fn new(
        request: FarthestPointRequest,
        witness: MetricPoint,
        completeness: CompleteMetricEvidence,
    ) -> Self {
        Self {
            request,
            witness,
            completeness,
        }
    }

    /// Request proven to have an unreachable part.
    pub fn request(&self) -> &FarthestPointRequest {
        &self.request
    }

    /// A point of the region from which no target is reachable.
    pub fn witness(&self) -> &MetricPoint {
        &self.witness
    }

    /// Exact completeness provenance.
    pub fn completeness(&self) -> &CompleteMetricEvidence {
        &self.completeness
    }
}

/// Answer to a [`FarthestPointRequest`].
#[derive(Clone, Debug, PartialEq)]
pub enum FarthestPointOutcome {
    /// Every point of the region reaches a target; the largest distance is
    /// bracketed.
    Bounded(FarthestPointEvidence),
    /// Some point of the region reaches no target.
    Unreachable(UnreachableRegionEvidence),
}

/// How much of a walked polyline lies over each of several objects.
///
/// The polyline is a route's waypoints, such as a nearest-target answer's;
/// its length and every part of it are measured in plan, as routes are. A
/// part lies over an object where it lies inside the object's plan
/// footprint, whatever the heights: a walk under a stair lies over it.
#[derive(Clone, Debug, PartialEq)]
pub struct PathTraceRequest {
    waypoints: Vec<MetricPoint>,
    objects: Vec<ObjectId>,
}

impl PathTraceRequest {
    /// Creates a request; the objects are sorted and deduplicated, and the
    /// answer follows that order.
    pub fn try_new(
        waypoints: Vec<MetricPoint>,
        mut objects: Vec<ObjectId>,
    ) -> Result<Self, MetricRoutingError> {
        if waypoints.is_empty() {
            return Err(MetricRoutingError::EmptyRouteEvidence);
        }
        objects.sort();
        objects.dedup();
        Ok(Self { waypoints, objects })
    }

    /// The polyline, in walking order.
    pub fn waypoints(&self) -> &[MetricPoint] {
        &self.waypoints
    }

    /// The objects measured, sorted.
    pub fn objects(&self) -> &[ObjectId] {
        &self.objects
    }

    /// The polyline's length in plan, in metres.
    pub fn plan_length_metres(&self) -> f64 {
        self.waypoints
            .windows(2)
            .map(|pair| {
                let ([ax, ay, _], [bx, by, _]) =
                    (pair[0].coordinates_metres(), pair[1].coordinates_metres());
                (bx - ax).hypot(by - ay)
            })
            .sum()
    }
}

/// The length of a polyline over each requested object, in request order.
///
/// Each length is an interval: its upper bound counts every part that may
/// lie over the object, along the boundary of its footprint included; its
/// lower bound only the parts surely inside. An object whose footprint is
/// unknown answers why instead.
#[derive(Clone, Debug, PartialEq)]
pub struct PathTrace {
    lengths: Vec<Result<LengthInterval, String>>,
    evidence: Evidence,
}

impl PathTrace {
    /// Validates exact, reviewable provenance.
    pub fn try_new(
        lengths: Vec<Result<LengthInterval, String>>,
        evidence: Evidence,
    ) -> Result<Self, MetricRoutingError> {
        if !reviewable_exact_evidence(&evidence) {
            return Err(MetricRoutingError::InexactRouteEvidence);
        }
        Ok(Self { lengths, evidence })
    }

    /// The length over each object, in request order, or why it is unknown.
    pub fn lengths(&self) -> &[Result<LengthInterval, String>] {
        &self.lengths
    }

    /// Measurement provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// The shortest walk from a point to the nearest of several targets that
/// enters an object's plan footprint (touching it counts).
///
/// A lower bound beyond the upper bound of the plain walk's length proves
/// that no shortest walk enters the object. With [`Self::with_avoided`]
/// every walk keeps out of the named objects, as for a
/// [`NearestTargetRequest`].
#[derive(Clone, Debug, PartialEq)]
pub struct ForcedWalkRequest {
    origin: MetricPoint,
    targets: Vec<MetricPoint>,
    through: ObjectId,
    profile: MobilityProfile,
    tolerance_metres: f64,
    avoided: Vec<ObjectId>,
    connectors: Option<ConnectorRouting>,
}

impl ForcedWalkRequest {
    /// Creates a request; `tolerance_metres` is how narrow the bracket
    /// should become.
    ///
    /// # Errors
    ///
    /// [`MetricRoutingError::NoTargets`] for no target,
    /// [`MetricRoutingError::InvalidTolerance`] for a negative or
    /// non-finite tolerance.
    pub fn try_new(
        origin: MetricPoint,
        targets: Vec<MetricPoint>,
        through: ObjectId,
        profile: MobilityProfile,
        tolerance_metres: f64,
    ) -> Result<Self, MetricRoutingError> {
        if targets.is_empty() {
            return Err(MetricRoutingError::NoTargets);
        }
        if !valid_non_negative(tolerance_metres) {
            return Err(MetricRoutingError::InvalidTolerance);
        }
        Ok(Self {
            origin,
            targets,
            through,
            profile,
            tolerance_metres,
            avoided: Vec::new(),
            connectors: None,
        })
    }

    /// The same request, walking around `avoided` (sorted, deduplicated).
    #[must_use]
    pub fn with_avoided(mut self, mut avoided: Vec<ObjectId>) -> Self {
        avoided.sort();
        avoided.dedup();
        self.avoided = avoided;
        self
    }

    /// The same request, climbing through `connectors` between levels.
    #[must_use]
    pub fn with_connectors(mut self, connectors: ConnectorRouting) -> Self {
        self.connectors = Some(connectors);
        self
    }

    /// Where every walk starts.
    pub fn origin(&self) -> &MetricPoint {
        &self.origin
    }

    /// The targets, in request order.
    pub fn targets(&self) -> &[MetricPoint] {
        &self.targets
    }

    /// The object every walk measured enters.
    pub fn through(&self) -> &ObjectId {
        &self.through
    }

    /// Mobility envelope.
    pub fn profile(&self) -> MobilityProfile {
        self.profile
    }

    /// Requested bracket width in metres.
    pub fn tolerance_metres(&self) -> f64 {
        self.tolerance_metres
    }

    /// The objects every walk keeps out of, sorted.
    pub fn avoided(&self) -> &[ObjectId] {
        &self.avoided
    }

    /// The connectors a walk may climb through.
    pub fn connectors(&self) -> Option<&ConnectorRouting> {
        self.connectors.as_ref()
    }
}

/// Bounds on the shortest walk entering the object.
#[derive(Clone, Debug, PartialEq)]
pub struct ForcedWalkEvidence {
    lower_metres: f64,
    upper_metres: f64,
    converged: bool,
    evidence: Evidence,
}

impl ForcedWalkEvidence {
    /// Validates the bracket: `lower_metres` finite and non-negative,
    /// `upper_metres` no less, and infinite when no walk entering the
    /// object is known.
    ///
    /// # Errors
    ///
    /// [`MetricRoutingError::InvalidLengthInterval`] for bounds out of
    /// order, [`MetricRoutingError::InexactRouteEvidence`] for provenance
    /// that is not exact.
    pub fn try_new(
        lower_metres: f64,
        upper_metres: f64,
        converged: bool,
        evidence: Evidence,
    ) -> Result<Self, MetricRoutingError> {
        if !valid_non_negative(lower_metres) || upper_metres.is_nan() || upper_metres < lower_metres
        {
            return Err(MetricRoutingError::InvalidLengthInterval);
        }
        if !reviewable_exact_evidence(&evidence) {
            return Err(MetricRoutingError::InexactRouteEvidence);
        }
        Ok(Self {
            lower_metres,
            upper_metres,
            converged,
            evidence,
        })
    }

    /// No walk entering the object is shorter.
    pub fn lower_metres(&self) -> f64 {
        self.lower_metres
    }

    /// Some walk entering the object is no longer; infinite when none is
    /// known.
    pub fn upper_metres(&self) -> f64 {
        self.upper_metres
    }

    /// Whether the bracket is no wider than the requested tolerance.
    pub fn converged(&self) -> bool {
        self.converged
    }

    /// Measurement provenance.
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
}

/// No walk from the origin to a target enters the object, under complete
/// exact evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct NeverEnteredEvidence {
    request: ForcedWalkRequest,
    completeness: CompleteMetricEvidence,
}

impl NeverEnteredEvidence {
    /// Binds complete evidence to one request.
    pub fn new(request: ForcedWalkRequest, completeness: CompleteMetricEvidence) -> Self {
        Self {
            request,
            completeness,
        }
    }

    /// Request proven never entered.
    pub fn request(&self) -> &ForcedWalkRequest {
        &self.request
    }

    /// Exact completeness provenance.
    pub fn completeness(&self) -> &CompleteMetricEvidence {
        &self.completeness
    }
}

/// Answer to a [`ForcedWalkRequest`].
#[derive(Clone, Debug, PartialEq)]
pub enum ForcedWalkOutcome {
    /// The shortest walk entering the object is bracketed.
    Bounded(ForcedWalkEvidence),
    /// No walk to a target enters the object.
    NeverEntered(Box<NeverEnteredEvidence>),
}

/// Relative slack for a backend's rounding when a traced length is checked
/// against the polyline's own length.
const TRACE_ROUNDING: f64 = 1e-9;

/// Backend-neutral metric routing interface implemented by trusted host code.
pub trait MetricRoutingService: Send + Sync + 'static {
    /// Evaluates one route request or explicitly refuses unavailable evidence.
    fn route(&self, request: &MetricRouteRequest)
    -> Result<MetricRouteOutcome, MetricRoutingError>;

    /// Bounds the distance from the origin to the nearest target. The
    /// default refuses: a backend that cannot search many targets at once
    /// must not answer with a single pair.
    fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        let _ = request;
        Err(MetricRoutingError::Unavailable(
            "this backend does not measure nearest targets".into(),
        ))
    }

    /// Brackets the largest distance from any point of the region to the
    /// nearest target. The default refuses rather than sampling points.
    fn farthest_point(
        &self,
        request: &FarthestPointRequest,
    ) -> Result<FarthestPointOutcome, MetricRoutingError> {
        let _ = request;
        Err(MetricRoutingError::Unavailable(
            "this backend does not measure farthest points".into(),
        ))
    }

    /// Whether [`Self::nearest_target`] honours
    /// [`NearestTargetRequest::avoided`]. The default is `false`, and the
    /// handle then refuses a request avoiding anything rather than let the
    /// backend answer the plain walk.
    fn avoids_objects(&self) -> bool {
        false
    }

    /// Whether [`Self::nearest_target`] and [`Self::farthest_point`] honour
    /// a request's [`TravelCost`]s. The default is `false`, and the handle
    /// then refuses a weighted request rather than let the backend answer
    /// the plain length.
    fn weighs_travel(&self) -> bool {
        false
    }

    /// Brackets the shortest walk to a target that enters an object. The
    /// default refuses.
    fn forced_walk(
        &self,
        request: &ForcedWalkRequest,
    ) -> Result<ForcedWalkOutcome, MetricRoutingError> {
        let _ = request;
        Err(MetricRoutingError::Unavailable(
            "this backend does not measure walks forced through objects".into(),
        ))
    }

    /// Whether the queries honour a request's [`ConnectorRouting`]. The
    /// default is `false`, and the handle then refuses a request carrying
    /// connectors rather than let the backend answer a walk on one level.
    fn climbs_connectors(&self) -> bool {
        false
    }

    /// Measures how much of a polyline lies over each requested object. The
    /// default refuses.
    fn trace_path(&self, request: &PathTraceRequest) -> Result<PathTrace, MetricRoutingError> {
        let _ = request;
        Err(MetricRoutingError::Unavailable(
            "this backend does not trace paths over objects".into(),
        ))
    }
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
        self.climbing(request.connectors())?;
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

    /// Executes a nearest-target query and checks the answer is bound to it:
    /// the target exists, the route starts at the origin and ends at it, and
    /// an unreachable verdict names this request.
    ///
    /// A request avoiding objects is refused unless the backend [avoids
    /// objects](MetricRoutingService::avoids_objects).
    pub fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        if !request.avoided().is_empty() && !self.0.avoids_objects() {
            return Err(MetricRoutingError::Unavailable(
                "this backend does not walk around objects".into(),
            ));
        }
        self.weighing(request.costs())?;
        self.climbing(request.connectors())?;
        let outcome = self.0.nearest_target(request)?;
        match &outcome {
            NearestTargetOutcome::Reached(reached) => {
                let target = request
                    .targets()
                    .get(reached.target())
                    .ok_or(MetricRoutingError::InconsistentResponse)?;
                let (Some(first), Some(last)) =
                    (reached.waypoints().first(), reached.waypoints().last())
                else {
                    return Err(MetricRoutingError::EmptyRouteEvidence);
                };
                if first != request.origin() || last != target {
                    return Err(MetricRoutingError::ResponseEndpointMismatch);
                }
            }
            NearestTargetOutcome::Unreachable(unreachable) => {
                if unreachable.request() != request {
                    return Err(MetricRoutingError::ResponseEndpointMismatch);
                }
            }
        }
        Ok(outcome)
    }

    /// Executes a farthest-point query and checks the answer is bound to it:
    /// the witness lies on the requested region, a claimed convergence holds
    /// for the requested tolerance, and an unreachable verdict names this
    /// request.
    pub fn farthest_point(
        &self,
        request: &FarthestPointRequest,
    ) -> Result<FarthestPointOutcome, MetricRoutingError> {
        self.weighing(request.costs())?;
        self.climbing(request.connectors())?;
        let outcome = self.0.farthest_point(request)?;
        match &outcome {
            FarthestPointOutcome::Bounded(bounded) => {
                if bounded.witness().subject() != request.region() {
                    return Err(MetricRoutingError::ResponseEndpointMismatch);
                }
                let width = bounded.distance().upper_metres() - bounded.distance().lower_metres();
                if bounded.converged() && width > request.tolerance_metres() {
                    return Err(MetricRoutingError::InconsistentResponse);
                }
            }
            FarthestPointOutcome::Unreachable(unreachable) => {
                if unreachable.request() != request
                    || unreachable.witness().subject() != request.region()
                {
                    return Err(MetricRoutingError::ResponseEndpointMismatch);
                }
            }
        }
        Ok(outcome)
    }
}

impl MetricRoutingServiceHandle {
    /// Refuses a weighted request unless the backend [weighs
    /// travel](MetricRoutingService::weighs_travel).
    fn weighing(&self, costs: &[TravelCost]) -> Result<(), MetricRoutingError> {
        if !costs.is_empty() && !self.0.weighs_travel() {
            return Err(MetricRoutingError::Unavailable(
                "this backend does not weigh travel over objects".into(),
            ));
        }
        Ok(())
    }

    /// Executes a forced-walk query and checks the answer is bound to it: a
    /// claimed convergence holds for the requested tolerance, and a
    /// never-entered verdict names this request. A request avoiding
    /// objects or climbing connectors is refused as for
    /// [`Self::nearest_target`].
    pub fn forced_walk(
        &self,
        request: &ForcedWalkRequest,
    ) -> Result<ForcedWalkOutcome, MetricRoutingError> {
        if !request.avoided().is_empty() && !self.0.avoids_objects() {
            return Err(MetricRoutingError::Unavailable(
                "this backend does not walk around objects".into(),
            ));
        }
        self.climbing(request.connectors())?;
        let outcome = self.0.forced_walk(request)?;
        match &outcome {
            ForcedWalkOutcome::Bounded(bounded) => {
                if bounded.converged()
                    && bounded.upper_metres() - bounded.lower_metres() > request.tolerance_metres()
                {
                    return Err(MetricRoutingError::InconsistentResponse);
                }
            }
            ForcedWalkOutcome::NeverEntered(never) => {
                if never.request() != request {
                    return Err(MetricRoutingError::ResponseEndpointMismatch);
                }
            }
        }
        Ok(outcome)
    }

    /// Refuses a request carrying connectors unless the backend [climbs
    /// connectors](MetricRoutingService::climbs_connectors).
    fn climbing(&self, connectors: Option<&ConnectorRouting>) -> Result<(), MetricRoutingError> {
        if connectors.is_some() && !self.0.climbs_connectors() {
            return Err(MetricRoutingError::Unavailable(
                "this backend does not route through vertical connectors".into(),
            ));
        }
        Ok(())
    }

    /// Traces a polyline over objects and checks the answer is bound to it:
    /// one length per requested object, none surely longer than the
    /// polyline itself.
    pub fn trace_path(&self, request: &PathTraceRequest) -> Result<PathTrace, MetricRoutingError> {
        let trace = self.0.trace_path(request)?;
        if trace.lengths().len() != request.objects().len() {
            return Err(MetricRoutingError::InconsistentResponse);
        }
        let most = request.plan_length_metres() * (1.0 + TRACE_ROUNDING) + TRACE_ROUNDING;
        if trace
            .lengths()
            .iter()
            .flatten()
            .any(|length| length.lower_metres() > most)
        {
            return Err(MetricRoutingError::InconsistentResponse);
        }
        Ok(trace)
    }
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}
