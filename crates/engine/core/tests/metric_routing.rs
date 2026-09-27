//! Metric routing evidence and typed service contract tests.

use std::sync::Arc;

use axioval_engine::{
    BlockedMetricRouteEvidence, CompleteMetricEvidence, FarthestPointEvidence,
    FarthestPointOutcome, FarthestPointRequest, LengthInterval, MetricPoint, MetricRouteEvidence,
    MetricRouteOutcome, MetricRouteRequest, MetricRoutingError, MetricRoutingService,
    MetricRoutingServiceHandle, MobilityProfile, NearestTargetEvidence, NearestTargetOutcome,
    NearestTargetRequest, PathTrace, PathTraceRequest, ServiceRegistry, ThresholdVerdict,
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

/// Answers many-target queries with whatever `answer` says, to test the
/// handle's checks.
struct ManyTargets {
    target: usize,
    last: MetricPoint,
    converged: bool,
    witness: ObjectId,
}

impl MetricRoutingService for ManyTargets {
    fn route(
        &self,
        _request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        Err(MetricRoutingError::Unavailable(
            "pairs are not routed".into(),
        ))
    }

    fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        Ok(NearestTargetOutcome::Reached(
            NearestTargetEvidence::try_new(
                self.target,
                LengthInterval::try_new(2.0, 2.5)?,
                vec![request.origin().clone(), self.last.clone()],
                evidence("nearest"),
            )?,
        ))
    }

    fn farthest_point(
        &self,
        _request: &FarthestPointRequest,
    ) -> Result<FarthestPointOutcome, MetricRoutingError> {
        Ok(FarthestPointOutcome::Bounded(
            FarthestPointEvidence::try_new(
                LengthInterval::try_new(7.0, 7.5)?,
                MetricPoint::try_new(self.witness.clone(), [1.0, 1.0, 0.0])?,
                self.converged,
                evidence("farthest"),
            )?,
        ))
    }
}

fn many(target: usize, last: MetricPoint, converged: bool, witness: ObjectId) -> ManyTargets {
    ManyTargets {
        target,
        last,
        converged,
        witness,
    }
}

#[test]
fn many_target_requests_need_targets_and_a_valid_tolerance() {
    assert_eq!(
        NearestTargetRequest::try_new(point("cad", "a", 0.0), Vec::new(), profile()),
        Err(MetricRoutingError::NoTargets)
    );
    assert_eq!(
        FarthestPointRequest::try_new(object("cad", "room"), Vec::new(), profile(), 0.01),
        Err(MetricRoutingError::NoTargets)
    );
    for tolerance in [-0.1, f64::NAN, f64::INFINITY] {
        assert_eq!(
            FarthestPointRequest::try_new(
                object("cad", "room"),
                vec![point("cad", "exit", 1.0)],
                profile(),
                tolerance,
            ),
            Err(MetricRoutingError::InvalidTolerance)
        );
    }
}

#[test]
fn a_nearest_target_answer_must_reach_the_target_it_names() {
    let targets = vec![point("cad", "x", 3.0), point("cad", "y", 4.0)];
    let request =
        NearestTargetRequest::try_new(point("cad", "a", 0.0), targets.clone(), profile()).unwrap();
    let handle = |service: ManyTargets| MetricRoutingServiceHandle::new(Arc::new(service));
    let room = object("cad", "room");
    let answered = handle(many(1, targets[1].clone(), false, room.clone()))
        .nearest_target(&request)
        .unwrap();
    let NearestTargetOutcome::Reached(reached) = answered else {
        panic!("expected a reached target");
    };
    assert_eq!(reached.target(), 1);
    assert_eq!(
        handle(many(0, targets[1].clone(), false, room.clone())).nearest_target(&request),
        Err(MetricRoutingError::ResponseEndpointMismatch)
    );
    assert_eq!(
        handle(many(2, targets[1].clone(), false, room)).nearest_target(&request),
        Err(MetricRoutingError::InconsistentResponse)
    );
}

#[test]
fn a_farthest_point_answer_must_lie_on_the_region_and_converge_honestly() {
    let room = object("cad", "room");
    let exit = point("cad", "exit", 1.0);
    let handle = |service: ManyTargets| MetricRoutingServiceHandle::new(Arc::new(service));
    let request = |tolerance| {
        FarthestPointRequest::try_new(room.clone(), vec![exit.clone()], profile(), tolerance)
            .unwrap()
    };
    // The interval is 0.5 m wide: converged for a 0.5 m tolerance, not 0.1 m.
    assert!(
        handle(many(0, exit.clone(), true, room.clone()))
            .farthest_point(&request(0.5))
            .is_ok()
    );
    assert_eq!(
        handle(many(0, exit.clone(), true, room.clone())).farthest_point(&request(0.1)),
        Err(MetricRoutingError::InconsistentResponse)
    );
    assert!(
        handle(many(0, exit.clone(), false, room.clone()))
            .farthest_point(&request(0.1))
            .is_ok()
    );
    assert_eq!(
        handle(many(0, exit.clone(), false, object("cad", "elsewhere")))
            .farthest_point(&request(0.1)),
        Err(MetricRoutingError::ResponseEndpointMismatch)
    );
}

