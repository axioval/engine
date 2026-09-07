//! Metric routing evidence and typed service contract tests.

use std::sync::Arc;

use axioval_engine::{
    BlockedMetricRouteEvidence, CompleteMetricEvidence, LengthInterval, MetricPoint,
    MetricRouteEvidence, MetricRouteOutcome, MetricRouteRequest, MetricRoutingError,
    MetricRoutingService, MetricRoutingServiceHandle, MobilityProfile, ServiceRegistry,
    ThresholdVerdict,
};
use axioval_ir::{Evidence, ObjectId, SourceId};

fn source(document: &str) -> SourceId {
    SourceId::new("test", document).unwrap()
}

fn object(document: &str, local_id: &str) -> ObjectId {
    ObjectId::new(source(document), local_id).unwrap()
}

fn point(document: &str, local_id: &str, x: f64) -> MetricPoint {
    MetricPoint::try_new(object(document, local_id), [x, 0.0, 0.0]).unwrap()
}

fn evidence(locator: &str) -> Evidence {
    Evidence::exact(source("geometry"), locator)
}

fn profile() -> MobilityProfile {
    MobilityProfile::try_new(0.4, 1.8, 0.05, 0.08).unwrap()
}

#[test]
fn source_qualified_metric_points_do_not_collapse() {
    let left = point("proprietary-cad", "same", 0.0);
    let right = point("ifc", "same", 1.0);
    assert_ne!(left.subject(), right.subject());
}

#[test]
fn non_finite_coordinates_fail_closed() {
    assert_eq!(
        MetricPoint::try_new(object("model", "bad"), [f64::NAN, 0.0, 0.0]),
        Err(MetricRoutingError::InvalidCoordinate)
    );
}

#[test]
fn invalid_distance_interval_is_rejected() {
    assert_eq!(
        LengthInterval::try_new(5.0, 4.0),
        Err(MetricRoutingError::InvalidLengthInterval)
    );
    assert_eq!(
        LengthInterval::try_new(-1.0, 4.0),
        Err(MetricRoutingError::InvalidLengthInterval)
    );
}

#[test]
fn threshold_comparison_is_three_valued() {
    let exact = LengthInterval::exact(5.0).unwrap();
    assert_eq!(
        exact.compare_maximum(5.0).unwrap(),
        ThresholdVerdict::Satisfied
    );
    assert_eq!(
        exact.compare_maximum(4.9).unwrap(),
        ThresholdVerdict::Violated
    );

    let bounded = LengthInterval::try_new(4.0, 6.0).unwrap();
    assert_eq!(
        bounded.compare_maximum(5.0).unwrap(),
        ThresholdVerdict::Indeterminate
    );
}

#[test]
fn incomplete_shortcut_cannot_be_reported_as_exact_shortest_distance() {
    let origin = point("model", "origin", 0.0);
    let destination = point("model", "destination", 10.0);
    let route = MetricRouteEvidence::try_new(
        LengthInterval::try_new(0.0, 10.0).unwrap(),
        vec![origin.clone(), destination.clone()],
        vec![origin.subject().clone(), destination.subject().clone()],
        evidence("known-long-route-with-unavailable-shortcut"),
    )
    .unwrap();

    assert_eq!(
        route.shortest_distance().compare_maximum(8.0).unwrap(),
        ThresholdVerdict::Indeterminate
    );
    assert!(!route.shortest_distance().is_exact());
}

#[test]
fn blocked_route_requires_exact_complete_metric_evidence() {
    let approximate = Evidence {
        source: source("geometry"),
        locator: "partial-obstacles".into(),
        exact: false,
    };
    assert_eq!(
        CompleteMetricEvidence::try_new(approximate),
        Err(MetricRoutingError::IncompleteMetricEvidence)
    );
}

#[derive(Clone)]
struct DeterministicRouter;

impl MetricRoutingService for DeterministicRouter {
    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        Ok(MetricRouteOutcome::Reachable(MetricRouteEvidence::try_new(
            LengthInterval::exact(3.0)?,
            vec![request.origin().clone(), request.destination().clone()],
            vec![
                request.origin().subject().clone(),
                request.destination().subject().clone(),
            ],
            evidence("mock-exact-route"),
        )?))
    }
}

#[test]
fn typed_service_handle_is_backend_neutral() {
    let request =
        MetricRouteRequest::new(point("cad", "a", 0.0), point("cad", "b", 3.0), profile());
    let service = MetricRoutingServiceHandle::new(Arc::new(DeterministicRouter));
    let MetricRouteOutcome::Reachable(route) = service.route(&request).unwrap() else {
        panic!("expected route")
    };
    assert_eq!(
        route.shortest_distance(),
        &LengthInterval::exact(3.0).unwrap()
    );
}

#[test]
fn typed_service_handle_registers_in_rule_context_registry() {
    let mut services = ServiceRegistry::new();
    services
        .register(MetricRoutingServiceHandle::new(Arc::new(
            DeterministicRouter,
        )))
        .unwrap();
    assert!(services.get::<MetricRoutingServiceHandle>().is_some());
}

#[derive(Clone)]
struct WrongEndpointRouter;

impl MetricRoutingService for WrongEndpointRouter {
    fn route(
        &self,
        request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        Ok(MetricRouteOutcome::Reachable(MetricRouteEvidence::try_new(
            LengthInterval::exact(1.0)?,
            vec![
                point("foreign", "wrong", 0.0),
                request.destination().clone(),
            ],
            vec![request.destination().subject().clone()],
            evidence("wrong-endpoint"),
        )?))
    }
}

#[test]
fn service_response_for_different_endpoints_is_rejected() {
    let request =
        MetricRouteRequest::new(point("cad", "a", 0.0), point("cad", "b", 1.0), profile());
    let service = MetricRoutingServiceHandle::new(Arc::new(WrongEndpointRouter));
    assert_eq!(
        service.route(&request),
        Err(MetricRoutingError::ResponseEndpointMismatch)
    );
}

#[derive(Clone)]
struct WrongBlockedRequestRouter;

impl MetricRoutingService for WrongBlockedRequestRouter {
    fn route(
        &self,
        _request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        let wrong_request = MetricRouteRequest::new(
            point("cad", "other-origin", 0.0),
            point("cad", "other-destination", 1.0),
            profile(),
        );
        Ok(MetricRouteOutcome::Blocked(
            BlockedMetricRouteEvidence::new(
                wrong_request,
                CompleteMetricEvidence::try_new(evidence("complete-wrong-query"))?,
            ),
        ))
    }
}

#[test]
fn blocked_evidence_is_bound_to_the_exact_request() {
    let request =
        MetricRouteRequest::new(point("cad", "a", 0.0), point("cad", "b", 1.0), profile());
    let service = MetricRoutingServiceHandle::new(Arc::new(WrongBlockedRequestRouter));
    assert_eq!(
        service.route(&request),
        Err(MetricRoutingError::ResponseEndpointMismatch)
    );
}