#[test]
fn a_backend_without_many_target_search_refuses_rather_than_answers() {
    let service = MetricRoutingServiceHandle::new(Arc::new(DeterministicRouter));
    let request = NearestTargetRequest::try_new(
        point("cad", "a", 0.0),
        vec![point("cad", "b", 3.0)],
        profile(),
    )
    .unwrap();
    assert!(matches!(
        service.nearest_target(&request),
        Err(MetricRoutingError::Unavailable(_))
    ));
    let request = FarthestPointRequest::try_new(
        object("cad", "room"),
        vec![point("cad", "b", 3.0)],
        profile(),
        0.01,
    )
    .unwrap();
    assert!(matches!(
        service.farthest_point(&request),
        Err(MetricRoutingError::Unavailable(_))
    ));
}

/// Answers every trace with `lengths`, and says whether it walks around
/// objects.
struct Tracer {
    lengths: Vec<Result<LengthInterval, String>>,
    avoids: bool,
}

impl MetricRoutingService for Tracer {
    fn route(
        &self,
        _request: &MetricRouteRequest,
    ) -> Result<MetricRouteOutcome, MetricRoutingError> {
        Err(MetricRoutingError::Unavailable(
            "pairs are not routed".into(),
        ))
    }

    fn nearest_target(
        &self,
        request: &NearestTargetRequest,
    ) -> Result<NearestTargetOutcome, MetricRoutingError> {
        Ok(NearestTargetOutcome::Reached(
            NearestTargetEvidence::try_new(
                0,
                LengthInterval::try_new(2.0, 2.5)?,
                vec![request.origin().clone(), request.targets()[0].clone()],
                evidence("nearest"),
            )?,
        ))
    }

    fn avoids_objects(&self) -> bool {
        self.avoids
    }

    fn trace_path(&self, _request: &PathTraceRequest) -> Result<PathTrace, MetricRoutingError> {
        PathTrace::try_new(self.lengths.clone(), evidence("trace"))
    }
}

#[test]
fn a_walk_around_objects_is_asked_only_of_a_backend_that_avoids_them() {
    let request = NearestTargetRequest::try_new(
        point("cad", "a", 0.0),
        vec![point("cad", "b", 3.0)],
        profile(),
    )
    .unwrap();
    let avoiding = request.clone().with_avoided(vec![
        object("cad", "z"),
        object("cad", "c"),
        object("cad", "z"),
    ]);
    assert_eq!(avoiding.avoided(), [object("cad", "c"), object("cad", "z")]);
    assert!(request.avoided().is_empty());
    let handle = |avoids| {
        MetricRoutingServiceHandle::new(Arc::new(Tracer {
            lengths: Vec::new(),
            avoids,
        }))
    };
    // The plain walk is asked of any backend; a detour only of one that
    // walks around objects, never answered as the plain walk.
    assert!(handle(false).nearest_target(&request).is_ok());
    assert!(matches!(
        handle(false).nearest_target(&avoiding),
        Err(MetricRoutingError::Unavailable(_))
    ));
    assert!(handle(true).nearest_target(&avoiding).is_ok());
}

#[test]
fn a_trace_answers_each_object_and_none_longer_than_the_path() {
    // 3 m, then 4 m: 7 m in plan, whatever the heights.
    let waypoints = vec![
        MetricPoint::try_new(object("cad", "a"), [0.0, 0.0, 0.0]).unwrap(),
        MetricPoint::try_new(object("cad", "room"), [3.0, 0.0, 0.0]).unwrap(),
        MetricPoint::try_new(object("cad", "b"), [3.0, 4.0, 1.0]).unwrap(),
    ];
    assert_eq!(
        PathTraceRequest::try_new(Vec::new(), vec![object("cad", "s")]),
        Err(MetricRoutingError::EmptyRouteEvidence)
    );
    let request = PathTraceRequest::try_new(
        waypoints,
        vec![object("cad", "t"), object("cad", "s"), object("cad", "t")],
    )
    .unwrap();
    assert_eq!(request.objects(), [object("cad", "s"), object("cad", "t")]);
    assert!((request.plan_length_metres() - 7.0).abs() < 1e-12);
    let handle = |lengths: Vec<Result<LengthInterval, String>>| {
        MetricRoutingServiceHandle::new(Arc::new(Tracer {
            lengths,
            avoids: false,
        }))
    };
    let within = LengthInterval::try_new(2.0, 3.0).unwrap();
    let trace = handle(vec![Ok(within), Err("unmeasured".into())])
        .trace_path(&request)
        .unwrap();
    assert_eq!(trace.lengths()[0], Ok(within));
    assert_eq!(
        handle(vec![Ok(within)]).trace_path(&request),
        Err(MetricRoutingError::InconsistentResponse)
    );
    // An upper bound past the path is only conservative; a lower bound past
    // it is impossible.
    assert!(
        handle(vec![
            Ok(LengthInterval::try_new(6.0, 8.0).unwrap()),
            Ok(within)
        ])
        .trace_path(&request)
        .is_ok()
    );
    assert_eq!(
        handle(vec![Ok(LengthInterval::exact(7.5).unwrap()), Ok(within)]).trace_path(&request),
        Err(MetricRoutingError::InconsistentResponse)
    );
    assert!(matches!(
        MetricRoutingServiceHandle::new(Arc::new(DeterministicRouter)).trace_path(&request),
        Err(MetricRoutingError::Unavailable(_))
    ));
    assert_eq!(
        PathTrace::try_new(
            Vec::new(),
            Evidence {
                source: source("geometry"),
                locator: "trace".into(),
                exact: false,
            }
        ),
        Err(MetricRoutingError::InexactRouteEvidence)
    );
}
